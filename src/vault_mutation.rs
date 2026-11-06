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
use crate::git::{WriteOrigin, WriteRecord};
use crate::runtime_config::ConfigSnapshot;
use crate::vault::{
    AttachmentOutcome, ExcludeMatcher, LayerMap, NoteEntry, SectionMode, TagDelete, TagDeleteError,
    TagRename, TagRenameError, VaultIndex, WriteError, WriteOutcome, append_note, archive_note,
    check_attachment_import_target, check_note_content_hash, create_note, delete_attachment,
    delete_note, delete_tag, edit_note, import_attachment_bytes, move_attachment,
    move_or_rename_note, normalize_attachment_relative_path, normalize_note_relative_path,
    note_exists_conflict, note_target, rename_attachment, rename_tag, replace_section, update_note,
    update_note_frontmatter,
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
    /// Whether this Vault's filesystem can commit a conditional write as one
    /// atomic exchange. `Some(false)` means writes still work but through the
    /// weaker check-then-rename path, which an editor outside Hatchdoor can
    /// race (ADR-26).
    ///
    /// This answers for the filesystem, not for the Vault's permissions, so a
    /// read-only Vault on a capable filesystem still answers `Some(true)`.
    /// `None` is for a filesystem that could not be asked at all, a missing
    /// path being the case that occurs. What must never happen is a Vault
    /// blamed on its filesystem for being unwritable for its own reasons
    /// (#345).
    pub atomic_compare_and_swap: Option<bool>,
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
    /// Where every write this core makes came in from (ADR-49). Stated by
    /// the adapter that builds the core, which is the only party that knows.
    origin: WriteOrigin,
    /// The caller's summary of the change, given to every target
    /// [`VaultMutationCore::open`] opens (#249, #598).
    commit_summary: Option<String>,
}

impl<'a> VaultMutationCore<'a> {
    pub fn new(
        cache: &'a SqliteCache,
        vaults: &'a VaultCollectionRuntime,
        settings: Arc<ConfigSnapshot>,
        origin: WriteOrigin,
    ) -> Self {
        Self {
            cache,
            vaults,
            settings,
            origin,
            commit_summary: None,
        }
    }

    /// Carry `summary` into the commit that records each write this core
    /// makes, filtered as [`VaultMutation::with_commit_summary`] filters it.
    pub fn with_commit_summary(mut self, summary: Option<String>) -> Self {
        self.commit_summary = summary;
        self
    }

    /// The core as an adapter holding an [`AppState`] builds it.
    pub fn from_state(state: &'a AppState, origin: WriteOrigin) -> Self {
        Self::new(
            &state.startup_sqlite,
            &state.vaults,
            state.runtime_snapshot(),
            origin,
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
            self.origin.clone(),
        )
        .with_commit_summary(self.commit_summary.clone()))
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
            atomic_compare_and_swap: match crate::rename_flags::support(
                &vault_path,
                crate::rename_flags::RenameFlag::Exchange,
            ) {
                crate::rename_flags::FlagSupport::Supported => Some(true),
                crate::rename_flags::FlagSupport::Unsupported => Some(false),
                crate::rename_flags::FlagSupport::Undetermined => None,
            },
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
    /// expected content hash and, when given, the Vault-relative path the
    /// caller believes the slug names (#627).
    pub async fn update_note(
        &self,
        vault_id: VaultId,
        slug: &str,
        content: &str,
        expected_content_hash: &str,
        expected_relative_path: Option<&str>,
    ) -> Result<NoteWriteOutcome, VaultOperationError> {
        let target = self.open(vault_id)?;
        let _guard = target.acquire_mutation().await?;
        target
            .update_note(slug, content, expected_content_hash, expected_relative_path)
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

    /// Whether an import to `target_relative_path` would be refused before
    /// its bytes arrive: the Vault gate, the marker and noise refusals, the
    /// path and extension checks, and an existing file that may not be
    /// replaced. Writes nothing and takes no lock, so the answer can be stale
    /// by the time the import runs, which checks again.
    pub async fn check_attachment_import(
        &self,
        vault_id: VaultId,
        target_relative_path: &str,
        overwrite: bool,
    ) -> Result<(), VaultOperationError> {
        self.open(vault_id)?
            .check_attachment_import(target_relative_path, overwrite)
            .await
    }

    /// Whether a note upload to `target_relative_path` would be refused before
    /// its bytes arrive (ADR-32): the Vault gate, the marker and noise
    /// refusals, the path checks, and then, with no `expected_content_hash`,
    /// a note already there, or with one, a note that is missing or no longer
    /// has that hash. Writes nothing and takes no lock, so the upload checks
    /// again when it lands.
    pub async fn check_note_upload(
        &self,
        vault_id: VaultId,
        target_relative_path: &str,
        expected_content_hash: Option<&str>,
    ) -> Result<(), VaultOperationError> {
        self.open(vault_id)?
            .check_note_upload(target_relative_path, expected_content_hash)
            .await
    }

    /// Write an uploaded Markdown file as a note (ADR-32): the note write
    /// `create_note` performs with no `expected_content_hash`, and the one
    /// `update_note` performs with one. `max_bytes` is the authoritative check
    /// on the uploaded length, as for an attachment.
    pub async fn upload_note(
        &self,
        vault_id: VaultId,
        target_relative_path: &str,
        bytes: Vec<u8>,
        max_bytes: u64,
        expected_content_hash: Option<&str>,
    ) -> Result<NoteWriteOutcome, VaultOperationError> {
        let target = self.open(vault_id)?;
        let _guard = target.acquire_mutation().await?;
        target
            .upload_note(
                target_relative_path,
                bytes,
                max_bytes,
                expected_content_hash,
            )
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
    /// Plan, or plan and apply, one Vault-wide tag rename. The lock is held
    /// for the whole call, so an applied rename is one write in the ledger.
    pub async fn rename_tag(
        &self,
        vault_id: VaultId,
        old_tag: &str,
        new_tag: &str,
        expected_plan_hash: Option<&str>,
    ) -> Result<TagRename, VaultOperationError> {
        let target = self.open(vault_id)?;
        let _guard = target.acquire_mutation().await?;
        target
            .rename_tag(old_tag, new_tag, expected_plan_hash)
            .await
    }

    /// Plan, or plan and apply, one Vault-wide tag delete, under the same
    /// whole-call lock as a rename.
    pub async fn delete_tag(
        &self,
        vault_id: VaultId,
        tag: &str,
        expected_plan_hash: Option<&str>,
    ) -> Result<TagDelete, VaultOperationError> {
        let target = self.open(vault_id)?;
        let _guard = target.acquire_mutation().await?;
        target.delete_tag(tag, expected_plan_hash).await
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
/// A mutation that left the Vault in a state only a human can settle needs
/// operator action, so its message survives under its own code rather than
/// collapsing into the generic `write_failed` every other `Io` failure gets.
/// Two things raise it: a multi-phase mutation whose rollback was incomplete,
/// and a conditional write whose commit exchange could not be undone. What
/// each surface then shows the caller — a sanitized 500 over HTTP, the message
/// over MCP — is the adapter's mapping, not this core's business. Because the
/// message survives, its producer is the one that has to keep host paths out
/// of it.
pub fn write_operation_error(vault_id: VaultId, error: WriteError) -> VaultOperationError {
    if let Some(message) = error.recovery_message() {
        // The failure that most needs a person is also the one an operator is
        // least likely to have captured from their client (#345).
        tracing::error!(
            vault_id = %vault_id,
            "Vault write needs manual recovery: {message}"
        );
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
        WriteError::HeadingNotFound(message) => ("heading_not_found", message, false),
        WriteError::Io(message) => {
            // The only record of an I/O write failure used to be whatever the
            // client happened to log, which is how a whole filesystem being
            // unable to commit a write took an strace of a running container
            // to find (#345). A conflict is an expected answer and stays
            // quiet; this is not.
            tracing::error!(
                vault_id = %vault_id,
                "Vault write failed: {message}"
            );
            ("write_failed", message, false)
        }
        WriteError::LinkRewriteUnsupported(notes) => {
            let listed = notes
                .iter()
                .map(|note| format!("'{}' ({})", note.relative_path, note.reason))
                .collect::<Vec<_>>()
                .join("; ");
            let message = format!(
                "Nothing was written: {} note(s) link to what this operation moves or deletes, \
                 and their links cannot be rewritten in place. Fix them in the vault, then try again: {listed}",
                notes.len()
            );
            ("link_rewrite_unsupported", message, false)
        }
    };
    VaultOperationError::new(code, message, Some(vault_id), retryable)
}

/// What a `vault/write` outcome must expose for its write to be recorded in
/// the Vault's write ledger: which files it touched, and where the write
/// landed. Two implementors, one caller ([`VaultMutation::run_write`]) — it
/// exists so the ledger entry is built once for every mutation rather than
/// fifteen times, and so a new mutation cannot quietly skip it.
trait RecordedWrite {
    /// Absolute paths this operation created, modified, or removed.
    fn affected_paths(&self) -> &[std::path::PathBuf];
    /// The path to record this write against, when the outcome names a
    /// better one than the caller did — a slug is lowercased, and a create
    /// carries the `.md` the record should not. `None` falls back to what the
    /// caller addressed.
    fn written_path(&self) -> Option<&str>;
}

impl RecordedWrite for WriteOutcome {
    fn affected_paths(&self) -> &[std::path::PathBuf] {
        &self.affected_paths
    }

    fn written_path(&self) -> Option<&str> {
        // Always `Some` in practice, a delete included: `delete_note` reports
        // the path the note was deleted *from*, and reports where it went
        // separately as `trashed_path`.
        self.relative_path.as_deref()
    }
}

impl RecordedWrite for AttachmentOutcome {
    fn affected_paths(&self) -> &[std::path::PathBuf] {
        &self.affected_paths
    }

    fn written_path(&self) -> Option<&str> {
        // A trashed attachment reports its path *inside* the trash folder,
        // and trash is bookkeeping rather than a destination anyone chose
        // (ADR-11), so record the path the caller deleted instead — the same
        // path a deleted note reports. An archive, a move, and a rename all
        // keep reporting where the file went, because there the destination
        // is the point of the operation.
        if self.trashed_path.is_some() {
            return None;
        }
        Some(&self.attachment.relative_path)
    }
}

impl RecordedWrite for TagRename {
    fn affected_paths(&self) -> &[std::path::PathBuf] {
        &self.affected_paths
    }

    fn written_path(&self) -> Option<&str> {
        // A rename touches many notes and names none of them; the commit title
        // carries the tags instead, from what the caller addressed.
        None
    }
}

impl RecordedWrite for TagDelete {
    fn affected_paths(&self) -> &[std::path::PathBuf] {
        &self.affected_paths
    }

    fn written_path(&self) -> Option<&str> {
        // As with a rename, the commit title names the tag, not a note.
        None
    }
}

/// The structured error for a tag rename that did not run. A write failure
/// takes the same mapping as every other mutation; the three refusals only a
/// tag rename can give have codes of their own, so a caller can tell "fix
/// your arguments", "fix these notes" and "plan again" apart without reading
/// the message.
pub fn tag_rename_error(vault_id: VaultId, error: TagRenameError) -> VaultOperationError {
    match error {
        TagRenameError::InvalidTagName(message) => {
            VaultOperationError::new("invalid_tag_name", message, Some(vault_id), false)
        }
        TagRenameError::UnsupportedShape(notes) => {
            let listed = notes
                .iter()
                .map(|note| format!("'{}' ({})", note.relative_path, note.reason))
                .collect::<Vec<_>>()
                .join("; ");
            VaultOperationError::new(
                "tag_shape_unsupported",
                format!(
                    "Nothing was renamed: {} note(s) carry the tag in a form this rename cannot edit in place. \
                     Fix them in the vault, then plan again: {listed}",
                    notes.len()
                ),
                Some(vault_id),
                false,
            )
        }
        TagRenameError::StalePlan => VaultOperationError::new(
            "tag_rename_plan_stale",
            "Nothing was renamed: the Vault changed since this plan was made, so \
             expected_plan_hash no longer matches. Call rename_tag without it to see the current plan.",
            Some(vault_id),
            false,
        ),
        TagRenameError::Write(error) => write_operation_error(vault_id, error),
    }
}

/// The structured error for a tag delete that did not run. Each refusal has
/// its own code: a nested tag and an inline use are both "clear these first",
/// but what to clear differs, and a caller branches on the code.
pub fn tag_delete_error(vault_id: VaultId, error: TagDeleteError) -> VaultOperationError {
    match error {
        TagDeleteError::InvalidTagName(message) => {
            VaultOperationError::new("invalid_tag_name", message, Some(vault_id), false)
        }
        TagDeleteError::NestedTags(nested) => {
            let listed = nested
                .iter()
                .map(|nested| format!("'{}' ({} note(s))", nested.tag, nested.notes))
                .collect::<Vec<_>>()
                .join("; ");
            VaultOperationError::new(
                "tag_has_nested_tags",
                format!(
                    "Nothing was deleted: {} tag(s) are nested under this one, and a search for it \
                     would still find their notes. Delete or rename them first, deepest first: {listed}",
                    nested.len()
                ),
                Some(vault_id),
                false,
            )
        }
        TagDeleteError::InlineUse(notes) => {
            let listed = notes
                .iter()
                .map(|path| format!("'{path}'"))
                .collect::<Vec<_>>()
                .join("; ");
            VaultOperationError::new(
                "tag_used_inline",
                format!(
                    "Nothing was deleted: {} note(s) carry this tag inline in their body, which \
                     delete_tag never edits. Remove it from their text first, then plan again: {listed}",
                    notes.len()
                ),
                Some(vault_id),
                false,
            )
        }
        TagDeleteError::UnsupportedShape(notes) => {
            let listed = notes
                .iter()
                .map(|note| format!("'{}' ({})", note.relative_path, note.reason))
                .collect::<Vec<_>>()
                .join("; ");
            VaultOperationError::new(
                "tag_shape_unsupported",
                format!(
                    "Nothing was deleted: {} note(s) carry the tag in a form this delete cannot edit in place. \
                     Fix them in the vault, then plan again: {listed}",
                    notes.len()
                ),
                Some(vault_id),
                false,
            )
        }
        TagDeleteError::StalePlan => VaultOperationError::new(
            "tag_delete_plan_stale",
            "Nothing was deleted: the Vault changed since this plan was made, so \
             expected_plan_hash no longer matches. Call delete_tag without it to see the current plan.",
            Some(vault_id),
            false,
        ),
        TagDeleteError::Write(error) => write_operation_error(vault_id, error),
    }
}

/// One gated, capability-checked Vault, ready to mutate.
pub struct VaultMutation {
    vault_id: VaultId,
    control: VaultControlBlock,
    settings: Arc<ConfigSnapshot>,
    /// The caller's one-line description of the change, carried into the
    /// commit message of whichever Git turn ends up recording this write
    /// (#249). Set per operation, because that is the grain the MCP
    /// `commit_summary` argument arrives at: one summary describes one write,
    /// and a batch supplies a different one per item. `None` from a surface
    /// that has no such argument, which is every HTTP write route.
    commit_summary: Option<String>,
    /// Where this target's writes came in from, recorded with each one and
    /// carried into the trailers of the commit that records it (ADR-49).
    origin: WriteOrigin,
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
        origin: WriteOrigin,
    ) -> Self {
        Self {
            vault_id,
            control,
            settings,
            commit_summary: None,
            origin,
        }
    }

    /// Attach the caller's summary of this change. It reaches the body of the
    /// commit that records the write, one `- ` line per summary in the batch
    /// that commit coalesces (`git::build_commit_message`).
    pub fn with_commit_summary(mut self, summary: Option<String>) -> Self {
        self.commit_summary = summary.filter(|summary| !summary.trim().is_empty());
        self
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
        self.reject_noise_note_write(relative_path)?;
        // A metadata-only catalog, not the full index: `create_note` has no
        // pre-write entry to read a slug from, so it needs this to compute
        // one, but never touches the (expensive, content-reading) wikilink
        // graph.
        let catalog = self.authoritative_catalog().await?;
        let layers = catalog.layers.clone();
        let vault_path = self.control.vault_path().to_path_buf();
        let target_path = relative_path.to_string();
        let content = content.to_string();
        let outcome = self
            .run_write("create", relative_path, move || {
                create_note(&vault_path, &target_path, &content, overwrite, &catalog)
            })
            .await?;
        Ok(NoteWriteOutcome::resolve(&layers, outcome))
    }

    /// Replace one note's whole content.
    ///
    /// A slug moves when a same-named note appears earlier in path order
    /// (ADR-24), and two files with the same content share a hash, so the
    /// hash alone cannot tell the note the caller opened from the one that
    /// now holds its slug. `expected_relative_path` can: when the slug names
    /// a different file, the write is refused with `note_moved` before the
    /// hash is compared (#627).
    pub async fn update_note(
        &self,
        slug: &str,
        content: &str,
        expected_content_hash: &str,
        expected_relative_path: Option<&str>,
    ) -> Result<NoteWriteOutcome, VaultOperationError> {
        let index = self.authoritative_index().await?;
        let entry = self.note_entry(&index, slug)?;
        if let Some(expected) = expected_relative_path
            && expected != entry.relative_path
        {
            return Err(self.note_moved(slug, expected));
        }
        self.replace_entry(
            &index,
            entry,
            slug,
            content.to_string(),
            expected_content_hash,
        )
        .await
    }

    /// The whole-content replace `update_note` and a replacing note upload
    /// share, once the note is resolved.
    async fn replace_entry(
        &self,
        index: &VaultIndex,
        entry: NoteEntry,
        addressed: &str,
        content: String,
        expected_content_hash: &str,
    ) -> Result<NoteWriteOutcome, VaultOperationError> {
        let expected_content_hash = expected_content_hash.to_string();
        let outcome = self
            .run_write("update", addressed, move || {
                update_note(&entry, &content, &expected_content_hash)
            })
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
            .run_write("append", slug, move || {
                append_note(&entry, &content, &expected_content_hash)
            })
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
            .run_write("edit", slug, move || {
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
            .run_write("replace section in", slug, move || {
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
            .run_write("update frontmatter of", slug, move || {
                update_note_frontmatter(&entry, frontmatter, &expected_content_hash)
            })
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
        self.move_entry(
            "rename",
            slug,
            index,
            entry,
            target_relative_path,
            expected_content_hash,
        )
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
        self.move_entry(
            "move",
            slug,
            index,
            entry,
            target_relative_path,
            expected_content_hash,
        )
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
            "move",
            slug,
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
        self.reject_noise_note_write(&format!(
            "{archive_folder}/{}",
            file_name_of(&entry.relative_path)
        ))?;

        let layers = index.layers.clone();
        let vault_path = self.control.vault_path().to_path_buf();
        let expected_content_hash = expected_content_hash.to_string();
        let intent_log = self.control.intent_log();
        let outcome = self
            .run_write("archive", slug, move || {
                archive_note(
                    &vault_path,
                    &index,
                    &entry,
                    &archive_prefix,
                    &expected_content_hash,
                    Some(&intent_log),
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
        let intent_log = self.control.intent_log();
        let outcome = self
            .run_write("delete", slug, move || {
                delete_note(
                    &vault_path,
                    &index,
                    &entry,
                    &expected_content_hash,
                    Some(&intent_log),
                )
            })
            .await?;
        Ok(NoteWriteOutcome::resolve(&layers, outcome))
    }

    /// The shared tail of every note move: refuse an invisible target, then
    /// run the one `vault/write` primitive rename, move, and move-rename all
    /// reduce to.
    async fn move_entry(
        &self,
        label: &'static str,
        addressed: &str,
        index: VaultIndex,
        entry: NoteEntry,
        target_relative_path: String,
        expected_content_hash: &str,
    ) -> Result<NoteWriteOutcome, VaultOperationError> {
        self.reject_noise_note_write(&target_relative_path)?;
        let layers = index.layers.clone();
        let vault_path = self.control.vault_path().to_path_buf();
        let expected_content_hash = expected_content_hash.to_string();
        let intent_log = self.control.intent_log();
        let outcome = self
            .run_write(label, addressed, move || {
                move_or_rename_note(
                    &vault_path,
                    &index,
                    &entry,
                    &target_relative_path,
                    &expected_content_hash,
                    Some(&intent_log),
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
        let target_path = target_relative_path.to_string();
        self.run_write("import attachment", target_relative_path, move || {
            import_attachment_bytes(&vault_path, &target_path, &bytes, max_bytes, overwrite)
        })
        .await
    }

    /// [`VaultMutationCore::check_attachment_import`] on a Vault already
    /// gated. Answers the same refusals `import_attachment` would, with the
    /// same codes, and records nothing in the write ledger.
    pub async fn check_attachment_import(
        &self,
        target_relative_path: &str,
        overwrite: bool,
    ) -> Result<(), VaultOperationError> {
        self.reject_marker_write(target_relative_path)?;
        self.reject_noise_write(target_relative_path)?;
        let vault_path = self.control.vault_path().to_path_buf();
        let target_path = target_relative_path.to_string();
        offload(move || check_attachment_import_target(&vault_path, &target_path, overwrite))
            .await
            .map_err(|error| write_operation_error(self.vault_id, error))
    }

    /// [`VaultMutationCore::check_note_upload`] on a Vault already gated.
    pub async fn check_note_upload(
        &self,
        target_relative_path: &str,
        expected_content_hash: Option<&str>,
    ) -> Result<(), VaultOperationError> {
        let target_relative_path = target_relative_path.trim();
        self.reject_marker_write(target_relative_path)?;
        self.reject_noise_note_write(target_relative_path)?;
        let Some(expected_content_hash) = expected_content_hash else {
            let target = self.note_target(target_relative_path).await?;
            if target.exists {
                return Err(write_operation_error(
                    self.vault_id,
                    note_exists_conflict(&target),
                ));
            }
            return Ok(());
        };
        let catalog = self.authoritative_catalog().await?;
        let entry = self.note_at_path(&catalog, target_relative_path).await?;
        let expected_content_hash = expected_content_hash.to_string();
        offload(move || check_note_content_hash(&entry, &expected_content_hash))
            .await
            .map_err(|error| write_operation_error(self.vault_id, error))
    }

    /// [`VaultMutationCore::upload_note`] on a Vault already gated.
    pub async fn upload_note(
        &self,
        target_relative_path: &str,
        bytes: Vec<u8>,
        max_bytes: u64,
        expected_content_hash: Option<&str>,
    ) -> Result<NoteWriteOutcome, VaultOperationError> {
        let invalid = |message: String| {
            write_operation_error(self.vault_id, WriteError::InvalidInput(message))
        };
        let size = bytes.len() as u64;
        if size > max_bytes {
            return Err(invalid(format!(
                "uploaded note exceeds max size: {size} > {max_bytes}"
            )));
        }
        let content = String::from_utf8(bytes)
            .map_err(|_| invalid("an uploaded note must be valid UTF-8 text".to_string()))?;
        let target_relative_path = target_relative_path.trim();
        let Some(expected_content_hash) = expected_content_hash else {
            return self
                .create_note(target_relative_path, &content, false)
                .await;
        };
        self.reject_marker_write(target_relative_path)?;
        self.reject_noise_note_write(target_relative_path)?;
        let index = self.authoritative_index().await?;
        let entry = self.note_at_path(&index, target_relative_path).await?;
        self.replace_entry(
            &index,
            entry,
            target_relative_path,
            content,
            expected_content_hash,
        )
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
        self.reject_noise_write(source_relative_path)?;
        self.reject_noise_write(target_relative_path)?;
        let index = self.authoritative_index().await?;
        let vault_path = self.control.vault_path().to_path_buf();
        let source_path = source_relative_path.to_string();
        let target_path = target_relative_path.to_string();
        let intent_log = self.control.intent_log();
        self.run_write("move attachment", source_relative_path, move || {
            move_attachment(
                &vault_path,
                &index,
                &source_path,
                &target_path,
                Some(&intent_log),
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
        self.reject_noise_write(source_relative_path)?;
        let source_path = normalize_attachment_relative_path(source_relative_path)
            .map_err(|error| write_operation_error(self.vault_id, error))?;
        self.reject_excluded_path(&sibling_path(&source_path, new_filename.trim()))?;
        let index = self.authoritative_index().await?;
        let vault_path = self.control.vault_path().to_path_buf();
        let source_path = source_relative_path.to_string();
        let new_filename = new_filename.to_string();
        let intent_log = self.control.intent_log();
        self.run_write("rename attachment", source_relative_path, move || {
            rename_attachment(
                &vault_path,
                &index,
                &source_path,
                &new_filename,
                Some(&intent_log),
            )
        })
        .await
    }

    /// Move one attachment into this Vault's recoverable trash.
    pub async fn delete_attachment(
        &self,
        source_relative_path: &str,
    ) -> Result<AttachmentOutcome, VaultOperationError> {
        // The one attachment operation that was missing this guard. It did not
        // show while the write layer's upload allowlist refused the extensionless
        // marker basename underneath; dropping that allowlist (#247) makes the
        // refusal this path's own job, and without it a caller could trash a
        // folder's marker and silently promote the whole subtree.
        self.reject_marker_write(source_relative_path)?;
        self.reject_noise_write(source_relative_path)?;
        let index = self.authoritative_index().await?;
        let vault_path = self.control.vault_path().to_path_buf();
        let source_path = source_relative_path.to_string();
        let intent_log = self.control.intent_log();
        self.run_write("delete attachment", source_relative_path, move || {
            delete_attachment(&vault_path, &index, &source_path, Some(&intent_log))
        })
        .await
    }

    // -----------------------------------------------------------------
    // Vault-wide mutations
    // -----------------------------------------------------------------

    /// Rename a tag across the whole Vault. Without `expected_plan_hash` this
    /// only plans, writes nothing, and records nothing; with it, the plan is
    /// made again and applied only if its fingerprint still matches.
    pub async fn rename_tag(
        &self,
        old_tag: &str,
        new_tag: &str,
        expected_plan_hash: Option<&str>,
    ) -> Result<TagRename, VaultOperationError> {
        let catalog = self.authoritative_catalog().await?;
        let vault_path = self.control.vault_path().to_path_buf();
        let old_tag = old_tag.to_string();
        let new_tag = new_tag.to_string();
        let Some(expected_plan_hash) = expected_plan_hash.map(str::to_string) else {
            return offload(move || {
                rename_tag(&vault_path, &catalog, &old_tag, &new_tag, None, None)
            })
            .await
            .map_err(|error| tag_rename_error(self.vault_id, error));
        };
        let addressed = format!(
            "#{} -> #{}",
            old_tag.trim_start_matches('#'),
            new_tag.trim_start_matches('#')
        );
        let intent_log = self.control.intent_log();
        self.run_recorded("rename tag", &addressed, tag_rename_error, move || {
            rename_tag(
                &vault_path,
                &catalog,
                &old_tag,
                &new_tag,
                Some(&expected_plan_hash),
                Some(&intent_log),
            )
        })
        .await
    }

    /// Delete one exact tag from every frontmatter `tags` value in the Vault.
    /// Planning and applying work as they do for [`Self::rename_tag`].
    pub async fn delete_tag(
        &self,
        tag: &str,
        expected_plan_hash: Option<&str>,
    ) -> Result<TagDelete, VaultOperationError> {
        let catalog = self.authoritative_catalog().await?;
        let vault_path = self.control.vault_path().to_path_buf();
        let tag = tag.to_string();
        let Some(expected_plan_hash) = expected_plan_hash.map(str::to_string) else {
            return offload(move || delete_tag(&vault_path, &catalog, &tag, None, None))
                .await
                .map_err(|error| tag_delete_error(self.vault_id, error));
        };
        let addressed = format!("#{}", tag.strip_prefix('#').unwrap_or(&tag).to_lowercase());
        let intent_log = self.control.intent_log();
        self.run_recorded("delete tag", &addressed, tag_delete_error, move || {
            delete_tag(
                &vault_path,
                &catalog,
                &tag,
                Some(&expected_plan_hash),
                Some(&intent_log),
            )
        })
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

    /// Where a note write addressed by path would land, with `create_note`'s
    /// path refusals.
    async fn note_target(
        &self,
        relative_path: &str,
    ) -> Result<crate::vault::NoteTarget, VaultOperationError> {
        let vault_path = self.control.vault_path().to_path_buf();
        let relative_path = relative_path.to_string();
        offload(move || note_target(&vault_path, &relative_path))
            .await
            .map_err(|error| write_operation_error(self.vault_id, error))
    }

    /// The note an index holds at a Vault-relative path, for the writes that
    /// address a note by path rather than slug.
    async fn note_at_path(
        &self,
        index: &VaultIndex,
        relative_path: &str,
    ) -> Result<NoteEntry, VaultOperationError> {
        let target = self.note_target(relative_path).await?;
        index
            .ordered_entries()
            .into_iter()
            .find(|entry| entry.relative_path == target.relative_path)
            .ok_or_else(|| self.note_not_found(relative_path))
    }

    /// The slug now names a different file than the one the caller opened.
    /// Not retryable: the same request would land in the same other note.
    fn note_moved(&self, slug: &str, expected_relative_path: &str) -> VaultOperationError {
        VaultOperationError::new(
            "note_moved",
            format!(
                "Note {slug} no longer names {expected_relative_path}: another note took over its slug. Nothing was written."
            ),
            Some(self.vault_id),
            false,
        )
    }

    fn note_not_found(&self, slug: &str) -> VaultOperationError {
        VaultOperationError::new(
            "note_not_found",
            format!("Note not found: {slug}"),
            Some(self.vault_id),
            false,
        )
    }

    /// Check the note filename the write layer will use, including `.md`,
    /// rather than the caller's possibly extension-free spelling.
    fn reject_noise_note_write(&self, path: &str) -> Result<(), VaultOperationError> {
        let normalized = normalize_note_relative_path(path)
            .map_err(|error| write_operation_error(self.vault_id, error))?;
        self.reject_excluded_path(&normalized)
    }

    /// Refuse paths the index would hide, after the write layer's separator
    /// normalisation. Attachment operations check both source and destination;
    /// an excluded file such as `.obsidian/app.json` is not content to manage.
    fn reject_noise_write(&self, path: &str) -> Result<(), VaultOperationError> {
        let normalized = normalize_attachment_relative_path(path)
            .map_err(|error| write_operation_error(self.vault_id, error))?;
        self.reject_excluded_path(&normalized)
    }

    /// Match an already-normalised path without trimming it again: removing a
    /// leading `./` can expose whitespace that belongs to a real folder name.
    fn reject_excluded_path(&self, path: &str) -> Result<(), VaultOperationError> {
        let exclude = ExcludeMatcher::new(self.control.definition().exclude_patterns())
            .map_err(|error| self.internal(error))?;
        if exclude.is_excluded(std::path::Path::new(path), false) {
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

    /// Runs a synchronous `vault/write` primitive on the blocking pool, then
    /// records what it did in this Vault's write ledger.
    ///
    /// Moves rewrite every backlinking note, which must not stall a tokio
    /// worker; a panic maps to a `write_failed` error instead of unwinding
    /// through the adapter. Both surfaces offload, because the core does.
    ///
    /// The recording is here rather than in each operation above so that no
    /// mutation can be added without it: `label` names the operation for the
    /// commit title, `addressed` is what the caller named, and the outcome
    /// supplies the resulting path and the files actually touched. A write
    /// that failed records nothing, because nothing changed on disk. The same
    /// goes for marking the Vault's snapshot stale and asking for its Index
    /// and commit turns (#324).
    async fn run_write<T: RecordedWrite + Send + 'static>(
        &self,
        label: &'static str,
        addressed: &str,
        op: impl FnOnce() -> Result<T, WriteError> + Send + 'static,
    ) -> Result<T, VaultOperationError> {
        self.run_recorded(label, addressed, write_operation_error, op)
            .await
    }

    /// [`VaultMutation::run_write`] for a primitive with an error type of its
    /// own, which `translate` maps onto the structured error. A panic still
    /// reaches the caller as `write_failed`.
    async fn run_recorded<
        T: RecordedWrite + Send + 'static,
        E: From<WriteError> + Send + 'static,
    >(
        &self,
        label: &'static str,
        addressed: &str,
        translate: fn(VaultId, E) -> VaultOperationError,
        op: impl FnOnce() -> Result<T, E> + Send + 'static,
    ) -> Result<T, VaultOperationError> {
        // The published snapshot is marked stale on the blocking pool, right
        // after the write lands and still under the caller's mutation guard,
        // which is what keeps an Index turn already building from publishing
        // over it as fresh (#324).
        let control = self.control.clone();
        let outcome = offload(move || {
            let outcome = op()?;
            control.mark_snapshot_behind_write();
            Ok(outcome)
        })
        .await
        .map_err(|error| translate(self.vault_id, error))?;
        self.control.write_ledger().record(WriteRecord {
            op: label.to_string(),
            target: outcome.written_path().unwrap_or(addressed).to_string(),
            affected_paths: outcome.affected_paths().to_vec(),
            summary: self.commit_summary.clone(),
            origin: self.origin.clone(),
        });
        // After the record, so the commit this asks for can name the write.
        // The write path asks for its own Index and commit turns rather than
        // leaving that to a watcher, which may never have started or may have
        // lost the event (#324).
        self.control.report_write();
        Ok(outcome)
    }

    fn internal(&self, message: impl Into<String>) -> VaultOperationError {
        VaultOperationError::new("internal_error", message, Some(self.vault_id), false)
    }
}

/// Whether an upload to `target_relative_path` is a note upload rather than
/// an attachment one (ADR-32): its filename ends in `.md`, in any case, by the
/// rule the index and the write layer share (#637). A path with no extension
/// is not a note, though `create_note` would add one. The target is written as
/// given, so `Report.MD` replaces the `Report.MD` note the index lists.
pub fn is_note_upload_target(target_relative_path: &str) -> bool {
    crate::vault::is_note_path(std::path::Path::new(target_relative_path.trim()))
}

/// Run a blocking `vault/write` call on the blocking pool, turning a panic
/// into a `write_failed` rather than letting it unwind through the adapter.
async fn offload<T: Send + 'static, E: From<WriteError> + Send + 'static>(
    op: impl FnOnce() -> Result<T, E> + Send + 'static,
) -> Result<T, E> {
    tokio::task::spawn_blocking(op)
        .await
        .unwrap_or_else(|join_error| {
            Err(WriteError::Io(format!("write task panicked: {join_error}")).into())
        })
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

    use super::{VaultMutationCore, VaultOperationError, is_note_upload_target};
    use crate::cache::SqliteCache;
    use crate::runtime_config::{ConfigSnapshot, RuntimeConfig};
    use crate::vault::SectionMode;
    use crate::vault_registry::{
        NewVaultDefinition, VaultGitMode, VaultId, VaultRegistryStore, VaultSource,
    };
    use crate::vault_runtime::VaultCollectionRuntime;

    #[test]
    fn a_note_upload_target_ends_in_md_in_any_case_and_nothing_else_is_one() {
        for note in ["Report.md", "Imports/Report.md", " Notes/x.MD ", "a.b.Md"] {
            assert!(is_note_upload_target(note), "{note}");
        }
        for other in ["Report", "Imports/scan.png", ".md", "a.mdx", "md"] {
            assert!(!is_note_upload_target(other), "{other}");
        }
    }

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
                crate::git::WriteOrigin::web(),
            )
        }

        /// This Vault's pending write records, taken from the same ledger the
        /// Vault's Git turn takes them from.
        fn pending_writes(&self) -> Vec<crate::git::WriteRecord> {
            self.vaults
                .runtime(self.vault_id)
                .expect("Vault runtime")
                .write_ledger()
                .take()
        }

        /// A gated mutation target carrying `summary`, the way the MCP write
        /// tools build one from their `commit_summary` argument.
        fn summarized(&self, summary: &str) -> super::VaultMutation {
            let control = self.vaults.runtime(self.vault_id).expect("Vault runtime");
            super::VaultMutation::gated(
                self.vault_id,
                control,
                std::sync::Arc::clone(&self.settings),
                crate::git::WriteOrigin::mcp(Some("claude-code")),
            )
            .with_commit_summary(Some(summary.to_string()))
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

    /// Issue #249: `commit_summary` is what an agent spends tokens composing on
    /// every write, and it has to survive as far as the batch the Vault's next
    /// commit is named from.
    #[tokio::test]
    async fn a_summarized_write_reaches_the_batch_its_commit_will_be_named_from() {
        let workspace = workspace(Fixture::new(&[("Home.md", "# Home\n")]));

        workspace
            .summarized("tighten the intro")
            .update_note("home", "# Home\n\nrewritten\n", &hash("# Home\n"), None)
            .await
            .expect("update");

        let batch = workspace.pending_writes();
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].op, "update");
        assert_eq!(batch[0].target, "Home");
        assert_eq!(batch[0].summary.as_deref(), Some("tighten the intro"));
        assert_eq!(
            batch[0].origin,
            crate::git::WriteOrigin::mcp(Some("claude-code")),
            "the origin the target was built with is recorded with the write"
        );
        assert_eq!(
            batch[0].affected_paths,
            vec![workspace.vault_path.join("Home.md")]
        );
    }

    /// A burst of writes is what one debounced commit coalesces, so the batch
    /// has to keep every summary in the order they were written.
    #[tokio::test]
    async fn every_write_in_a_burst_keeps_its_own_summary() {
        let workspace = workspace(Fixture::new(&[("Home.md", "# Home\n")]));

        workspace
            .summarized("first")
            .append_to_note("home", "one\n", &hash("# Home\n"))
            .await
            .expect("append");
        workspace
            .summarized("second")
            .create_note("Second.md", "# Second\n", false)
            .await
            .expect("create");

        let summaries: Vec<_> = workspace
            .pending_writes()
            .into_iter()
            .filter_map(|record| record.summary)
            .collect();
        assert_eq!(summaries, vec!["first".to_string(), "second".to_string()]);
    }

    /// A write from a surface with no `commit_summary` argument — every HTTP
    /// route — still shapes the commit title, it just adds no body line.
    #[tokio::test]
    async fn an_unsummarized_write_still_reaches_the_batch() {
        let workspace = workspace(Fixture::new(&[("Home.md", "# Home\n")]));

        workspace
            .core()
            .delete_note(workspace.vault_id, "home", &hash("# Home\n"))
            .await
            .expect("delete");

        let batch = workspace.pending_writes();
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].op, "delete");
        assert_eq!(
            batch[0].target, "Home",
            "a delete is recorded where it was, not in the trash"
        );
        assert_eq!(batch[0].summary, None);
        assert_eq!(batch[0].origin, crate::git::WriteOrigin::web());
    }

    /// A refused write changed nothing on disk, so there is nothing for a
    /// commit message to claim.
    #[tokio::test]
    async fn a_refused_write_records_nothing() {
        let workspace = workspace(Fixture::new(&[("Home.md", "# Home\n")]));

        workspace
            .summarized("never happened")
            .update_note("home", "# Home\n\nrewritten\n", &hash("stale"), None)
            .await
            .expect_err("stale hash is refused");

        assert!(workspace.pending_writes().is_empty());
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
                None,
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
                None,
            )
            .await
            .expect_err("stale hash must be refused");

        assert_code(&error, "write_conflict");
        assert!(error.retryable);
        assert_eq!(workspace.read("Home.md"), "# Home\n");
    }

    /// #627: a same-named note earlier in path order takes the slug over
    /// (ADR-24). Same content means the same hash, so only the path can tell
    /// the opened note from the one now answering to its slug.
    #[tokio::test]
    async fn update_note_refuses_when_the_slug_now_names_another_file() {
        let workspace = workspace(Fixture::new(&[
            ("z/Plan.md", "body\n"),
            ("a/Plan.md", "body\n"),
        ]));
        let error = workspace
            .core()
            .update_note(
                workspace.vault_id,
                "plan",
                "typed in z\n",
                &hash("body\n"),
                Some("z/Plan"),
            )
            .await
            .expect_err("a slug naming another file must be refused");

        assert_code(&error, "note_moved");
        assert!(!error.retryable);
        assert_eq!(workspace.read("a/Plan.md"), "body\n");
        assert_eq!(workspace.read("z/Plan.md"), "body\n");
    }

    #[tokio::test]
    async fn update_note_writes_when_the_expected_path_still_matches() {
        let workspace = workspace(Fixture::new(&[
            ("z/Plan.md", "body\n"),
            ("a/Plan.md", "body\n"),
        ]));
        workspace
            .core()
            .update_note(
                workspace.vault_id,
                "plan-2",
                "typed in z\n",
                &hash("body\n"),
                Some("z/Plan"),
            )
            .await
            .expect("the note at the expected path takes the write");

        assert_eq!(workspace.read("z/Plan.md"), "typed in z\n");
        assert_eq!(workspace.read("a/Plan.md"), "body\n");
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
            .update_note(workspace.vault_id, "home", "x", &hash("# Home\n"), None)
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
            .update_note(workspace.vault_id, "home", "x", &hash("# Home\n"), None)
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

    /// #630: refused as a retryable conflict, so a client retried into the
    /// same symlink, each time leaving a sidecar holding its content.
    #[tokio::test]
    async fn an_overwrite_onto_a_symlinked_note_is_refused_for_good_and_leaves_nothing() {
        let workspace = workspace(Fixture::new(&[]));
        let outside = tempfile::tempdir().expect("outside");
        let target = outside.path().join("target.md");
        std::fs::write(&target, "outside\n").expect("outside note");
        std::os::unix::fs::symlink(&target, workspace.vault_path.join("Out.md")).expect("link");

        let error = workspace
            .core()
            .create_note(workspace.vault_id, "Out.md", "# Leaked\n", true)
            .await
            .expect_err("a symlinked note must not be replaced");

        assert_code(&error, "invalid_write_input");
        assert!(!error.retryable);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "outside\n");
        let names: Vec<_> = std::fs::read_dir(&workspace.vault_path)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["Out.md".to_string()]);
    }

    /// #630: a root stored through a symlink reported itself writable while
    /// every write failed, because the write layer refuses to follow a
    /// symlink anywhere above a note. The API stores canonical paths, so
    /// only a hand-edited or old registry holds one.
    #[tokio::test]
    async fn a_vault_whose_stored_root_is_a_symlink_takes_writes() {
        let workspace = workspace(Fixture::new(&[("Home.md", "# Home\n")]));
        let state = workspace._directory.path().join("state/vaults.json");
        let link = workspace._directory.path().join("vlink");
        std::os::unix::fs::symlink(&workspace.vault_path, &link).expect("root link");
        let real = std::fs::canonicalize(&workspace.vault_path).expect("canonical root");
        let registry = std::fs::read_to_string(&state).expect("registry");
        let real_json = serde_json::to_string(&real).expect("real path as JSON");
        let link_json = serde_json::to_string(&link).expect("link path as JSON");
        assert!(registry.contains(&real_json), "registry stores the root");
        std::fs::write(&state, registry.replace(&real_json, &link_json)).expect("hand edit");
        let store = VaultRegistryStore::new(state);
        let crate::vault_registry::VaultRegistryState::Ready(snapshot) =
            store.load().expect("reload registry")
        else {
            panic!("registry recovery");
        };
        assert_eq!(
            store.vault_path(&snapshot.definitions().next().expect("definition")),
            link,
            "the registry now stores the symlink"
        );
        // A restart is what activates a hand-edited registry.
        let restarted = VaultCollectionRuntime::new();
        restarted.reconcile(&store, &snapshot);
        let core = VaultMutationCore::new(
            &workspace.cache,
            &restarted,
            std::sync::Arc::clone(&workspace.settings),
            crate::git::WriteOrigin::web(),
        );

        let capabilities = core
            .write_capabilities(workspace.vault_id)
            .expect("capabilities");
        assert!(capabilities.enabled());
        core.create_note(workspace.vault_id, "New.md", "# New\n", false)
            .await
            .expect("a Vault reported writable must take a write");
        core.append_to_note(workspace.vault_id, "home", "more\n", &hash("# Home\n"))
            .await
            .expect("an existing note, reached through the index, takes a write too");
        assert_eq!(workspace.read("New.md"), "# New\n");
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
        let workspace = workspace(Fixture::new(&[]).excluding(&["*.excalidraw.md"]));
        let error = workspace
            .core()
            .create_note(
                workspace.vault_id,
                "Notes/Drawing.excalidraw",
                "# Ignored\n",
                false,
            )
            .await
            .expect_err("noise target must be refused");

        assert_code(&error, "noise_excluded_write");
        assert!(!workspace.exists("Notes/Drawing.excalidraw.md"));
    }

    #[tokio::test]
    async fn create_note_cannot_overwrite_trash_through_backslashes() {
        let workspace = workspace(Fixture::new(&[(
            ".hatchdoor-trash/Precious.md",
            "precious bytes\n",
        )]));
        for path in [
            ".hatchdoor-trash/Precious.md",
            r".hatchdoor-trash\Precious.md",
        ] {
            let error = workspace
                .core()
                .create_note(workspace.vault_id, path, "CLOBBERED", true)
                .await
                .expect_err("trash must be protected for either separator");
            assert_code(&error, "noise_excluded_write");
            assert_eq!(
                workspace.read(".hatchdoor-trash/Precious.md"),
                "precious bytes\n"
            );
        }
    }

    #[tokio::test]
    async fn create_note_matches_the_final_md_filename_not_the_callers_extension() {
        let workspace = workspace(Fixture::new(&[]));
        let created = workspace
            .core()
            .create_note(workspace.vault_id, r"Notes\x.tmp", "visible\n", false)
            .await
            .expect("a Markdown note is not a temporary file");
        assert_eq!(created.relative_path.as_deref(), Some("Notes/x.tmp"));
        assert_eq!(workspace.read("Notes/x.tmp.md"), "visible\n");
    }

    #[tokio::test]
    async fn note_exclusion_checks_preserve_whitespace_in_path_components() {
        let workspace = workspace(Fixture::new(&[]).excluding(&["[ ]private/"]));
        let error = workspace
            .core()
            .create_note(workspace.vault_id, "./ private/Secret", "hidden\n", false)
            .await
            .expect_err("the actual space-prefixed destination is excluded");
        assert_code(&error, "noise_excluded_write");
        assert!(!workspace.exists(" private/Secret.md"));
    }

    #[tokio::test]
    async fn note_exclusion_checks_allow_visible_whitespace_prefixed_folders() {
        let workspace = workspace(Fixture::new(&[]));
        workspace
            .core()
            .create_note(workspace.vault_id, "./ .obsidian/Note", "visible\n", false)
            .await
            .expect("a space-prefixed folder is not the Obsidian configuration folder");
        assert_eq!(workspace.read(" .obsidian/Note.md"), "visible\n");
        assert!(!workspace.exists(".obsidian"));
    }

    #[tokio::test]
    async fn attachment_rename_checks_preserve_whitespace_in_the_source_directory() {
        let workspace = workspace(
            Fixture::new(&[(" Media/payload.png", "payload\n")])
                .excluding(&["[ ]Media/blocked.png"]),
        );
        let error = workspace
            .core()
            .rename_attachment(workspace.vault_id, "./ Media/payload.png", "blocked.png")
            .await
            .expect_err("the actual sibling destination is excluded");
        assert_code(&error, "noise_excluded_write");
        assert_eq!(workspace.read(" Media/payload.png"), "payload\n");
        assert!(!workspace.exists(" Media/blocked.png"));
    }

    #[tokio::test]
    async fn note_moves_and_archive_check_normalized_destinations() {
        for folder in ["private/", r"private\"] {
            let workspace = workspace(
                Fixture::new(&[("Home.md", "# Home\n")])
                    .excluding(&["private/", "*.excalidraw.md"])
                    .archiving_into(folder),
            );
            let core = workspace.core();
            let id = workspace.vault_id;
            let current = hash("# Home\n");
            for error in [
                core.move_note(id, "home", folder, &current)
                    .await
                    .unwrap_err(),
                core.move_rename_note(id, "home", &format!("{folder}Home"), &current)
                    .await
                    .unwrap_err(),
                core.move_rename_note(id, "home", "Drawing.excalidraw", &current)
                    .await
                    .unwrap_err(),
                core.archive_note(id, "home", &current).await.unwrap_err(),
            ] {
                assert_code(&error, "noise_excluded_write");
            }
            assert_eq!(workspace.read("Home.md"), "# Home\n");
            assert!(!workspace.exists("private"));
            assert!(!workspace.exists("Drawing.excalidraw.md"));
        }
    }

    #[tokio::test]
    async fn attachment_destinations_and_upload_checks_cannot_bypass_exclusions() {
        let workspace = workspace(
            Fixture::new(&[("Media/payload.png", "payload\n")])
                .excluding(&["Media/blocked.png", "*.excalidraw.md"]),
        );
        let core = workspace.core();
        let id = workspace.vault_id;
        for target in [
            ".obsidian/plugins/x/main.js",
            r".obsidian\plugins\x\main.js",
        ] {
            for error in [
                core.move_attachment(id, "Media/payload.png", target)
                    .await
                    .unwrap_err(),
                core.import_attachment(id, target, b"bytes".to_vec(), 1024, false)
                    .await
                    .unwrap_err(),
                core.check_attachment_import(id, target, false)
                    .await
                    .unwrap_err(),
            ] {
                assert_code(&error, "noise_excluded_write");
            }
            assert_eq!(workspace.read("Media/payload.png"), "payload\n");
            assert!(!workspace.exists(".obsidian"));
        }
        for source in ["Media/payload.png", r"Media\payload.png"] {
            let error = core
                .rename_attachment(id, source, " blocked.png ")
                .await
                .unwrap_err();
            assert_code(&error, "noise_excluded_write");
            assert_eq!(workspace.read("Media/payload.png"), "payload\n");
            assert!(!workspace.exists("Media/blocked.png"));
        }
        // A note upload writes its target as named (#637); a note pattern
        // still covers every case of the extension.
        for path in [
            "Notes/Drawing.excalidraw.md",
            r"Notes\Drawing.excalidraw.MD",
        ] {
            let error = core.check_note_upload(id, path, None).await.unwrap_err();
            assert_code(&error, "noise_excluded_write");
            let error = core
                .upload_note(id, path, b"drawing".to_vec(), 1024, None)
                .await
                .unwrap_err();
            assert_code(&error, "noise_excluded_write");
        }
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

    #[tokio::test]
    async fn deleting_an_attachment_refuses_the_reserved_layer_marker_basename() {
        // #247: delete was the one attachment operation without this guard,
        // covered only by the upload allowlist the write layer no longer
        // applies to a file already in the Vault. Trashing a marker promotes
        // its whole folder back onto the default surface.
        let workspace = workspace(Fixture::new(&[("wiki/Home.md", "# Home\n")]));
        std::fs::write(
            workspace.vault_path.join("wiki/.hatchdoor-layer"),
            "name: wiki\n",
        )
        .expect("marker");

        let error = workspace
            .core()
            .delete_attachment(workspace.vault_id, "wiki/.hatchdoor-layer")
            .await
            .expect_err("marker must be refused");

        assert_code(&error, "layer_marker_write");
        assert!(workspace.exists("wiki/.hatchdoor-layer"));
    }

    #[tokio::test]
    async fn attachment_operations_refuse_a_source_the_vault_excludes_as_noise() {
        // #247: dropping the write layer's upload allowlist took with it the
        // only thing keeping these tools out of an Obsidian configuration
        // folder, which the Vault already says is not content. The destination
        // was always checked; the source is now checked too.
        let workspace = workspace(Fixture::new(&[("Home.md", "# Home\n")]));
        std::fs::create_dir_all(workspace.vault_path.join(".obsidian")).expect("obsidian dir");
        std::fs::write(workspace.vault_path.join(".obsidian/app.json"), "{}").expect("config");
        let core = workspace.core();
        let vault_id = workspace.vault_id;

        for source in [".obsidian/app.json", r".obsidian\app.json"] {
            for error in [
                core.move_attachment(vault_id, source, "Media/app.json")
                    .await
                    .expect_err("move must refuse"),
                core.rename_attachment(vault_id, source, "stolen.json")
                    .await
                    .expect_err("rename must refuse"),
                core.delete_attachment(vault_id, source)
                    .await
                    .expect_err("delete must refuse"),
            ] {
                assert_code(&error, "noise_excluded_write");
            }
            assert_eq!(workspace.read(".obsidian/app.json"), "{}");
        }
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

    /// #631: every multi-note write the core runs records its plan in the
    /// Vault's intent log before its first step. With the log's folder
    /// blocked, each one is refused and changes nothing, which shows the
    /// core hands every one of them the log.
    #[tokio::test]
    async fn every_multi_note_write_records_its_plan_first() {
        let workspace = workspace(Fixture::new(&[
            ("Home.md", "---\ntags: [old]\n---\n# Home\n![[pic.png]]\n"),
            ("Other.md", "# Other\n[[Home]]\n"),
            ("pic.png", "png"),
        ]));
        let state = workspace
            ._directory
            .path()
            .join("state")
            .join("write-intents");
        std::fs::write(&state, "a file where the record folder goes").expect("block");
        let core = workspace.core();
        let id = workspace.vault_id;
        let home = hash("---\ntags: [old]\n---\n# Home\n![[pic.png]]\n");

        let refusals = [
            core.rename_note(id, "home", "Renamed", &home)
                .await
                .map(|_| ()),
            core.move_note(id, "home", "Folder", &home)
                .await
                .map(|_| ()),
            core.archive_note(id, "home", &home).await.map(|_| ()),
            core.delete_note(id, "home", &home).await.map(|_| ()),
            core.move_attachment(id, "pic.png", "media/pic.png")
                .await
                .map(|_| ()),
            core.rename_attachment(id, "pic.png", "photo.png")
                .await
                .map(|_| ()),
            core.delete_attachment(id, "pic.png").await.map(|_| ()),
        ];
        for refusal in refusals {
            let error = refusal.expect_err("refused without a record");
            assert!(
                error.message.contains("could not record this change"),
                "{error:?}"
            );
        }
        let plan = core
            .rename_tag(id, "old", "new", None)
            .await
            .expect("a plan writes nothing and needs no record");
        let error = core
            .rename_tag(id, "old", "new", plan.plan_hash.as_deref())
            .await
            .expect_err("refused without a record");
        assert!(
            error.message.contains("could not record this change"),
            "{error:?}"
        );
        let plan = core
            .delete_tag(id, "old", None)
            .await
            .expect("a plan writes nothing and needs no record");
        let error = core
            .delete_tag(id, "old", plan.plan_hash.as_deref())
            .await
            .expect_err("refused without a record");
        assert!(
            error.message.contains("could not record this change"),
            "{error:?}"
        );

        assert!(workspace.exists("Home.md"));
        assert!(workspace.exists("pic.png"));
        assert_eq!(
            std::fs::read_to_string(workspace.vault_path.join("Home.md")).expect("home"),
            "---\ntags: [old]\n---\n# Home\n![[pic.png]]\n"
        );
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
    async fn replace_section_reports_heading_not_found_without_changing_frontmatter() {
        let body = "---\ntitle: W\n# status: draft\ntags: [a]\n---\nIntro\n\n## Log\nMonday\n";
        let workspace = workspace(Fixture::new(&[("Home.md", body)]));
        let core = workspace.core();
        for mode in [
            SectionMode::Replace,
            SectionMode::Before,
            SectionMode::After,
        ] {
            let error = core
                .replace_section(
                    workspace.vault_id,
                    "home",
                    "# status: draft",
                    mode,
                    "new\n",
                    &hash(body),
                )
                .await
                .expect_err("frontmatter is not a section");
            assert_code(&error, "heading_not_found");
            assert!(!error.retryable);
            assert_eq!(workspace.read("Home.md"), body);
        }
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
            core.update_note(vault_id, "nowhere", "x", &irrelevant, None)
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
            core.update_note(vault_id, "   ", "x", &irrelevant, None)
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
        assert_eq!(
            capabilities.atomic_compare_and_swap,
            Some(true),
            "an ordinary filesystem commits a conditional write in one step"
        );
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
        assert_ne!(
            capabilities.atomic_compare_and_swap,
            Some(false),
            "a read-only Vault has its own reason to report and must never be \
             blamed on its filesystem; the answer here is about the filesystem, \
             which has not changed (#345)"
        );
    }

    /// The report has to separate "writes work, with weaker protection" from
    /// "writes do not work", because they call for different answers from the
    /// operator (#345).
    #[tokio::test]
    async fn write_capabilities_report_a_filesystem_that_cannot_compare_and_swap() {
        let workspace = workspace(Fixture::new(&[("Home.md", "# Home\n")]));
        crate::rename_flags::force_unsupported_for_tests(&workspace.vault_path);

        let capabilities = workspace
            .core()
            .write_capabilities(workspace.vault_id)
            .expect("capabilities");

        assert_eq!(capabilities.atomic_compare_and_swap, Some(false));
        assert!(
            capabilities.enabled(),
            "the Vault is still writable, just without the atomic commit"
        );
    }

    /// The bug as reported: on such a filesystem every edit of an existing
    /// note failed with `write_failed: Invalid argument (os error 22)` while
    /// creating one worked (#345).
    #[tokio::test]
    async fn note_writes_go_through_the_mutation_core_without_an_exchange() {
        let workspace = workspace(Fixture::new(&[("Home.md", "# Home\n")]));
        crate::rename_flags::force_unsupported_for_tests(&workspace.vault_path);
        let core = workspace.core();

        core.append_to_note(workspace.vault_id, "home", "appended\n", &hash("# Home\n"))
            .await
            .expect("append must work without an exchange");
        assert!(workspace.read("Home.md").contains("appended"));

        let current = hash(&workspace.read("Home.md"));
        core.edit_note(
            workspace.vault_id,
            "home",
            "appended",
            "edited",
            &current,
            false,
        )
        .await
        .expect("edit must work without an exchange");
        assert!(workspace.read("Home.md").contains("edited"));

        let stale = core
            .append_to_note(workspace.vault_id, "home", "more\n", &hash("# Home\n"))
            .await
            .expect_err("a stale hash must still be refused");
        assert_code(&stale, "write_conflict");
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
                None,
            )
            .await
            .expect_err("unknown Vault");
        assert_code(&error, "vault_not_found");
    }
}
