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
//! Tracer bullet (#184): `update_note` and `archive_note` only. The remaining
//! primitives follow in #186, at which point the adapters' own index-build,
//! entry-lookup, and noise-refusal helpers disappear with them.

use std::sync::Arc;

use crate::app_state::AppState;
use crate::cache::SqliteCache;
use crate::runtime_config::ConfigSnapshot;
use crate::vault::{
    ExcludeMatcher, LayerMap, NoteEntry, VaultIndex, WriteError, WriteOutcome, archive_note,
    update_note,
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
    /// [`VaultMutationCore::update_note`] and [`VaultMutationCore::archive_note`]
    /// take and release it around the write. A caller whose critical section
    /// is wider than one operation — the MCP `batch` tool, which holds one
    /// Vault's lock for a whole call — builds its target with
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

    /// Replace one note's whole content, under optimistic concurrency by
    /// expected content hash, holding this Vault's mutation lock for the
    /// write.
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

    /// Move one note into this Vault's archive folder, under optimistic
    /// concurrency by expected content hash, holding this Vault's mutation
    /// lock for the write.
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
    /// operations, so it cannot go through
    /// [`VaultMutationCore::update_note`]'s one-shot gate-lock-write. Reusing
    /// the block it already holds also keeps every operation in that call on
    /// one Vault generation, where a fresh lookup could observe a
    /// reconciled replacement mid-batch.
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

    /// Replace one note's whole content. The caller must already hold this
    /// Vault's mutation lock.
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

    /// Move one note into this Vault's archive folder. The caller must
    /// already hold this Vault's mutation lock.
    pub async fn archive_note(
        &self,
        slug: &str,
        expected_content_hash: &str,
    ) -> Result<NoteWriteOutcome, VaultOperationError> {
        let index = self.authoritative_index().await?;
        let entry = self.note_entry(&index, slug)?;
        let archive_prefix = self.archive_prefix()?;
        let archive_folder = archive_prefix.trim().trim_matches('/');
        let file_name = entry
            .relative_path
            .rsplit('/')
            .next()
            .unwrap_or(&entry.relative_path);
        self.reject_noise_write(&format!("{archive_folder}/{file_name}"))?;

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
        let vault_id = self.vault_id;
        let control = self.control.clone();
        match tokio::task::spawn_blocking(move || control.authoritative_index()).await {
            Ok(Ok(index)) => Ok(index),
            Ok(Err(error)) => Err(crate::vault_read::runtime_error(vault_id, error).into()),
            Err(join_error) => Err(VaultOperationError::new(
                "vault_read_unavailable",
                format!("vault index build panicked: {join_error}"),
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

    /// Runs a synchronous `vault/write` primitive on the blocking pool.
    /// Moves rewrite every backlinking note, which must not stall a tokio
    /// worker; a panic maps to a `write_failed` error instead of unwinding
    /// through the adapter. Both surfaces offload, because the core does.
    async fn run_write(
        &self,
        op: impl FnOnce() -> Result<WriteOutcome, WriteError> + Send + 'static,
    ) -> Result<WriteOutcome, VaultOperationError> {
        let result = tokio::task::spawn_blocking(op)
            .await
            .unwrap_or_else(|join_error| {
                Err(WriteError::Io(format!("write task panicked: {join_error}")))
            });
        result.map_err(|error| self.write_error(error))
    }

    /// A partially-applied multi-phase mutation needs operator action, so its
    /// message survives under its own code rather than collapsing into the
    /// generic `write_failed` every other `Io` failure gets. What each
    /// surface then shows the caller — a sanitized 500 over HTTP, the message
    /// over MCP — is the adapter's mapping, not this core's business.
    fn write_error(&self, error: WriteError) -> VaultOperationError {
        if let Some(message) = error.recovery_message() {
            return VaultOperationError::new(
                "write_recovery_required",
                message.to_string(),
                Some(self.vault_id),
                false,
            );
        }
        let (code, message, retryable) = match error {
            WriteError::Conflict(message) => ("write_conflict", message, true),
            WriteError::InvalidInput(message) => ("invalid_write_input", message, false),
            WriteError::Io(message) => ("write_failed", message, false),
        };
        VaultOperationError::new(code, message, Some(self.vault_id), retryable)
    }

    fn internal(&self, message: impl Into<String>) -> VaultOperationError {
        VaultOperationError::new("internal_error", message, Some(self.vault_id), false)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tempfile::TempDir;

    use super::{VaultMutationCore, VaultOperationError};
    use crate::cache::SqliteCache;
    use crate::runtime_config::{ConfigSnapshot, RuntimeConfig};
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
    async fn mutations_refuse_an_unknown_slug() {
        let workspace = workspace(Fixture::new(&[("Home.md", "# Home\n")]));
        let update = workspace
            .core()
            .update_note(workspace.vault_id, "nowhere", "x", &hash("# Home\n"))
            .await
            .expect_err("unknown slug");
        assert_code(&update, "note_not_found");

        let archive = workspace
            .core()
            .archive_note(workspace.vault_id, "nowhere", &hash("# Home\n"))
            .await
            .expect_err("unknown slug");
        assert_code(&archive, "note_not_found");
        assert_eq!(archive.vault_id, Some(workspace.vault_id));
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
