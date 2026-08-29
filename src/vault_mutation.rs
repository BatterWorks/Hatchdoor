//! Shared-core, Vault-qualified mutations over authoritative Markdown.
//!
//! ADR-19 makes this the only seam a write adapter crosses. Everything the
//! HTTP and MCP adapters used to repeat around a `vault/write` primitive
//! lives here: resolving the Vault ID to a control block and refusing a
//! missing, disabled, or runtime-less Vault; the mutation capability check;
//! the per-Vault mutation lock; building the authoritative index off the
//! async runtime; resolving the slug to an entry; refusing a write to a path
//! this Vault's own exclusion patterns would make invisible; resolving the
//! archive prefix; running the blocking write off the async runtime; and
//! returning [`NoteWriteOutcome`] or a structured [`VaultOperationError`].
//! The adapters map that outcome or error onto their own wire shape and hold
//! nothing else.
//!
//! Since #186 every write primitive both surfaces expose runs through here:
//! the eleven note mutations, the four attachment mutations, and
//! write-capability discovery. The adapters kept only what is genuinely
//! theirs — parsing their own transport's arguments, and shaping this core's
//! typed outcome or structured error onto a status code or a JSON-RPC
//! failure. No index build, entry lookup, path refusal, or write-error
//! translation survives in either of them.

use std::sync::Arc;

use crate::app_state::AppState;
use crate::cache::SqliteCache;
use crate::runtime_config::ConfigSnapshot;
use crate::vault::{
    AttachmentOutcome, ExcludeMatcher, LayerMap, NoteEntry, SectionMode, VaultIndex, WriteError,
    WriteOutcome, append_note, archive_note, create_note, delete_attachment, delete_note,
    edit_note, import_attachment_bytes, move_attachment, move_or_rename_note, rename_attachment,
    replace_section, update_note, update_note_frontmatter,
};
use crate::vault_error::VaultOperationError;
use crate::vault_read::VaultReadCore;
use crate::vault_registry::VaultId;
use crate::vault_runtime::{VaultCollectionRuntime, VaultControlBlock};

/// One completed note mutation, with the resulting layer already resolved.
///
/// The layer comes from the `LayerMap` the write's own pre-write index build
/// already holds, never from a fresh post-write rescan: a rescan would delay
/// a mutation that has already committed to disk, and could turn a rescan
/// failure into an error for a write that succeeded (#101). A delete leaves
/// no note behind and always reports `None`; `trashed_path.is_some()` stands
/// in for "this is a delete", which is currently only ever true of
/// `delete_note`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteWriteOutcome {
    pub slug: Option<String>,
    pub relative_path: Option<String>,
    pub content_hash: Option<String>,
    pub quality_warnings: Vec<String>,
    pub rewritten_notes: usize,
    pub moved_assets: usize,
    pub trashed_path: Option<String>,
    pub layer: Option<String>,
}

impl NoteWriteOutcome {
    fn resolve(layers: &LayerMap, outcome: WriteOutcome) -> Self {
        let layer = if outcome.trashed_path.is_some() {
            None
        } else {
            outcome
                .relative_path
                .as_deref()
                .and_then(|relative_path| layers.layer_for(relative_path))
                .map(str::to_string)
        };
        Self {
            slug: outcome.slug,
            relative_path: outcome.relative_path,
            content_hash: outcome.content_hash,
            quality_warnings: outcome.quality_warnings,
            rewritten_notes: outcome.rewritten_notes,
            moved_assets: outcome.moved_assets,
            trashed_path: outcome.trashed_path,
            layer,
        }
    }
}

/// What one Vault currently permits a browser write surface to attempt: the
/// Vault's own source/lifecycle capability, and whether its Markdown directory
/// is writable on this filesystem. Both must hold for a write to be worth
/// offering. The operator-facing wording each surface builds from this — the
/// HTTP route's `warnings`, which also fold in the instance's web-auth posture
/// — stays with that surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WriteCapabilities {
    pub mutate_capable: bool,
    pub vault_writable: bool,
}

impl WriteCapabilities {
    pub fn enabled(&self) -> bool {
        self.mutate_capable && self.vault_writable
    }
}

/// The Vault-qualified mutation core. Cheap to construct per call, like
/// [`VaultReadCore`]: it borrows the shared cache and the Vault runtime and
/// holds one live settings snapshot for the instance-wide defaults a
/// mutation may need.
pub struct VaultMutationCore<'a> {
    cache: &'a SqliteCache,
    vaults: &'a VaultCollectionRuntime,
    settings: Arc<ConfigSnapshot>,
}

impl<'a> VaultMutationCore<'a> {
    pub fn new(
        cache: &'a SqliteCache,
        vaults: &'a VaultCollectionRuntime,
        settings: Arc<ConfigSnapshot>,
    ) -> Self {
        Self {
            cache,
            vaults,
            settings,
        }
    }

    /// The core as an adapter holding an [`AppState`] builds it.
    pub fn from_state(state: &'a AppState) -> Self {
        Self::new(
            &state.startup_sqlite,
            &state.vaults,
            state.runtime_snapshot(),
        )
    }

    /// Resolve and gate one Vault for mutation: the same not-found, disabled,
    /// and no-runtime check an exact read applies, then the Vault's own
    /// source/lifecycle capability — a pull-only managed Git Vault never
    /// allows mutation (#62).
    ///
    /// The returned target does *not* hold the mutation lock; the one-shot
    /// operations below take and release it around the write. A caller whose
    /// critical section is wider than one operation — the MCP `batch` tool,
    /// which holds one Vault's lock for a whole call — builds its target with
    /// [`VaultMutation::gated`] instead and takes the lock itself.
    fn open(&self, vault_id: VaultId) -> Result<VaultMutation, VaultOperationError> {
        let control = VaultReadCore::new(self.cache, self.vaults).control_block(vault_id)?;
        ensure_mutable(vault_id, &control)?;
        Ok(VaultMutation::gated(
            vault_id,
            control,
            Arc::clone(&self.settings),
        ))
    }

    /// Whether this Vault is worth offering a write surface for. Unlike every
    /// operation below this is a *discovery* call, so it deliberately does not
    /// go through [`VaultMutationCore::open`]: a Vault that refuses mutation
    /// must answer here rather than fail, which is the whole point of asking.
    pub fn write_capabilities(
        &self,
        vault_id: VaultId,
    ) -> Result<WriteCapabilities, VaultOperationError> {
        let reads = VaultReadCore::new(self.cache, self.vaults);
        let control = reads.control_block(vault_id)?;
        let vault_path = reads.vault_directory(vault_id)?;
        Ok(WriteCapabilities {
            mutate_capable: control.snapshot().capabilities.mutate,
            vault_writable: std::fs::metadata(&vault_path)
                .map(|metadata| !metadata.permissions().readonly())
                .unwrap_or(false),
        })
    }

    // -----------------------------------------------------------------
    // The one-shot form of each mutation: gate the Vault, take its mutation
    // lock, run the operation, release the lock. Every HTTP write route is
    // exactly one of these. A caller whose critical section is wider than a
    // single operation — the MCP `batch` tool — builds a [`VaultMutation`]
    // instead and holds the lock across several of them; the operations
    // themselves live there, on that type, and each method below is only the
    // gate-and-lock preamble.
    // -----------------------------------------------------------------

    /// Create one note at a Vault-relative path.
    pub async fn create_note(
        &self,
        vault_id: VaultId,
        relative_path: &str,
        content: &str,
        overwrite: bool,
    ) -> Result<NoteWriteOutcome, VaultOperationError> {
        let target = self.open(vault_id)?;
        let _guard = target.acquire_mutation().await?;
        target.create_note(relative_path, content, overwrite).await
    }

    /// Replace one note's whole content, under optimistic concurrency by
    /// expected content hash.
    pub async fn update_note(
        &self,
        vault_id: VaultId,
        slug: &str,
        content: &str,
        expected_content_hash: &str,
    ) -> Result<NoteWriteOutcome, VaultOperationError> {
        let target = self.open(vault_id)?;
        let _guard = target.acquire_mutation().await?;
        target
            .update_note(slug, content, expected_content_hash)
            .await
    }

    /// Append content to the end of one note.
    pub async fn append_to_note(
        &self,
        vault_id: VaultId,
        slug: &str,
        content: &str,
        expected_content_hash: &str,
    ) -> Result<NoteWriteOutcome, VaultOperationError> {
        let target = self.open(vault_id)?;
        let _guard = target.acquire_mutation().await?;
        target
            .append_to_note(slug, content, expected_content_hash)
            .await
    }

    /// Make one surgical string replacement in a note.
    pub async fn edit_note(
        &self,
        vault_id: VaultId,
        slug: &str,
        old_string: &str,
        new_string: &str,
        expected_content_hash: &str,
        replace_all: bool,
    ) -> Result<NoteWriteOutcome, VaultOperationError> {
        let target = self.open(vault_id)?;
        let _guard = target.acquire_mutation().await?;
        target
            .edit_note(
                slug,
                old_string,
                new_string,
                expected_content_hash,
                replace_all,
            )
            .await
    }

    /// Replace or insert around one whole Markdown section.
    pub async fn replace_section(
        &self,
        vault_id: VaultId,
        slug: &str,
        heading: &str,
        mode: SectionMode,
        content: &str,
        expected_content_hash: &str,
    ) -> Result<NoteWriteOutcome, VaultOperationError> {
        let target = self.open(vault_id)?;
        let _guard = target.acquire_mutation().await?;
        target
            .replace_section(slug, heading, mode, content, expected_content_hash)
            .await
    }

    /// Shallow-merge top-level keys into one note's frontmatter.
    pub async fn update_frontmatter(
        &self,
        vault_id: VaultId,
        slug: &str,
        frontmatter: serde_json::Map<String, serde_json::Value>,
        expected_content_hash: &str,
    ) -> Result<NoteWriteOutcome, VaultOperationError> {
        let target = self.open(vault_id)?;
        let _guard = target.acquire_mutation().await?;
        target
            .update_frontmatter(slug, frontmatter, expected_content_hash)
            .await
    }

    /// Rename one note within its own folder.
    pub async fn rename_note(
        &self,
        vault_id: VaultId,
        slug: &str,
        new_title: &str,
        expected_content_hash: &str,
    ) -> Result<NoteWriteOutcome, VaultOperationError> {
        let target = self.open(vault_id)?;
        let _guard = target.acquire_mutation().await?;
        target
            .rename_note(slug, new_title, expected_content_hash)
            .await
    }

    /// Move one note into another folder, keeping its filename.
    pub async fn move_note(
        &self,
        vault_id: VaultId,
        slug: &str,
        target_folder: &str,
        expected_content_hash: &str,
    ) -> Result<NoteWriteOutcome, VaultOperationError> {
        let target = self.open(vault_id)?;
        let _guard = target.acquire_mutation().await?;
        target
            .move_note(slug, target_folder, expected_content_hash)
            .await
    }

    /// Move and rename one note in a single operation.
    pub async fn move_rename_note(
        &self,
        vault_id: VaultId,
        slug: &str,
        target_relative_path: &str,
        expected_content_hash: &str,
    ) -> Result<NoteWriteOutcome, VaultOperationError> {
        let target = self.open(vault_id)?;
        let _guard = target.acquire_mutation().await?;
        target
            .move_rename_note(slug, target_relative_path, expected_content_hash)
            .await
    }

    /// Move one note into this Vault's archive folder, under optimistic
    /// concurrency by expected content hash.
    pub async fn archive_note(
        &self,
        vault_id: VaultId,
        slug: &str,
        expected_content_hash: &str,
    ) -> Result<NoteWriteOutcome, VaultOperationError> {
        let target = self.open(vault_id)?;
        let _guard = target.acquire_mutation().await?;
        target.archive_note(slug, expected_content_hash).await
    }

    /// Move one note into this Vault's recoverable trash (ADR-11).
    pub async fn delete_note(
        &self,
        vault_id: VaultId,
        slug: &str,
        expected_content_hash: &str,
    ) -> Result<NoteWriteOutcome, VaultOperationError> {
        let target = self.open(vault_id)?;
        let _guard = target.acquire_mutation().await?;
        target.delete_note(slug, expected_content_hash).await
    }

    /// Write one attachment's already-decoded bytes into this Vault. The
    /// caller's transport owns how those bytes arrived and enforces its own
    /// limit while reading them; `max_bytes` is the authoritative check on
    /// the decoded length.
    pub async fn import_attachment(
        &self,
        vault_id: VaultId,
        target_relative_path: &str,
        bytes: Vec<u8>,
        max_bytes: u64,
        overwrite: bool,
    ) -> Result<AttachmentOutcome, VaultOperationError> {
        let target = self.open(vault_id)?;
        let _guard = target.acquire_mutation().await?;
        target
            .import_attachment(target_relative_path, bytes, max_bytes, overwrite)
            .await
    }

    /// Move one attachment, rewriting every reference to it.
    pub async fn move_attachment(
        &self,
        vault_id: VaultId,
        source_relative_path: &str,
        target_relative_path: &str,
    ) -> Result<AttachmentOutcome, VaultOperationError> {
        let target = self.open(vault_id)?;
        let _guard = target.acquire_mutation().await?;
        target
            .move_attachment(source_relative_path, target_relative_path)
            .await
    }

    /// Rename one attachment in place, rewriting every reference to it.
    pub async fn rename_attachment(
        &self,
        vault_id: VaultId,
        source_relative_path: &str,
        new_filename: &str,
    ) -> Result<AttachmentOutcome, VaultOperationError> {
        let target = self.open(vault_id)?;
        let _guard = target.acquire_mutation().await?;
        target
            .rename_attachment(source_relative_path, new_filename)
            .await
    }

    /// Move one attachment into this Vault's recoverable trash.
    pub async fn delete_attachment(
        &self,
        vault_id: VaultId,
        source_relative_path: &str,
    ) -> Result<AttachmentOutcome, VaultOperationError> {
        let target = self.open(vault_id)?;
        let _guard = target.acquire_mutation().await?;
        target.delete_attachment(source_relative_path).await
    }
}

/// This Vault's current source/lifecycle capability: a pull-only managed Git
/// Vault never allows mutation (#62). Exposed because an adapter that already
/// holds a control block gates with it directly rather than making the core
/// look the Vault up a second time.
pub fn ensure_mutable(
    vault_id: VaultId,
    control: &VaultControlBlock,
) -> Result<(), VaultOperationError> {
    if control.snapshot().capabilities.mutate {
        Ok(())
    } else {
        Err(VaultOperationError::new(
            "capability_unavailable",
            "This Vault's current source and lifecycle do not allow mutation",
            Some(vault_id),
            false,
        ))
    }
}

/// Translates one `vault/write` failure into the structured error every
/// surface reports. Public because the MCP `list_note_attachments` read tool
/// calls a `vault/write` function without being a mutation, and must not
/// grow a second copy of this mapping to do it.
///
/// A partially-applied multi-phase mutation needs operator action, so its
/// message survives under its own code rather than collapsing into the
/// generic `write_failed` every other `Io` failure gets. What each surface
/// then shows the caller — a sanitized 500 over HTTP, the message over MCP —
/// is the adapter's mapping, not this core's business.
pub fn write_operation_error(vault_id: VaultId, error: WriteError) -> VaultOperationError {
    if let Some(message) = error.recovery_message() {
        return VaultOperationError::new(
            "write_recovery_required",
            message.to_string(),
            Some(vault_id),
            false,
        );
    }
    let (code, message, retryable) = match error {
        WriteError::Conflict(message) => ("write_conflict", message, true),
        WriteError::InvalidInput(message) => ("invalid_write_input", message, false),
        WriteError::Io(message) => ("write_failed", message, false),
    };
    VaultOperationError::new(code, message, Some(vault_id), retryable)
}

/// One gated, capability-checked Vault, ready to mutate.
pub struct VaultMutation {
    vault_id: VaultId,
    control: VaultControlBlock,
    settings: Arc<ConfigSnapshot>,
}

impl VaultMutation {
    /// For an adapter that resolved this Vault's control block itself and has
    /// already applied [`ensure_mutable`] to it. The MCP dispatcher does:
    /// `batch` gates and locks one Vault for a whole call spanning several
    /// operations, so it cannot go through the one-shot gate-lock-write on
    /// [`VaultMutationCore`]. Reusing the block it already holds also keeps
    /// every operation in that call on one Vault generation, where a fresh
    /// lookup could observe a reconciled replacement mid-batch.
    pub fn gated(
        vault_id: VaultId,
        control: VaultControlBlock,
        settings: Arc<ConfigSnapshot>,
    ) -> Self {
        Self {
            vault_id,
            control,
            settings,
        }
    }

    /// Take this Vault's mutation lock. The guard is owned, so a caller may
    /// hold it across as many operations as its own critical section covers.
    /// `tokio::sync::Mutex` is not reentrant: an operation called on this
    /// target never re-takes the lock, so the caller holding it is the only
    /// thing keeping writes serialized.
    pub async fn acquire_mutation(
        &self,
    ) -> Result<tokio::sync::OwnedMutexGuard<()>, VaultOperationError> {
        self.control
            .acquire_mutation()
            .await
            .map_err(|error| crate::vault_read::runtime_error(self.vault_id, error).into())
    }

    // -----------------------------------------------------------------
    // Note mutations. Each one takes the same shape: fetch whatever view of
    // the Vault this primitive needs, resolve the addressed note, refuse a
    // target path that would be invisible or reserved, then run the
    // `vault/write` primitive off the async runtime. The caller must already
    // hold this Vault's mutation lock.
    // -----------------------------------------------------------------

    /// Create one note at a Vault-relative path.
    pub async fn create_note(
        &self,
        relative_path: &str,
        content: &str,
        overwrite: bool,
    ) -> Result<NoteWriteOutcome, VaultOperationError> {
        self.reject_marker_write(relative_path)?;
        self.reject_noise_write(relative_path)?;
        // A metadata-only catalog, not the full index: `create_note` has no
        // pre-write entry to read a slug from, so it needs this to compute
        // one, but never touches the (expensive, content-reading) wikilink
        // graph.
        let catalog = self.authoritative_catalog().await?;
        let layers = catalog.layers.clone();
        let vault_path = self.control.vault_path().to_path_buf();
        let relative_path = relative_path.to_string();
        let content = content.to_string();
        let outcome = self
            .run_write(move || {
                create_note(&vault_path, &relative_path, &content, overwrite, &catalog)
            })
            .await?;
        Ok(NoteWriteOutcome::resolve(&layers, outcome))
    }

    /// Replace one note's whole content.
    pub async fn update_note(
        &self,
        slug: &str,
        content: &str,
        expected_content_hash: &str,
    ) -> Result<NoteWriteOutcome, VaultOperationError> {
        let index = self.authoritative_index().await?;
        let entry = self.note_entry(&index, slug)?;
        let content = content.to_string();
        let expected_content_hash = expected_content_hash.to_string();
        let outcome = self
            .run_write(move || update_note(&entry, &content, &expected_content_hash))
            .await?;
        Ok(NoteWriteOutcome::resolve(&index.layers, outcome))
    }

    /// Append content to the end of one note.
    pub async fn append_to_note(
        &self,
        slug: &str,
        content: &str,
        expected_content_hash: &str,
    ) -> Result<NoteWriteOutcome, VaultOperationError> {
        let index = self.authoritative_index().await?;
        let entry = self.note_entry(&index, slug)?;
        let content = content.to_string();
        let expected_content_hash = expected_content_hash.to_string();
        let outcome = self
            .run_write(move || append_note(&entry, &content, &expected_content_hash))
            .await?;
        Ok(NoteWriteOutcome::resolve(&index.layers, outcome))
    }

    /// Make one surgical string replacement in a note.
    pub async fn edit_note(
        &self,
        slug: &str,
        old_string: &str,
        new_string: &str,
        expected_content_hash: &str,
        replace_all: bool,
    ) -> Result<NoteWriteOutcome, VaultOperationError> {
        let index = self.authoritative_index().await?;
        let entry = self.note_entry(&index, slug)?;
        let old_string = old_string.to_string();
        let new_string = new_string.to_string();
        let expected_content_hash = expected_content_hash.to_string();
        let outcome = self
            .run_write(move || {
                edit_note(
                    &entry,
                    &old_string,
                    &new_string,
                    &expected_content_hash,
                    replace_all,
                )
            })
            .await?;
        Ok(NoteWriteOutcome::resolve(&index.layers, outcome))
    }

    /// Replace or insert around one whole Markdown section.
    pub async fn replace_section(
        &self,
        slug: &str,
        heading: &str,
        mode: SectionMode,
        content: &str,
        expected_content_hash: &str,
    ) -> Result<NoteWriteOutcome, VaultOperationError> {
        let index = self.authoritative_index().await?;
        let entry = self.note_entry(&index, slug)?;
        let heading = heading.to_string();
        let content = content.to_string();
        let expected_content_hash = expected_content_hash.to_string();
        let outcome = self
            .run_write(move || {
                replace_section(&entry, &heading, mode, &content, &expected_content_hash)
            })
            .await?;
        Ok(NoteWriteOutcome::resolve(&index.layers, outcome))
    }

    /// Shallow-merge top-level keys into one note's frontmatter.
    pub async fn update_frontmatter(
        &self,
        slug: &str,
        frontmatter: serde_json::Map<String, serde_json::Value>,
        expected_content_hash: &str,
    ) -> Result<NoteWriteOutcome, VaultOperationError> {
        let index = self.authoritative_index().await?;
        let entry = self.note_entry(&index, slug)?;
        let expected_content_hash = expected_content_hash.to_string();
        let outcome = self
            .run_write(move || update_note_frontmatter(&entry, frontmatter, &expected_content_hash))
            .await?;
        Ok(NoteWriteOutcome::resolve(&index.layers, outcome))
    }

    /// Rename one note within its own folder.
    pub async fn rename_note(
        &self,
        slug: &str,
        new_title: &str,
        expected_content_hash: &str,
    ) -> Result<NoteWriteOutcome, VaultOperationError> {
        let index = self.authoritative_index().await?;
        let entry = self.note_entry(&index, slug)?;
        let target_relative_path = renamed_note_path(&entry.relative_path, new_title);
        self.move_entry(index, entry, target_relative_path, expected_content_hash)
            .await
    }

    /// Move one note into another folder, keeping its filename. An empty
    /// target folder means the Vault root.
    pub async fn move_note(
        &self,
        slug: &str,
        target_folder: &str,
        expected_content_hash: &str,
    ) -> Result<NoteWriteOutcome, VaultOperationError> {
        let index = self.authoritative_index().await?;
        let entry = self.note_entry(&index, slug)?;
        let target_folder = target_folder.trim().trim_matches('/');
        let target_relative_path = if target_folder.is_empty() {
            file_name_of(&entry.relative_path).to_string()
        } else {
            format!("{target_folder}/{}", file_name_of(&entry.relative_path))
        };
        self.move_entry(index, entry, target_relative_path, expected_content_hash)
            .await
    }

    /// Move and rename one note in a single operation.
    pub async fn move_rename_note(
        &self,
        slug: &str,
        target_relative_path: &str,
        expected_content_hash: &str,
    ) -> Result<NoteWriteOutcome, VaultOperationError> {
        let index = self.authoritative_index().await?;
        let entry = self.note_entry(&index, slug)?;
        self.move_entry(
            index,
            entry,
            target_relative_path.to_string(),
            expected_content_hash,
        )
        .await
    }

    /// Move one note into this Vault's archive folder.
    pub async fn archive_note(
        &self,
        slug: &str,
        expected_content_hash: &str,
    ) -> Result<NoteWriteOutcome, VaultOperationError> {
        let index = self.authoritative_index().await?;
        let entry = self.note_entry(&index, slug)?;
        let archive_prefix = self.archive_prefix()?;
        let archive_folder = archive_prefix.trim().trim_matches('/');
        self.reject_noise_write(&format!(
            "{archive_folder}/{}",
            file_name_of(&entry.relative_path)
        ))?;

        let layers = index.layers.clone();
        let vault_path = self.control.vault_path().to_path_buf();
        let expected_content_hash = expected_content_hash.to_string();
        let outcome = self
            .run_write(move || {
                archive_note(
                    &vault_path,
                    &index,
                    &entry,
                    &archive_prefix,
                    &expected_content_hash,
                )
            })
            .await?;
        Ok(NoteWriteOutcome::resolve(&layers, outcome))
    }

    /// Move one note into this Vault's recoverable trash (ADR-11).
    pub async fn delete_note(
        &self,
        slug: &str,
        expected_content_hash: &str,
    ) -> Result<NoteWriteOutcome, VaultOperationError> {
        let index = self.authoritative_index().await?;
        let entry = self.note_entry(&index, slug)?;
        let layers = index.layers.clone();
        let vault_path = self.control.vault_path().to_path_buf();
        let expected_content_hash = expected_content_hash.to_string();
        let outcome = self
            .run_write(move || delete_note(&vault_path, &index, &entry, &expected_content_hash))
            .await?;
        Ok(NoteWriteOutcome::resolve(&layers, outcome))
    }

    /// The shared tail of every note move: refuse an invisible target, then
    /// run the one `vault/write` primitive rename, move, and move-rename all
    /// reduce to.
    async fn move_entry(
        &self,
        index: VaultIndex,
        entry: NoteEntry,
        target_relative_path: String,
        expected_content_hash: &str,
    ) -> Result<NoteWriteOutcome, VaultOperationError> {
        self.reject_noise_write(&target_relative_path)?;
        let layers = index.layers.clone();
        let vault_path = self.control.vault_path().to_path_buf();
        let expected_content_hash = expected_content_hash.to_string();
        let outcome = self
            .run_write(move || {
                move_or_rename_note(
                    &vault_path,
                    &index,
                    &entry,
                    &target_relative_path,
                    &expected_content_hash,
                )
            })
            .await?;
        Ok(NoteWriteOutcome::resolve(&layers, outcome))
    }

    // -----------------------------------------------------------------
    // Attachment mutations. An attachment carries no slug and no layer, so
    // these report `vault/write`'s own outcome unchanged.
    // -----------------------------------------------------------------

    /// Write one attachment's already-decoded bytes into this Vault.
    pub async fn import_attachment(
        &self,
        target_relative_path: &str,
        bytes: Vec<u8>,
        max_bytes: u64,
        overwrite: bool,
    ) -> Result<AttachmentOutcome, VaultOperationError> {
        self.reject_marker_write(target_relative_path)?;
        self.reject_noise_write(target_relative_path)?;
        let vault_path = self.control.vault_path().to_path_buf();
        let target_relative_path = target_relative_path.to_string();
        self.run_write(move || {
            import_attachment_bytes(
                &vault_path,
                &target_relative_path,
                &bytes,
                max_bytes,
                overwrite,
            )
        })
        .await
    }

    /// Move one attachment, rewriting every reference to it.
    pub async fn move_attachment(
        &self,
        source_relative_path: &str,
        target_relative_path: &str,
    ) -> Result<AttachmentOutcome, VaultOperationError> {
        self.reject_marker_write(source_relative_path)?;
        self.reject_marker_write(target_relative_path)?;
        self.reject_noise_write(target_relative_path)?;
        let index = self.authoritative_index().await?;
        let vault_path = self.control.vault_path().to_path_buf();
        let source_relative_path = source_relative_path.to_string();
        let target_relative_path = target_relative_path.to_string();
        self.run_write(move || {
            move_attachment(
                &vault_path,
                &index,
                &source_relative_path,
                &target_relative_path,
            )
        })
        .await
    }

    /// Rename one attachment in place, rewriting every reference to it.
    pub async fn rename_attachment(
        &self,
        source_relative_path: &str,
        new_filename: &str,
    ) -> Result<AttachmentOutcome, VaultOperationError> {
        self.reject_marker_write(source_relative_path)?;
        self.reject_marker_write(new_filename)?;
        self.reject_noise_write(&sibling_path(source_relative_path, new_filename))?;
        let index = self.authoritative_index().await?;
        let vault_path = self.control.vault_path().to_path_buf();
        let source_relative_path = source_relative_path.to_string();
        let new_filename = new_filename.to_string();
        self.run_write(move || {
            rename_attachment(&vault_path, &index, &source_relative_path, &new_filename)
        })
        .await
    }

    /// Move one attachment into this Vault's recoverable trash.
    pub async fn delete_attachment(
        &self,
        source_relative_path: &str,
    ) -> Result<AttachmentOutcome, VaultOperationError> {
        let index = self.authoritative_index().await?;
        let vault_path = self.control.vault_path().to_path_buf();
        let source_relative_path = source_relative_path.to_string();
        self.run_write(move || delete_attachment(&vault_path, &index, &source_relative_path))
            .await
    }

    // -----------------------------------------------------------------
    // Shared steps
    // -----------------------------------------------------------------

    /// This Vault's own configured archive folder overrides the instance-wide
    /// setting when present (#130).
    fn archive_prefix(&self) -> Result<Arc<str>, VaultOperationError> {
        AppState::vault_archive_prefix(Some(self.control.definition()), &self.settings)
            .map_err(|error| self.internal(error))
    }

    /// Builds this Vault's authoritative index off the async runtime: a
    /// synchronous full-Vault filesystem scan that must never run directly on
    /// a tokio worker.
    async fn authoritative_index(&self) -> Result<VaultIndex, VaultOperationError> {
        self.build_index("index", |control| control.authoritative_index())
            .await
    }

    /// Builds this Vault's metadata-only catalog off the async runtime, like
    /// [`VaultMutation::authoritative_index`] but skipping the content-reading
    /// wikilink-graph pass (`vault/links.rs`).
    async fn authoritative_catalog(&self) -> Result<VaultIndex, VaultOperationError> {
        self.build_index("catalog", |control| control.authoritative_catalog())
            .await
    }

    /// `kind` names the build in its panic message only — "index" or
    /// "catalog" — so the two callers stay distinguishable in a crash report.
    async fn build_index(
        &self,
        kind: &'static str,
        build: fn(
            &VaultControlBlock,
        ) -> Result<VaultIndex, crate::vault_runtime::VaultRuntimeError>,
    ) -> Result<VaultIndex, VaultOperationError> {
        let vault_id = self.vault_id;
        let control = self.control.clone();
        match tokio::task::spawn_blocking(move || build(&control)).await {
            Ok(Ok(index)) => Ok(index),
            Ok(Err(error)) => Err(crate::vault_read::runtime_error(vault_id, error).into()),
            Err(join_error) => Err(VaultOperationError::new(
                "vault_read_unavailable",
                format!("vault {kind} build panicked: {join_error}"),
                Some(vault_id),
                true,
            )),
        }
    }

    fn note_entry(&self, index: &VaultIndex, slug: &str) -> Result<NoteEntry, VaultOperationError> {
        let slug = slug.trim();
        // Explicit rather than leaning on `find_by_slug("")` happening to
        // miss: an empty slug names no Note, and that should not depend on a
        // lookup's behaviour for a degenerate key.
        if slug.is_empty() {
            return Err(self.note_not_found(slug));
        }
        index
            .find_by_slug(slug)
            .cloned()
            .ok_or_else(|| self.note_not_found(slug))
    }

    fn note_not_found(&self, slug: &str) -> VaultOperationError {
        VaultOperationError::new(
            "note_not_found",
            format!("Note not found: {slug}"),
            Some(self.vault_id),
            false,
        )
    }

    /// Refuse a write whose target path matches this Vault's own
    /// noise-exclusion patterns: the index applies the same matcher, so the
    /// file would land on disk yet be invisible to every read surface.
    fn reject_noise_write(&self, path: &str) -> Result<(), VaultOperationError> {
        let exclude = ExcludeMatcher::new(self.control.definition().exclude_patterns())
            .map_err(|error| self.internal(error))?;
        if exclude.is_excluded(std::path::Path::new(path.trim()), false) {
            return Err(VaultOperationError::new(
                "noise_excluded_write",
                format!(
                    "'{path}' matches this Vault's noise-exclusion pattern and would be ignored \
                     by the index; choose a path outside the excluded set."
                ),
                Some(self.vault_id),
                false,
            ));
        }
        Ok(())
    }

    /// Hard-refuse any write whose target basename is the layer marker file.
    /// A marker demotes its whole folder, so letting a write create or rename
    /// one would let a caller silently reclassify a subtree; markers are
    /// edited in the Vault directly, never through an API.
    fn reject_marker_write(&self, path: &str) -> Result<(), VaultOperationError> {
        // Take the last non-empty path segment so trailing separators or a
        // bare `.` component can't hide the marker basename, and compare
        // case-insensitively so a case-folding filesystem can't smuggle one
        // in either.
        let basename = path
            .split(['/', '\\'])
            .rfind(|segment| !segment.is_empty() && *segment != ".")
            .unwrap_or(path);
        if basename.eq_ignore_ascii_case(crate::vault::MARKER_FILE_NAME) {
            return Err(VaultOperationError::new(
                "layer_marker_write",
                format!(
                    "'{}' is a reserved Hatchdoor layer marker and cannot be written through the \
                     API; edit it directly in the vault.",
                    crate::vault::MARKER_FILE_NAME
                ),
                Some(self.vault_id),
                false,
            ));
        }
        Ok(())
    }

    /// Runs a synchronous `vault/write` primitive on the blocking pool.
    /// Moves rewrite every backlinking note, which must not stall a tokio
    /// worker; a panic maps to a `write_failed` error instead of unwinding
    /// through the adapter. Both surfaces offload, because the core does.
    async fn run_write<T: Send + 'static>(
        &self,
        op: impl FnOnce() -> Result<T, WriteError> + Send + 'static,
    ) -> Result<T, VaultOperationError> {
        let result = tokio::task::spawn_blocking(op)
            .await
            .unwrap_or_else(|join_error| {
                Err(WriteError::Io(format!("write task panicked: {join_error}")))
            });
        result.map_err(|error| write_operation_error(self.vault_id, error))
    }

    fn internal(&self, message: impl Into<String>) -> VaultOperationError {
        VaultOperationError::new("internal_error", message, Some(self.vault_id), false)
    }
}

/// The filename of a Vault-relative path, or the whole path when it names no
/// folder.
fn file_name_of(relative_path: &str) -> &str {
    relative_path.rsplit('/').next().unwrap_or(relative_path)
}

/// Where a note lands when it is renamed to `new_title` in place. A note is
/// addressed by title, not by filename, so this is the one caller that has to
/// supply the `.md` extension itself.
fn renamed_note_path(relative_path: &str, new_title: &str) -> String {
    sibling_path(relative_path, &format!("{new_title}.md"))
}

/// The path `file_name` would have in the same folder as `relative_path`.
fn sibling_path(relative_path: &str, file_name: &str) -> String {
    match relative_path.rsplit_once('/') {
        Some((directory, _)) if !directory.is_empty() => format!("{directory}/{file_name}"),
        _ => file_name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tempfile::TempDir;

    use super::{VaultMutationCore, VaultOperationError};
    use crate::cache::SqliteCache;
    use crate::runtime_config::{ConfigSnapshot, RuntimeConfig};
    use crate::vault::SectionMode;
    use crate::vault_registry::{
        NewVaultDefinition, VaultGitMode, VaultId, VaultRegistryStore, VaultSource,
    };
    use crate::vault_runtime::VaultCollectionRuntime;

    /// One Vault on a real filesystem, reconciled through the real registry
    /// and runtime, so a core test exercises the same gating, index build, and
    /// write primitives a request does — only without a transport.
    struct Workspace {
        _directory: TempDir,
        cache: SqliteCache,
        vaults: VaultCollectionRuntime,
        settings: std::sync::Arc<ConfigSnapshot>,
        vault_id: VaultId,
        vault_path: PathBuf,
    }

    /// A Vault to build, with only the fields these tests vary.
    struct Fixture {
        enabled: bool,
        exclude_patterns: Vec<String>,
        archive_folder: Option<String>,
        pull_only: bool,
        files: Vec<(String, String)>,
    }

    impl Fixture {
        fn new(files: &[(&str, &str)]) -> Self {
            Self {
                enabled: true,
                exclude_patterns: Vec::new(),
                archive_folder: None,
                pull_only: false,
                files: files
                    .iter()
                    .map(|(path, content)| ((*path).to_string(), (*content).to_string()))
                    .collect(),
            }
        }

        fn disabled(mut self) -> Self {
            self.enabled = false;
            self
        }

        fn excluding(mut self, patterns: &[&str]) -> Self {
            self.exclude_patterns = patterns.iter().map(|p| (*p).to_string()).collect();
            self
        }

        fn archiving_into(mut self, folder: &str) -> Self {
            self.archive_folder = Some(folder.to_string());
            self
        }

        /// A Pull-only Vault never allows mutation (#62). The registry only
        /// accepts an `existing_git` source pointing at a real working
        /// checkout, so this initialises one; no remote traffic is involved.
        fn pull_only(mut self) -> Self {
            self.pull_only = true;
            self
        }
    }

    fn workspace(fixture: Fixture) -> Workspace {
        let directory = tempfile::tempdir().expect("tempdir");
        let vault_path = directory.path().join("vault");
        std::fs::create_dir_all(&vault_path).expect("create vault directory");
        if fixture.pull_only {
            git2::Repository::init(&vault_path).expect("init git repo");
        }
        for (path, content) in &fixture.files {
            let path = vault_path.join(path);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
            std::fs::write(path, content).expect("write note");
        }

        let source = if fixture.pull_only {
            VaultSource::ExistingGit {
                repository_path: vault_path.clone(),
                repository_url: Some("https://example.test/vault.git".to_string()),
                branch: None,
                vault_subdirectory: None,
                mode: VaultGitMode::PullOnly,
                poll_interval_secs: 900,
            }
        } else {
            VaultSource::Local {
                path: vault_path.clone(),
            }
        };

        let store = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
        let snapshot = store
            .add(
                0,
                NewVaultDefinition {
                    name: "Fixture".to_string(),
                    enabled: fixture.enabled,
                    source,
                    exclude_patterns: fixture.exclude_patterns,
                    https_credentials: None,
                    archive_folder: fixture.archive_folder,
                    commit_identity: None,
                },
            )
            .expect("add Vault");
        let vault_id = snapshot
            .definitions()
            .next()
            .expect("definition")
            .vault_id();

        let vaults = VaultCollectionRuntime::new();
        vaults.reconcile(&store, &snapshot);

        Workspace {
            _directory: directory,
            cache: SqliteCache::in_memory(384).expect("cache"),
            vaults,
            settings: RuntimeConfig::for_tests().snapshot(),
            vault_id,
            vault_path,
        }
    }

    impl Workspace {
        fn core(&self) -> VaultMutationCore<'_> {
            VaultMutationCore::new(
                &self.cache,
                &self.vaults,
                std::sync::Arc::clone(&self.settings),
            )
        }

        fn read(&self, relative_path: &str) -> String {
            std::fs::read_to_string(self.vault_path.join(relative_path)).expect("read note")
        }

        fn exists(&self, relative_path: &str) -> bool {
            self.vault_path.join(relative_path).exists()
        }

        /// Runs `probe` with this Vault's directory made read-only, restoring
        /// the original permissions afterwards so the `TempDir` can still be
        /// cleaned up.
        fn while_read_only<T>(&self, probe: impl FnOnce() -> T) -> T {
            let original = std::fs::metadata(&self.vault_path)
                .expect("vault metadata")
                .permissions();
            let mut read_only = original.clone();
            read_only.set_readonly(true);
            std::fs::set_permissions(&self.vault_path, read_only).expect("make read-only");
            let outcome = probe();
            std::fs::set_permissions(&self.vault_path, original).expect("restore permissions");
            outcome
        }
    }

    fn hash(content: &str) -> String {
        crate::cache::parse::content_hash(content)
    }

    fn assert_code(error: &VaultOperationError, code: &str) {
        assert_eq!(error.code, code, "unexpected error: {error:?}");
    }

    #[tokio::test]
    async fn update_note_replaces_content_under_optimistic_concurrency() {
        let workspace = workspace(Fixture::new(&[("Home.md", "# Home\n")]));
        let outcome = workspace
            .core()
            .update_note(
                workspace.vault_id,
                "home",
                "# Home\n\nrewritten\n",
                &hash("# Home\n"),
            )
            .await
            .expect("update");

        assert_eq!(outcome.slug.as_deref(), Some("home"));
        assert_eq!(outcome.relative_path.as_deref(), Some("Home"));
        assert_eq!(outcome.layer, None);
        assert!(workspace.read("Home.md").contains("rewritten"));
    }

    #[tokio::test]
    async fn update_note_refuses_a_stale_expected_content_hash() {
        let workspace = workspace(Fixture::new(&[("Home.md", "# Home\n")]));
        let error = workspace
            .core()
            .update_note(
                workspace.vault_id,
                "home",
                "clobbered",
                "not-the-current-hash",
            )
            .await
            .expect_err("stale hash must be refused");

        assert_code(&error, "write_conflict");
        assert!(error.retryable);
        assert_eq!(workspace.read("Home.md"), "# Home\n");
    }

    #[tokio::test]
    async fn archive_note_refuses_a_stale_expected_content_hash() {
        let workspace = workspace(Fixture::new(&[("Home.md", "# Home\n")]));
        let error = workspace
            .core()
            .archive_note(workspace.vault_id, "home", "not-the-current-hash")
            .await
            .expect_err("stale hash must be refused");

        assert_code(&error, "write_conflict");
        assert!(workspace.exists("Home.md"));
        assert!(!workspace.exists("90-archive/Home.md"));
    }

    #[tokio::test]
    async fn archive_note_refuses_a_target_this_vaults_own_patterns_exclude() {
        // The index applies the same matcher, so the archived note would land
        // on disk yet be invisible to every read surface.
        let workspace =
            workspace(Fixture::new(&[("Home.md", "# Home\n")]).excluding(&["90-archive/"]));
        let error = workspace
            .core()
            .archive_note(workspace.vault_id, "home", &hash("# Home\n"))
            .await
            .expect_err("noise target must be refused");

        assert_code(&error, "noise_excluded_write");
        assert!(workspace.exists("Home.md"));
        assert!(!workspace.exists("90-archive/Home.md"));
    }

    #[tokio::test]
    async fn archive_note_prefers_this_vaults_own_archive_folder() {
        let workspace =
            workspace(Fixture::new(&[("Home.md", "# Home\n")]).archiving_into("Team Archive"));
        let outcome = workspace
            .core()
            .archive_note(workspace.vault_id, "home", &hash("# Home\n"))
            .await
            .expect("archive");

        assert_eq!(outcome.relative_path.as_deref(), Some("Team Archive/Home"));
        assert!(workspace.exists("Team Archive/Home.md"));
        assert!(!workspace.exists("90-archive/Home.md"));
    }

    #[tokio::test]
    async fn archive_note_falls_back_to_the_instance_default_archive_folder() {
        let workspace = workspace(Fixture::new(&[("Home.md", "# Home\n")]));
        let outcome = workspace
            .core()
            .archive_note(workspace.vault_id, "home", &hash("# Home\n"))
            .await
            .expect("archive");

        assert_eq!(outcome.relative_path.as_deref(), Some("90-archive/Home"));
        assert!(workspace.exists("90-archive/Home.md"));
    }

    #[tokio::test]
    async fn mutations_refuse_a_disabled_vault() {
        let workspace = workspace(Fixture::new(&[("Home.md", "# Home\n")]).disabled());
        let update = workspace
            .core()
            .update_note(workspace.vault_id, "home", "x", &hash("# Home\n"))
            .await
            .expect_err("disabled Vault");
        assert_code(&update, "vault_disabled");

        let archive = workspace
            .core()
            .archive_note(workspace.vault_id, "home", &hash("# Home\n"))
            .await
            .expect_err("disabled Vault");
        assert_code(&archive, "vault_disabled");
        assert_eq!(workspace.read("Home.md"), "# Home\n");
    }

    #[tokio::test]
    async fn mutations_refuse_a_pull_only_vault() {
        let workspace = workspace(Fixture::new(&[("Home.md", "# Home\n")]).pull_only());
        let update = workspace
            .core()
            .update_note(workspace.vault_id, "home", "x", &hash("# Home\n"))
            .await
            .expect_err("pull-only Vault");
        assert_code(&update, "capability_unavailable");

        let archive = workspace
            .core()
            .archive_note(workspace.vault_id, "home", &hash("# Home\n"))
            .await
            .expect_err("pull-only Vault");
        assert_code(&archive, "capability_unavailable");
        assert_eq!(workspace.read("Home.md"), "# Home\n");
    }

    // -----------------------------------------------------------------
    // Note creation
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn create_note_writes_the_file_and_reports_its_slug() {
        let workspace = workspace(Fixture::new(&[]));
        let outcome = workspace
            .core()
            .create_note(
                workspace.vault_id,
                "Projects/New Note.md",
                "# New Note\n",
                false,
            )
            .await
            .expect("create");

        assert_eq!(outcome.slug.as_deref(), Some("new-note"));
        assert_eq!(outcome.relative_path.as_deref(), Some("Projects/New Note"));
        assert_eq!(workspace.read("Projects/New Note.md"), "# New Note\n");
    }

    #[tokio::test]
    async fn create_note_refuses_an_existing_path_unless_overwrite_is_asked_for() {
        let workspace = workspace(Fixture::new(&[("Home.md", "# Home\n")]));
        let conflict = workspace
            .core()
            .create_note(workspace.vault_id, "Home.md", "# Clobbered\n", false)
            .await
            .expect_err("duplicate must be refused");
        assert_code(&conflict, "write_conflict");
        assert_eq!(workspace.read("Home.md"), "# Home\n");

        workspace
            .core()
            .create_note(workspace.vault_id, "Home.md", "# Overwritten\n", true)
            .await
            .expect("overwrite");
        assert_eq!(workspace.read("Home.md"), "# Overwritten\n");
    }

    #[tokio::test]
    async fn create_note_refuses_a_path_that_escapes_the_vault_root() {
        let workspace = workspace(Fixture::new(&[]));
        let error = workspace
            .core()
            .create_note(workspace.vault_id, "../escape.md", "# Nope\n", false)
            .await
            .expect_err("traversal must be refused");

        assert_code(&error, "invalid_write_input");
        assert!(!workspace.vault_path.join("../escape.md").exists());
    }

    #[tokio::test]
    async fn create_note_refuses_a_target_this_vaults_own_patterns_exclude() {
        let workspace = workspace(Fixture::new(&[]).excluding(&["*.tmp"]));
        let error = workspace
            .core()
            .create_note(
                workspace.vault_id,
                "Notes/scratch.tmp",
                "# Ignored\n",
                false,
            )
            .await
            .expect_err("noise target must be refused");

        assert_code(&error, "noise_excluded_write");
        assert!(!workspace.exists("Notes/scratch.tmp"));
    }

    #[tokio::test]
    async fn writes_refuse_the_reserved_layer_marker_basename() {
        // A marker demotes its whole folder, so a write that could create or
        // rename one would let a caller silently reclassify a subtree.
        let workspace = workspace(Fixture::new(&[]));
        let created = workspace
            .core()
            .create_note(workspace.vault_id, "wiki/.hatchdoor-layer", "sneaky", false)
            .await
            .expect_err("marker must be refused");
        assert_code(&created, "layer_marker_write");

        let imported = workspace
            .core()
            .import_attachment(
                workspace.vault_id,
                "wiki/.HATCHDOOR-LAYER",
                b"x".to_vec(),
                1024,
                false,
            )
            .await
            .expect_err("marker must be refused case-insensitively");
        assert_code(&imported, "layer_marker_write");
        assert!(!workspace.exists("wiki/.hatchdoor-layer"));
    }

    // -----------------------------------------------------------------
    // In-place note edits
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn append_edit_replace_section_and_frontmatter_each_rewrite_the_note_in_place() {
        let workspace = workspace(Fixture::new(&[("Home.md", "# Home\nalpha\n")]));
        let core = workspace.core();
        let vault_id = workspace.vault_id;

        let appended = core
            .append_to_note(vault_id, "home", "beta\n", &hash("# Home\nalpha\n"))
            .await
            .expect("append");
        assert_eq!(workspace.read("Home.md"), "# Home\nalpha\nbeta\n");

        let edited = core
            .edit_note(
                vault_id,
                "home",
                "alpha",
                "ALPHA",
                appended.content_hash.as_deref().expect("hash"),
                false,
            )
            .await
            .expect("edit");
        assert_eq!(workspace.read("Home.md"), "# Home\nALPHA\nbeta\n");

        let sectioned = core
            .replace_section(
                vault_id,
                "home",
                "# Home",
                SectionMode::Replace,
                "# Home\nrewritten\n",
                edited.content_hash.as_deref().expect("hash"),
            )
            .await
            .expect("replace section");
        assert_eq!(workspace.read("Home.md"), "# Home\nrewritten\n");

        let mut frontmatter = serde_json::Map::new();
        frontmatter.insert("status".to_string(), serde_json::json!("active"));
        core.update_frontmatter(
            vault_id,
            "home",
            frontmatter,
            sectioned.content_hash.as_deref().expect("hash"),
        )
        .await
        .expect("update frontmatter");
        assert_eq!(
            workspace.read("Home.md"),
            "---\nstatus: active\n---\n# Home\nrewritten\n"
        );
    }

    #[tokio::test]
    async fn every_in_place_edit_refuses_a_stale_expected_content_hash() {
        let workspace = workspace(Fixture::new(&[("Home.md", "# Home\n")]));
        let core = workspace.core();
        let vault_id = workspace.vault_id;
        let stale = "not-the-current-hash";

        assert_code(
            &core
                .append_to_note(vault_id, "home", "x", stale)
                .await
                .expect_err("append"),
            "write_conflict",
        );
        assert_code(
            &core
                .edit_note(vault_id, "home", "# Home", "x", stale, false)
                .await
                .expect_err("edit"),
            "write_conflict",
        );
        assert_code(
            &core
                .replace_section(vault_id, "home", "# Home", SectionMode::Replace, "x", stale)
                .await
                .expect_err("replace section"),
            "write_conflict",
        );
        assert_code(
            &core
                .update_frontmatter(
                    vault_id,
                    "home",
                    serde_json::Map::from_iter([("status".to_string(), serde_json::json!("x"))]),
                    stale,
                )
                .await
                .expect_err("update frontmatter"),
            "write_conflict",
        );
        assert_eq!(workspace.read("Home.md"), "# Home\n");
    }

    // -----------------------------------------------------------------
    // Note moves and deletion
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn rename_move_and_move_rename_relocate_one_note_in_turn() {
        let workspace = workspace(Fixture::new(&[("Home.md", "# Home\n")]));
        let core = workspace.core();
        let vault_id = workspace.vault_id;

        let renamed = core
            .rename_note(vault_id, "home", "Renamed Home", &hash("# Home\n"))
            .await
            .expect("rename");
        assert_eq!(renamed.slug.as_deref(), Some("renamed-home"));
        assert!(workspace.exists("Renamed Home.md"));
        assert!(!workspace.exists("Home.md"));

        let moved = core
            .move_note(
                vault_id,
                "renamed-home",
                "Projects",
                renamed.content_hash.as_deref().expect("hash"),
            )
            .await
            .expect("move");
        assert_eq!(
            moved.relative_path.as_deref(),
            Some("Projects/Renamed Home")
        );
        assert!(workspace.exists("Projects/Renamed Home.md"));

        let move_renamed = core
            .move_rename_note(
                vault_id,
                "renamed-home",
                "Archive/Final.md",
                moved.content_hash.as_deref().expect("hash"),
            )
            .await
            .expect("move-rename");
        assert_eq!(move_renamed.relative_path.as_deref(), Some("Archive/Final"));
        assert!(workspace.exists("Archive/Final.md"));
        assert!(!workspace.exists("Projects/Renamed Home.md"));
    }

    #[tokio::test]
    async fn move_note_into_an_empty_target_folder_lands_in_the_vault_root() {
        let workspace = workspace(Fixture::new(&[("Projects/Home.md", "# Home\n")]));
        let outcome = workspace
            .core()
            .move_note(workspace.vault_id, "home", "  ", &hash("# Home\n"))
            .await
            .expect("move to root");

        assert_eq!(outcome.relative_path.as_deref(), Some("Home"));
        assert!(workspace.exists("Home.md"));
    }

    #[tokio::test]
    async fn every_note_move_refuses_a_target_this_vaults_own_patterns_exclude() {
        let workspace = workspace(
            Fixture::new(&[("Home.md", "# Home\n")]).excluding(&[".trash/", "scratch.md"]),
        );
        let core = workspace.core();
        let vault_id = workspace.vault_id;
        let current = hash("# Home\n");

        assert_code(
            &core
                .move_note(vault_id, "home", ".trash", &current)
                .await
                .expect_err("move"),
            "noise_excluded_write",
        );
        assert_code(
            &core
                .move_rename_note(vault_id, "home", ".trash/Home.md", &current)
                .await
                .expect_err("move-rename"),
            "noise_excluded_write",
        );
        assert_code(
            &core
                .rename_note(vault_id, "home", "scratch", &current)
                .await
                .expect_err("rename"),
            "noise_excluded_write",
        );
        assert!(workspace.exists("Home.md"));
        assert!(!workspace.exists(".trash/Home.md"));
    }

    #[tokio::test]
    async fn delete_note_trashes_the_file_and_refuses_a_stale_hash() {
        let workspace = workspace(Fixture::new(&[("Home.md", "# Home\n")]));
        let stale = workspace
            .core()
            .delete_note(workspace.vault_id, "home", "not-the-current-hash")
            .await
            .expect_err("stale hash must be refused");
        assert_code(&stale, "write_conflict");
        assert!(workspace.exists("Home.md"));

        // ADR-11: a delete is recoverable, and leaves no note behind to carry
        // a layer.
        let outcome = workspace
            .core()
            .delete_note(workspace.vault_id, "home", &hash("# Home\n"))
            .await
            .expect("delete");
        assert!(outcome.trashed_path.is_some());
        assert_eq!(outcome.layer, None);
        assert!(!workspace.exists("Home.md"));
    }

    #[tokio::test]
    async fn every_note_mutation_refuses_a_slug_this_vault_does_not_hold() {
        // The index a write resolves against is built from this Vault's own
        // directory, so a slug that lives anywhere else is simply absent.
        let workspace = workspace(Fixture::new(&[("Home.md", "# Home\n")]));
        let core = workspace.core();
        let vault_id = workspace.vault_id;
        let irrelevant = hash("# Home\n");

        let mut refusals = vec![
            core.update_note(vault_id, "nowhere", "x", &irrelevant)
                .await
                .expect_err("update"),
            core.append_to_note(vault_id, "nowhere", "x", &irrelevant)
                .await
                .expect_err("append"),
            core.edit_note(vault_id, "nowhere", "a", "b", &irrelevant, false)
                .await
                .expect_err("edit"),
            core.replace_section(
                vault_id,
                "nowhere",
                "# H",
                SectionMode::Replace,
                "x",
                &irrelevant,
            )
            .await
            .expect_err("replace section"),
            core.update_frontmatter(vault_id, "nowhere", serde_json::Map::new(), &irrelevant)
                .await
                .expect_err("update frontmatter"),
            core.rename_note(vault_id, "nowhere", "New", &irrelevant)
                .await
                .expect_err("rename"),
            core.move_note(vault_id, "nowhere", "Projects", &irrelevant)
                .await
                .expect_err("move"),
            core.move_rename_note(vault_id, "nowhere", "Projects/New.md", &irrelevant)
                .await
                .expect_err("move-rename"),
            core.archive_note(vault_id, "nowhere", &irrelevant)
                .await
                .expect_err("archive"),
            core.delete_note(vault_id, "nowhere", &irrelevant)
                .await
                .expect_err("delete"),
        ];
        // An empty slug names no note either, and must not depend on how the
        // lookup happens to treat a degenerate key.
        refusals.push(
            core.update_note(vault_id, "   ", "x", &irrelevant)
                .await
                .expect_err("empty slug"),
        );

        for error in &refusals {
            assert_code(error, "note_not_found");
            assert_eq!(error.vault_id, Some(vault_id));
        }
    }

    // -----------------------------------------------------------------
    // Attachments
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn attachments_import_move_rename_and_delete_through_the_core() {
        let workspace = workspace(Fixture::new(&[]));
        let core = workspace.core();
        let vault_id = workspace.vault_id;

        let imported = core
            .import_attachment(
                vault_id,
                "Assets/diagram.png",
                b"png-bytes".to_vec(),
                1024,
                false,
            )
            .await
            .expect("import");
        assert_eq!(imported.attachment.relative_path, "Assets/diagram.png");
        assert_eq!(imported.attachment.size_bytes, 9);

        core.move_attachment(vault_id, "Assets/diagram.png", "Media/diagram.png")
            .await
            .expect("move");
        assert!(workspace.exists("Media/diagram.png"));

        let renamed = core
            .rename_attachment(vault_id, "Media/diagram.png", "chart.png")
            .await
            .expect("rename");
        assert_eq!(renamed.attachment.relative_path, "Media/chart.png");

        let deleted = core
            .delete_attachment(vault_id, "Media/chart.png")
            .await
            .expect("delete");
        assert!(deleted.trashed_path.is_some());
        assert!(!workspace.exists("Media/chart.png"));
    }

    #[tokio::test]
    async fn import_attachment_refuses_a_duplicate_and_an_over_limit_payload() {
        let workspace = workspace(Fixture::new(&[]));
        let core = workspace.core();
        let vault_id = workspace.vault_id;

        core.import_attachment(vault_id, "clip.png", b"first".to_vec(), 1024, false)
            .await
            .expect("import");

        let conflict = core
            .import_attachment(vault_id, "clip.png", b"second".to_vec(), 1024, false)
            .await
            .expect_err("duplicate must be refused");
        assert_code(&conflict, "write_conflict");

        // The decoded length is checked by the primitive, whatever transport
        // the bytes arrived on.
        let oversized = core
            .import_attachment(vault_id, "big.png", vec![b'x'; 32], 8, false)
            .await
            .expect_err("over-limit must be refused");
        assert_code(&oversized, "invalid_write_input");
        assert!(!workspace.exists("big.png"));
    }

    #[tokio::test]
    async fn attachment_writes_refuse_a_target_this_vaults_own_patterns_exclude() {
        let workspace = workspace(Fixture::new(&[]).excluding(&[".obsidian/"]));
        let error = workspace
            .core()
            .import_attachment(
                workspace.vault_id,
                ".obsidian/pasted.png",
                b"x".to_vec(),
                1024,
                false,
            )
            .await
            .expect_err("noise target must be refused");

        assert_code(&error, "noise_excluded_write");
        assert!(!workspace.exists(".obsidian/pasted.png"));
    }

    // -----------------------------------------------------------------
    // Write-capability discovery
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn write_capabilities_report_a_healthy_vault_as_writable() {
        let workspace = workspace(Fixture::new(&[("Home.md", "# Home\n")]));
        let capabilities = workspace
            .core()
            .write_capabilities(workspace.vault_id)
            .expect("capabilities");

        assert!(capabilities.mutate_capable);
        assert!(capabilities.vault_writable);
        assert!(capabilities.enabled());
    }

    #[tokio::test]
    async fn write_capabilities_report_a_read_only_directory_as_not_writable() {
        let workspace = workspace(Fixture::new(&[("Home.md", "# Home\n")]));
        let capabilities = workspace
            .while_read_only(|| workspace.core().write_capabilities(workspace.vault_id))
            .expect("capabilities");

        assert!(capabilities.mutate_capable);
        assert!(!capabilities.vault_writable);
        assert!(!capabilities.enabled());
    }

    #[tokio::test]
    async fn write_capabilities_report_a_pull_only_vault_as_not_mutable() {
        // Unlike every mutation, discovery must answer rather than fail for a
        // Vault that refuses writes — that is the whole point of asking.
        let workspace = workspace(Fixture::new(&[("Home.md", "# Home\n")]).pull_only());
        let capabilities = workspace
            .core()
            .write_capabilities(workspace.vault_id)
            .expect("capabilities");

        assert!(!capabilities.mutate_capable);
        assert!(!capabilities.enabled());
    }

    #[tokio::test]
    async fn mutations_refuse_an_unknown_vault() {
        let workspace = workspace(Fixture::new(&[("Home.md", "# Home\n")]));
        let error = workspace
            .core()
            .update_note(
                VaultId::generate().expect("generate Vault id"),
                "home",
                "x",
                &hash("# Home\n"),
            )
            .await
            .expect_err("unknown Vault");
        assert_code(&error, "vault_not_found");
    }
}
