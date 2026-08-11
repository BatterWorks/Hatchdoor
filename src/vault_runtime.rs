use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::path::PathBuf;
#[cfg(test)]
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock, Weak};

use serde::Serialize;
use tracing::error;

use crate::cache::SqliteCache;
use crate::cache::vault_snapshots::VaultSnapshotFreshness;
use crate::embed::Embedder;
use crate::git::{
    ManagedCheckoutLease, ManagedGitScheduler, ManagedGitTurnConfig, run_existing_git_remote_turn,
    run_managed_git_turn,
};
use crate::startup::IndexingProgressSnapshot;
use crate::vault_registry::{
    VaultDefinition, VaultGitMode, VaultId, VaultRegistrySnapshot, VaultRegistryStore,
    VaultSource as RegistryVaultSource,
};
use crate::vault_watcher::{VaultWatcherHandle, spawn_vault_change_watcher};
use crate::vault_work::{VaultWorkCoordinator, VaultWorkError, VaultWorkKind, VaultWorkRequest};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VaultSource {
    Local { vault_path: PathBuf },
    ManagedGit(ManagedGitSource),
}

impl VaultSource {
    pub fn kind(&self) -> VaultSourceKind {
        match self {
            Self::Local { .. } => VaultSourceKind::Local,
            Self::ManagedGit(_) => VaultSourceKind::ManagedGit,
        }
    }

    pub fn mode(&self) -> VaultSourceMode {
        match self {
            Self::Local { .. } => VaultSourceMode::Local,
            Self::ManagedGit(source) => source.mode.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManagedGitSource {
    pub repository_url: String,
    pub checkout_path: PathBuf,
    pub branch: Option<String>,
    pub vault_subdirectory: Option<PathBuf>,
    pub mode: ManagedGitMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManagedGitMode {
    PullOnly,
    Bidirectional,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum VaultSourceKind {
    Local,
    ManagedGit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum VaultSourceMode {
    Local,
    PullOnly,
    Bidirectional,
}

impl From<ManagedGitMode> for VaultSourceMode {
    fn from(value: ManagedGitMode) -> Self {
        match value {
            ManagedGitMode::PullOnly => Self::PullOnly,
            ManagedGitMode::Bidirectional => Self::Bidirectional,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultPhase {
    TermsRequired,
    Downloading,
    Validating,
    Scanning,
    Indexing,
    Ready,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct VaultCapabilities {
    pub browse: bool,
    pub search: bool,
    pub mutate: bool,
    pub pull: bool,
    pub push: bool,
    pub retry: bool,
}

impl VaultCapabilities {
    fn derive(source_mode: VaultSourceMode, phase: VaultPhase) -> Self {
        let ready = phase == VaultPhase::Ready;
        Self {
            browse: ready,
            search: ready,
            mutate: ready
                && matches!(
                    source_mode,
                    VaultSourceMode::Local | VaultSourceMode::Bidirectional
                ),
            pull: ready
                && matches!(
                    source_mode,
                    VaultSourceMode::PullOnly | VaultSourceMode::Bidirectional
                ),
            push: ready && source_mode == VaultSourceMode::Bidirectional,
            retry: false,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct VaultRuntimeSnapshot {
    pub phase: VaultPhase,
    pub source: VaultSourceKind,
    pub mode: VaultSourceMode,
    pub capabilities: VaultCapabilities,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub downloaded_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indexing: Option<IndexingProgressSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<VaultRuntimeError>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VaultRuntimeError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

impl VaultRuntimeSnapshot {
    fn new(source: &VaultSource, phase: VaultPhase) -> Self {
        let mode = source.mode();
        Self {
            phase,
            source: source.kind(),
            mode,
            capabilities: VaultCapabilities::derive(mode, phase),
            model: None,
            downloaded_bytes: None,
            total_bytes: None,
            indexing: None,
            error: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct VaultRuntime {
    source: Arc<VaultSource>,
    snapshot: Arc<RwLock<VaultRuntimeSnapshot>>,
}

impl VaultRuntime {
    pub fn new(source: VaultSource) -> Self {
        let snapshot = VaultRuntimeSnapshot::new(&source, VaultPhase::Validating);
        Self {
            source: Arc::new(source),
            snapshot: Arc::new(RwLock::new(snapshot)),
        }
    }

    pub fn ready(source: VaultSource) -> Self {
        let runtime = Self::new(source);
        runtime.set_phase(VaultPhase::Ready);
        runtime
    }

    pub fn source(&self) -> &VaultSource {
        &self.source
    }

    pub fn snapshot(&self) -> VaultRuntimeSnapshot {
        self.snapshot
            .read()
            .expect("vault runtime snapshot poisoned")
            .clone()
    }

    pub fn is_ready(&self) -> bool {
        self.snapshot().phase == VaultPhase::Ready
    }

    pub fn set_scanning(&self) {
        self.set_phase(VaultPhase::Scanning);
    }

    pub fn set_terms_required(&self) {
        self.set_phase(VaultPhase::TermsRequired);
    }

    pub fn set_downloading(
        &self,
        model: &'static str,
        downloaded_bytes: Option<u64>,
        total_bytes: Option<u64>,
    ) {
        let mut snapshot = self
            .snapshot
            .write()
            .expect("vault runtime snapshot poisoned");
        snapshot.phase = VaultPhase::Downloading;
        snapshot.capabilities = VaultCapabilities::derive(snapshot.mode, snapshot.phase);
        snapshot.model = Some(model);
        snapshot.downloaded_bytes = downloaded_bytes;
        snapshot.total_bytes = total_bytes;
        snapshot.indexing = None;
        snapshot.error = None;
    }

    pub fn set_indexing(&self, progress: IndexingProgressSnapshot) {
        let mut snapshot = self
            .snapshot
            .write()
            .expect("vault runtime snapshot poisoned");
        snapshot.phase = VaultPhase::Indexing;
        snapshot.capabilities = VaultCapabilities::derive(snapshot.mode, snapshot.phase);
        snapshot.model = None;
        snapshot.downloaded_bytes = None;
        snapshot.total_bytes = None;
        snapshot.indexing = Some(progress);
        snapshot.error = None;
    }

    pub fn set_ready(&self) {
        self.set_phase(VaultPhase::Ready);
    }

    pub fn set_unavailable(&self, code: impl Into<String>, message: impl Into<String>) {
        let mut snapshot = self
            .snapshot
            .write()
            .expect("vault runtime snapshot poisoned");
        snapshot.phase = VaultPhase::Unavailable;
        snapshot.capabilities = VaultCapabilities::derive(snapshot.mode, snapshot.phase);
        snapshot.model = None;
        snapshot.downloaded_bytes = None;
        snapshot.total_bytes = None;
        snapshot.indexing = None;
        snapshot.error = Some(VaultRuntimeError {
            code: code.into(),
            message: message.into(),
            retryable: false,
        });
    }

    fn set_phase(&self, phase: VaultPhase) {
        let mut snapshot = self
            .snapshot
            .write()
            .expect("vault runtime snapshot poisoned");
        snapshot.phase = phase;
        snapshot.capabilities = VaultCapabilities::derive(snapshot.mode, phase);
        snapshot.model = None;
        snapshot.downloaded_bytes = None;
        snapshot.total_bytes = None;
        snapshot.indexing = None;
        snapshot.error = None;
    }
}

/// Activation state for one definition in the live Vault collection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultActivationStatus {
    Active,
    Disabled,
    Unavailable,
}

/// Whether authoritative local Markdown can currently be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalContentStatus {
    ReadWrite,
    ReadOnly,
    Unavailable,
}

/// Search availability is independent from local Markdown availability.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultSearchStatus {
    Unavailable,
    Indexing,
    Ready,
    Stale,
}

/// Git status is kept separate so a Git failure cannot hide local Markdown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultGitStatus {
    Disabled,
    Pending,
    Ready,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultWatcherStatus {
    Running,
    Disabled,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CollectionVaultSnapshot {
    pub vault_id: VaultId,
    pub name: String,
    pub enabled: bool,
    pub activation: VaultActivationStatus,
    pub local_content: LocalContentStatus,
    pub search: VaultSearchStatus,
    pub git: VaultGitStatus,
    pub watcher: VaultWatcherStatus,
    pub capabilities: VaultCapabilities,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activation_error: Option<VaultRuntimeError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_error: Option<VaultRuntimeError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_error: Option<VaultRuntimeError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub watcher_error: Option<VaultRuntimeError>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VaultCollectionSnapshot {
    pub registry_revision: u64,
    pub collection_revision: u64,
    pub vaults: BTreeMap<VaultId, CollectionVaultSnapshot>,
}

/// Broad category of a published collection-revision change, so a subscriber
/// can decide how widely to refetch without inspecting every field that
/// changed. `Definition` covers registry-level changes reconciled into the
/// live collection (added/edited/enabled/disabled/disconnected); `Status`
/// covers one Vault's runtime capability/search/Git/watcher status moving.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultChangeCategory {
    Definition,
    Status,
}

/// One collection-revision advance, published to every subscriber. Since the
/// underlying channel keeps only the latest value, a subscriber that misses
/// intermediate advances still learns the current `collection_revision` and
/// should treat a gap as "refetch broadly" rather than trust `vault_ids` to be
/// a complete history — the same lightweight-invalidation tradeoff the
/// existing single-Vault `/api/vault-events` stream already makes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VaultCollectionRevisionEvent {
    pub collection_revision: u64,
    pub vault_ids: Vec<VaultId>,
    pub category: VaultChangeCategory,
}

impl VaultCollectionRevisionEvent {
    fn initial() -> Self {
        Self {
            collection_revision: 0,
            vault_ids: Vec::new(),
            category: VaultChangeCategory::Definition,
        }
    }
}

#[derive(Clone)]
pub struct VaultControlBlock {
    definition: Arc<VaultDefinition>,
    vault_path: Arc<PathBuf>,
    snapshot: Arc<RwLock<CollectionVaultSnapshot>>,
    mutation_lock: Arc<tokio::sync::Mutex<()>>,
    refresh_lock: Arc<tokio::sync::Mutex<()>>,
    accepting_operations: Arc<AtomicBool>,
    cancellation: tokio::sync::watch::Sender<bool>,
    revisions: CollectionRevisionPublisher,
    watcher: Arc<RwLock<Option<VaultWatcherHandle>>>,
}

impl VaultControlBlock {
    fn activate(
        definition: VaultDefinition,
        vault_path: PathBuf,
        watching: Option<&WatcherContext>,
        snapshot_cache: Option<&SqliteCache>,
        revisions: CollectionRevisionPublisher,
        prior_git: Option<(VaultGitStatus, Option<VaultRuntimeError>)>,
    ) -> Self {
        let mut snapshot = activation_snapshot(&definition, &vault_path, snapshot_cache, prior_git);
        let watcher = if snapshot.activation == VaultActivationStatus::Active {
            watching.and_then(|watching| {
                let exclude = match crate::vault::ExcludeMatcher::new(definition.exclude_patterns())
                {
                    Ok(exclude) => exclude,
                    Err(error) => {
                        snapshot.watcher = VaultWatcherStatus::Unavailable;
                        snapshot.watcher_error = Some(VaultRuntimeError {
                            code: "vault_watcher_unavailable".to_string(),
                            message: error,
                            retryable: true,
                        });
                        return None;
                    }
                };
                match spawn_vault_change_watcher(
                    definition.vault_id(),
                    vault_path.clone(),
                    watching.cache_db_path.as_ref().clone(),
                    exclude,
                    watching.changes.clone(),
                ) {
                    Ok(watcher) => {
                        snapshot.watcher = VaultWatcherStatus::Running;
                        Some(watcher)
                    }
                    Err(error) => {
                        snapshot.watcher = VaultWatcherStatus::Unavailable;
                        snapshot.watcher_error = Some(VaultRuntimeError {
                            code: "vault_watcher_unavailable".to_string(),
                            message: error,
                            retryable: true,
                        });
                        None
                    }
                }
            })
        } else {
            None
        };
        snapshot.capabilities = collection_capabilities(&definition, &snapshot);
        let (cancellation, _) = tokio::sync::watch::channel(false);
        Self {
            definition: Arc::new(definition),
            vault_path: Arc::new(vault_path),
            snapshot: Arc::new(RwLock::new(snapshot)),
            mutation_lock: Arc::new(tokio::sync::Mutex::new(())),
            refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
            accepting_operations: Arc::new(AtomicBool::new(true)),
            cancellation,
            revisions,
            watcher: Arc::new(RwLock::new(watcher)),
        }
    }

    pub fn definition(&self) -> &VaultDefinition {
        &self.definition
    }

    pub fn vault_path(&self) -> &Path {
        &self.vault_path
    }

    /// Build an authoritative index for an exact read. Collection projections
    /// use the shared disposable cache, but exact note, link, and resolve
    /// operations must always inspect this Vault's own Markdown directory.
    pub fn authoritative_index(&self) -> Result<crate::vault::VaultIndex, VaultRuntimeError> {
        self.ensure_accepting_operations()?;
        let exclude = crate::vault::ExcludeMatcher::new(self.definition.exclude_patterns())
            .map_err(|message| VaultRuntimeError {
                code: "vault_scan_config_invalid".to_string(),
                message,
                retryable: false,
            })?;
        crate::vault::VaultIndex::build_with_config(
            self.vault_path(),
            &crate::vault::VaultScanConfig { exclude },
        )
        .map_err(|error| VaultRuntimeError {
            code: "vault_read_unavailable".to_string(),
            message: format!(
                "Could not read Vault {} from '{}': {error}",
                self.definition.vault_id(),
                self.vault_path().display()
            ),
            retryable: true,
        })
    }

    /// Build this Vault's metadata-only catalog (slug/title/layer
    /// bookkeeping, no wikilink graph) for a write response that only needs
    /// to report a note's slug/layer after a commit already on disk. Cheaper
    /// than `authoritative_index`: it never reads a note's content, only its
    /// path.
    pub fn authoritative_catalog(&self) -> Result<crate::vault::VaultIndex, VaultRuntimeError> {
        self.ensure_accepting_operations()?;
        let exclude = crate::vault::ExcludeMatcher::new(self.definition.exclude_patterns())
            .map_err(|message| VaultRuntimeError {
                code: "vault_scan_config_invalid".to_string(),
                message,
                retryable: false,
            })?;
        crate::vault::VaultIndex::build_catalog_with_config(
            self.vault_path(),
            &crate::vault::VaultScanConfig { exclude },
        )
        .map_err(|error| VaultRuntimeError {
            code: "vault_read_unavailable".to_string(),
            message: format!(
                "Could not read Vault {} from '{}': {error}",
                self.definition.vault_id(),
                self.vault_path().display()
            ),
            retryable: true,
        })
    }

    pub fn snapshot(&self) -> CollectionVaultSnapshot {
        self.snapshot
            .read()
            .expect("Vault control snapshot poisoned")
            .clone()
    }

    pub async fn acquire_mutation(
        &self,
    ) -> Result<tokio::sync::OwnedMutexGuard<()>, VaultRuntimeError> {
        self.ensure_accepting_operations()?;
        let guard = self.mutation_lock.clone().lock_owned().await;
        self.ensure_accepting_operations()?;
        Ok(guard)
    }

    /// Wait until a mutation that was admitted before this control block was
    /// retired has completed. Callers must revoke operation admission first,
    /// so a queued mutation re-checks that state and cannot begin afterwards.
    async fn wait_for_mutation_safe_boundary(&self) {
        let _guard = self.mutation_lock.clone().lock_owned().await;
    }

    pub async fn acquire_refresh(
        &self,
    ) -> Result<tokio::sync::OwnedMutexGuard<()>, VaultRuntimeError> {
        self.ensure_accepting_operations()?;
        let guard = self.refresh_lock.clone().lock_owned().await;
        self.ensure_accepting_operations()?;
        Ok(guard)
    }

    pub fn subscribe_cancellation(&self) -> tokio::sync::watch::Receiver<bool> {
        self.cancellation.subscribe()
    }

    pub fn is_accepting_operations(&self) -> bool {
        self.accepting_operations.load(Ordering::SeqCst)
    }

    pub(crate) fn ensure_accepting_operations(&self) -> Result<(), VaultRuntimeError> {
        if self.is_accepting_operations() {
            return Ok(());
        }
        Err(VaultRuntimeError {
            code: "vault_runtime_not_active".to_string(),
            message: format!(
                "Vault runtime {} is no longer active",
                self.definition.vault_id()
            ),
            retryable: false,
        })
    }

    fn revoke(&self) {
        if self.accepting_operations.swap(false, Ordering::SeqCst) {
            self.cancellation.send_replace(true);
        }
        if let Some(watcher) = self
            .watcher
            .read()
            .expect("Vault watcher handle poisoned")
            .as_ref()
        {
            watcher.cancel();
        }
    }

    pub fn watcher_cancelled(&self) -> bool {
        self.watcher
            .read()
            .expect("Vault watcher handle poisoned")
            .as_ref()
            .is_some_and(VaultWatcherHandle::is_cancelled)
    }

    /// Publish search availability without changing local-content capability.
    /// The Vault-qualified cache packet owns the concrete transitions that call
    /// this seam.
    pub fn set_search_status(
        &self,
        status: VaultSearchStatus,
        error: Option<VaultRuntimeError>,
    ) -> Result<(), VaultRuntimeError> {
        self.ensure_accepting_operations()?;
        let mut snapshot = self
            .snapshot
            .write()
            .expect("Vault control snapshot poisoned");
        let previous = snapshot.clone();
        snapshot.search = status;
        snapshot.search_error = error;
        snapshot.capabilities = collection_capabilities(&self.definition, &snapshot);
        let changed = *snapshot != previous;
        drop(snapshot);
        if changed {
            self.revisions
                .bump(self.definition.vault_id(), VaultChangeCategory::Status);
        }
        Ok(())
    }

    /// Publish authoritative local-Markdown availability, without changing
    /// Git status. `activation_snapshot` stats `vault_path` only once, at
    /// `reconcile()` time; for a managed Git Vault that path does not exist
    /// until the per-Vault Git lifecycle packet completes first acquisition,
    /// so that packet owns the transitions that call this seam to make the
    /// Vault browsable once its checkout exists (or to report the source
    /// unavailable again, e.g. after the checkout is lost).
    pub fn set_local_content_status(
        &self,
        status: LocalContentStatus,
        error: Option<VaultRuntimeError>,
    ) -> Result<(), VaultRuntimeError> {
        self.ensure_accepting_operations()?;
        let mut snapshot = self
            .snapshot
            .write()
            .expect("Vault control snapshot poisoned");
        let previous = snapshot.clone();
        snapshot.local_content = status;
        snapshot.activation = if status == LocalContentStatus::Unavailable {
            VaultActivationStatus::Unavailable
        } else {
            VaultActivationStatus::Active
        };
        snapshot.activation_error = error;
        snapshot.capabilities = collection_capabilities(&self.definition, &snapshot);
        let changed = *snapshot != previous;
        drop(snapshot);
        if changed {
            self.revisions
                .bump(self.definition.vault_id(), VaultChangeCategory::Status);
        }
        Ok(())
    }

    /// Publish Git availability without changing authoritative local-content
    /// capability. The per-Vault Git lifecycle packet owns the operations that
    /// call this seam.
    pub fn set_git_status(
        &self,
        status: VaultGitStatus,
        error: Option<VaultRuntimeError>,
    ) -> Result<(), VaultRuntimeError> {
        self.ensure_accepting_operations()?;
        let mut snapshot = self
            .snapshot
            .write()
            .expect("Vault control snapshot poisoned");
        let previous = snapshot.clone();
        snapshot.git = status;
        snapshot.git_error = error;
        snapshot.capabilities = collection_capabilities(&self.definition, &snapshot);
        let changed = *snapshot != previous;
        drop(snapshot);
        if changed {
            self.revisions
                .bump(self.definition.vault_id(), VaultChangeCategory::Status);
        }
        Ok(())
    }

    /// Publish a `Definition`-category revision bump for this Vault without
    /// changing its runtime status snapshot. `reconcile()` retains this same
    /// `VaultControlBlock` unchanged whenever `VaultDefinition` equality
    /// can't see an edit — e.g. replacing an already-configured credential's
    /// value, where `credential_configured` stays `true` before and after —
    /// so it never bumps `collection_revision` or emits an event on its own
    /// for that case. Callers that know such a change happened (issue #98's
    /// reopening finding) use this to notify SSE subscribers explicitly.
    pub(crate) fn notify_definition_changed(&self) {
        self.revisions
            .bump(self.definition.vault_id(), VaultChangeCategory::Definition);
    }
}

#[derive(Clone)]
enum VaultCollectionEntry {
    Active(VaultControlBlock),
    Disabled(Box<CollectionVaultSnapshot>),
}

impl VaultCollectionEntry {
    fn snapshot(&self) -> CollectionVaultSnapshot {
        match self {
            Self::Active(runtime) => runtime.snapshot(),
            Self::Disabled(snapshot) => snapshot.as_ref().clone(),
        }
    }
}

#[cfg(test)]
type ReconcileTestHook = Arc<Mutex<Option<Arc<dyn Fn() + Send + Sync>>>>;

#[derive(Clone)]
pub struct VaultCollectionRuntime {
    state: Arc<RwLock<VaultCollectionState>>,
    revisions: tokio::sync::watch::Sender<VaultCollectionRevisionEvent>,
    watching: Option<WatcherContext>,
    snapshot_cache: Option<Arc<SqliteCache>>,
    reconcile_phase_lock: Arc<tokio::sync::Mutex<()>>,
    #[cfg(test)]
    after_reconcile_before_drain_hook: ReconcileTestHook,
    #[cfg(test)]
    after_post_wait_before_retirement_hook: ReconcileTestHook,
}

#[derive(Clone)]
struct CollectionRevisionPublisher {
    state: Weak<RwLock<VaultCollectionState>>,
    revisions: tokio::sync::watch::Sender<VaultCollectionRevisionEvent>,
}

impl CollectionRevisionPublisher {
    fn bump(&self, vault_id: VaultId, category: VaultChangeCategory) {
        let Some(state) = self.state.upgrade() else {
            return;
        };
        let event = {
            let mut state = state.write().expect("Vault collection runtime poisoned");
            state.collection_revision = state.collection_revision.saturating_add(1);
            VaultCollectionRevisionEvent {
                collection_revision: state.collection_revision,
                vault_ids: vec![vault_id],
                category,
            }
        };
        self.revisions.send_replace(event);
    }
}

#[derive(Clone)]
struct WatcherContext {
    cache_db_path: Arc<PathBuf>,
    changes: tokio::sync::broadcast::Sender<VaultId>,
}

struct VaultCollectionState {
    registry_revision: u64,
    collection_revision: u64,
    vaults: BTreeMap<VaultId, VaultCollectionEntry>,
}

impl VaultCollectionRuntime {
    #[cfg(test)]
    pub(crate) fn set_after_reconcile_before_drain_hook(
        &self,
        hook: Option<Arc<dyn Fn() + Send + Sync>>,
    ) {
        *self
            .after_reconcile_before_drain_hook
            .lock()
            .expect("reconcile phase hook poisoned") = hook;
    }

    #[cfg(test)]
    pub(crate) fn set_after_post_wait_before_retirement_hook(
        &self,
        hook: Option<Arc<dyn Fn() + Send + Sync>>,
    ) {
        *self
            .after_post_wait_before_retirement_hook
            .lock()
            .expect("post-wait hook poisoned") = hook;
    }

    pub fn new() -> Self {
        let (revisions, _) = tokio::sync::watch::channel(VaultCollectionRevisionEvent::initial());
        Self {
            state: Arc::new(RwLock::new(VaultCollectionState {
                registry_revision: 0,
                collection_revision: 0,
                vaults: BTreeMap::new(),
            })),
            revisions,
            watching: None,
            snapshot_cache: None,
            reconcile_phase_lock: Arc::new(tokio::sync::Mutex::new(())),
            #[cfg(test)]
            after_reconcile_before_drain_hook: Arc::new(Mutex::new(None)),
            #[cfg(test)]
            after_post_wait_before_retirement_hook: Arc::new(Mutex::new(None)),
        }
    }

    pub fn with_watching(cache_db_path: PathBuf) -> Self {
        let (changes, _) = tokio::sync::broadcast::channel(64);
        let (revisions, _) = tokio::sync::watch::channel(VaultCollectionRevisionEvent::initial());
        Self {
            state: Arc::new(RwLock::new(VaultCollectionState {
                registry_revision: 0,
                collection_revision: 0,
                vaults: BTreeMap::new(),
            })),
            revisions,
            watching: Some(WatcherContext {
                cache_db_path: Arc::new(cache_db_path),
                changes,
            }),
            snapshot_cache: None,
            reconcile_phase_lock: Arc::new(tokio::sync::Mutex::new(())),
            #[cfg(test)]
            after_reconcile_before_drain_hook: Arc::new(Mutex::new(None)),
            #[cfg(test)]
            after_post_wait_before_retirement_hook: Arc::new(Mutex::new(None)),
        }
    }

    /// Construct a watched collection that derives initial search capability
    /// from the existing disposable cache snapshot before rebuilt Index work
    /// has a chance to run.
    pub(crate) fn with_watching_and_cache(
        cache_db_path: PathBuf,
        snapshot_cache: Arc<SqliteCache>,
    ) -> Self {
        let (changes, _) = tokio::sync::broadcast::channel(64);
        let (revisions, _) = tokio::sync::watch::channel(VaultCollectionRevisionEvent::initial());
        Self {
            state: Arc::new(RwLock::new(VaultCollectionState {
                registry_revision: 0,
                collection_revision: 0,
                vaults: BTreeMap::new(),
            })),
            revisions,
            watching: Some(WatcherContext {
                cache_db_path: Arc::new(cache_db_path),
                changes,
            }),
            snapshot_cache: Some(snapshot_cache),
            reconcile_phase_lock: Arc::new(tokio::sync::Mutex::new(())),
            #[cfg(test)]
            after_reconcile_before_drain_hook: Arc::new(Mutex::new(None)),
            #[cfg(test)]
            after_post_wait_before_retirement_hook: Arc::new(Mutex::new(None)),
        }
    }

    /// Reconcile live control blocks to one authoritative registry snapshot.
    /// Existing enabled runtimes are retained when their definition and path
    /// are unchanged, so an unrelated Vault update cannot replace their locks
    /// or in-memory status.
    pub fn reconcile(
        &self,
        registry: &VaultRegistryStore,
        snapshot: &VaultRegistrySnapshot,
    ) -> bool {
        let mut state = self
            .state
            .write()
            .expect("Vault collection runtime poisoned");
        if snapshot.revision() <= state.registry_revision {
            return false;
        }
        let previous = std::mem::take(&mut state.vaults);
        let mut next = BTreeMap::new();
        let revision_publisher = CollectionRevisionPublisher {
            state: Arc::downgrade(&self.state),
            revisions: self.revisions.clone(),
        };

        for definition in snapshot.definitions() {
            let vault_id = definition.vault_id();
            let vault_path = registry.vault_path(&definition);
            let entry = if !definition.enabled() {
                VaultCollectionEntry::Disabled(Box::new(disabled_snapshot(&definition)))
            } else {
                match previous.get(&vault_id) {
                    Some(VaultCollectionEntry::Active(runtime))
                        if runtime.definition() == &definition
                            && runtime.vault_path() == vault_path.as_path() =>
                    {
                        VaultCollectionEntry::Active(runtime.clone())
                    }
                    previous_entry => {
                        // An in-place edit on a Vault that was already
                        // active (as opposed to a genuinely new Vault, or
                        // one transitioning from disabled to enabled) must
                        // not force its Git status back to `Pending`: that
                        // would make the active loop below request an
                        // immediate real Git turn regardless of whatever
                        // status (e.g. `Unavailable` mid-backoff from a real
                        // transient failure) the retiring control block
                        // actually had. See `activation_snapshot`'s doc
                        // comment.
                        let prior_git =
                            if let Some(VaultCollectionEntry::Active(runtime)) = previous_entry {
                                let prior_snapshot = runtime.snapshot();
                                Some((prior_snapshot.git, prior_snapshot.git_error.clone()))
                            } else {
                                None
                            };
                        VaultCollectionEntry::Active(VaultControlBlock::activate(
                            definition,
                            vault_path,
                            self.watching.as_ref(),
                            self.snapshot_cache.as_deref(),
                            revision_publisher.clone(),
                            prior_git,
                        ))
                    }
                }
            };
            next.insert(vault_id, entry);
        }

        for (vault_id, entry) in &previous {
            let VaultCollectionEntry::Active(previous_runtime) = entry else {
                continue;
            };
            let retained = matches!(
                next.get(vault_id),
                Some(VaultCollectionEntry::Active(next_runtime))
                    if Arc::ptr_eq(&previous_runtime.snapshot, &next_runtime.snapshot)
            );
            if !retained {
                previous_runtime.revoke();
            }
        }

        let previous_snapshots = collection_snapshots(&previous);
        let next_snapshots = collection_snapshots(&next);
        let changed_vault_ids: Vec<VaultId> = {
            let mut ids = BTreeSet::new();
            for (vault_id, snapshot) in &next_snapshots {
                if previous_snapshots.get(vault_id) != Some(snapshot) {
                    ids.insert(*vault_id);
                }
            }
            for vault_id in previous_snapshots.keys() {
                if !next_snapshots.contains_key(vault_id) {
                    ids.insert(*vault_id);
                }
            }
            ids.into_iter().collect()
        };
        state.registry_revision = snapshot.revision();
        let event = if changed_vault_ids.is_empty() {
            None
        } else {
            state.collection_revision = state.collection_revision.saturating_add(1);
            Some(VaultCollectionRevisionEvent {
                collection_revision: state.collection_revision,
                vault_ids: changed_vault_ids,
                category: VaultChangeCategory::Definition,
            })
        };
        state.vaults = next;
        drop(state);
        if let Some(event) = event {
            self.revisions.send_replace(event);
        }
        true
    }

    /// Rebuild disposable background work from the authoritative collection and
    /// currently usable local Markdown. A future cache or Git lifecycle may
    /// refine the requested operation, but it must use this one coordinator.
    ///
    /// `managed_git` tracks scheduling (daily polling, backoff) only for
    /// managed-Git Vaults; it is activated/deactivated alongside the
    /// coordinator so a retired or disabled Vault's schedule cannot outlive
    /// its runtime.
    pub async fn reconcile_and_reconstruct(
        &self,
        registry: &VaultRegistryStore,
        snapshot: &VaultRegistrySnapshot,
        coordinator: &VaultWorkCoordinator,
        managed_git: &ManagedGitScheduler,
    ) {
        self.reconcile_and_reconstruct_with_mutation_boundary(
            registry,
            snapshot,
            coordinator,
            managed_git,
            None,
        )
        .await;
    }

    pub(crate) async fn reconcile_and_reconstruct_and_wait_for_mutation_boundary(
        &self,
        registry: &VaultRegistryStore,
        snapshot: &VaultRegistrySnapshot,
        coordinator: &VaultWorkCoordinator,
        managed_git: &ManagedGitScheduler,
        mutation_boundary: tokio::sync::oneshot::Sender<Result<(), String>>,
    ) {
        self.reconcile_and_reconstruct_with_mutation_boundary(
            registry,
            snapshot,
            coordinator,
            managed_git,
            Some(mutation_boundary),
        )
        .await;
    }

    async fn reconcile_and_reconstruct_with_mutation_boundary(
        &self,
        registry: &VaultRegistryStore,
        snapshot: &VaultRegistrySnapshot,
        coordinator: &VaultWorkCoordinator,
        managed_git: &ManagedGitScheduler,
        mutation_boundary: Option<tokio::sync::oneshot::Sender<Result<(), String>>>,
    ) {
        let phase_guard = self.reconcile_phase_lock.lock().await;
        let previously_active = self.active_runtimes();
        let previous_collection = self.snapshot();
        let previously_present = previous_collection
            .vaults
            .keys()
            .copied()
            .collect::<BTreeSet<_>>();
        let previously_disabled = previous_collection
            .vaults
            .iter()
            .filter_map(|(id, snapshot)| (!snapshot.enabled).then_some(*id))
            .collect::<BTreeSet<_>>();
        let changed = self.reconcile(registry, snapshot);
        if !changed && !self.has_registry_revision(snapshot.revision()) {
            if let Some(mutation_boundary) = mutation_boundary {
                let _ = mutation_boundary.send(Ok(()));
            }
            return;
        }
        #[cfg(test)]
        let phase_hook = self
            .after_reconcile_before_drain_hook
            .lock()
            .expect("reconcile phase hook poisoned")
            .clone();
        #[cfg(test)]
        if let Some(phase_hook) = phase_hook {
            phase_hook();
        }
        let active = self.active_runtimes();
        let retired = previously_active
            .iter()
            .filter_map(|(vault_id, previous_runtime)| {
                (!active.get(vault_id).is_some_and(|next_runtime| {
                    Arc::ptr_eq(&previous_runtime.snapshot, &next_runtime.snapshot)
                }))
                .then_some((*vault_id, previous_runtime.clone()))
            })
            .collect::<Vec<_>>();

        let mut retirement_error = None;
        for (vault_id, _) in &retired {
            coordinator.drain_vault(*vault_id);
            // A Vault edited while it stays enabled is "retired" here only
            // because any definition change constructs a fresh
            // `VaultControlBlock` (a new `Arc`, so `reconcile()`'s ptr_eq
            // retention check fails) — it is not actually leaving the
            // collection. `managed_git.deactivate` genuinely stops tracking
            // a Vault: it drops the scheduler's schedule/backoff state and
            // releases the held checkout lease's OS-level lock, which is
            // correct for disable/disconnect/identity-change (source type
            // or remote identity actually changed, which the registry
            // requires disabling the Vault for first — so it would not be
            // simultaneously present in both `previously_active` and
            // `active` here) but wrong for a benign edit (interval, name,
            // exclude patterns, mode, credentials) to a Vault that remains
            // enabled with the same managed-Git identity: skipping it here
            // lets the active loop below's `managed_git.activate` see the
            // still-tracked entry and update it in place — issue #97's
            // reopening finding 2 requires this so an interval-only edit
            // does not also reset an in-progress backoff (or drop and
            // reacquire the checkout lease) as a side effect of the
            // control block being replaced.
            let still_active_managed_git = active.get(vault_id).is_some_and(|runtime| {
                runtime
                    .definition()
                    .source()
                    .managed_git_poll_interval()
                    .is_some()
            });
            if !still_active_managed_git {
                managed_git.deactivate(*vault_id);
            }
        }
        drop(phase_guard);
        for (_, runtime) in &retired {
            runtime.wait_for_mutation_safe_boundary().await;
        }
        for (vault_id, _) in &retired {
            coordinator.wait_for_vault_safe_boundary(*vault_id).await;
        }
        // Reacquire only after every await. This makes the post-wait revision
        // fence, cache retirement/convergence, and new-work admission one
        // short phase: a newer lifecycle cannot slip between the fence and
        // retirement, while no safe-boundary wait is ever lock-held.
        let _post_wait_phase_guard = self.reconcile_phase_lock.lock().await;
        // A newer registry revision may have superseded this lifecycle while
        // it waited for old work. Do not let stale retirement/quarantine
        // side effects undo that newer collection.
        if !self.has_registry_revision(snapshot.revision()) {
            if let Some(mutation_boundary) = mutation_boundary {
                let _ = mutation_boundary.send(Ok(()));
            }
            return;
        }
        #[cfg(test)]
        let retirement_hook = self
            .after_post_wait_before_retirement_hook
            .lock()
            .expect("post-wait hook poisoned")
            .clone();
        #[cfg(test)]
        if let Some(retirement_hook) = retirement_hook {
            retirement_hook();
        }
        // An admitted Index turn may publish after its control block is
        // revoked, so retire its cache rows only after its coordinator safe
        // boundary. This makes disable/disconnect the final writer.
        for (vault_id, _) in &retired {
            let result = match snapshot.definition(*vault_id) {
                Some(definition) if !definition.enabled() => self
                    .snapshot_cache
                    .as_deref()
                    .map(|cache| cache.disable_vault_snapshot(*vault_id)),
                None => self
                    .snapshot_cache
                    .as_deref()
                    .map(|cache| cache.disconnect_vault_snapshot(*vault_id)),
                Some(_) => None,
            };
            if let Some(Err(message)) = result {
                error!(%vault_id, %message, "failed to retire disposable Vault snapshot");
                retirement_error = Some(message);
            }
        }
        // Disabled entries have no active control block, but a later
        // disconnect must still remove their retained nonparticipating rows.
        let active_retired = retired
            .iter()
            .map(|(vault_id, _)| *vault_id)
            .collect::<BTreeSet<_>>();
        let currently_present = self
            .snapshot()
            .vaults
            .keys()
            .copied()
            .collect::<BTreeSet<_>>();
        for vault_id in previously_present.difference(&currently_present) {
            if active_retired.contains(vault_id) {
                continue;
            }
            if let Some(cache) = self.snapshot_cache.as_deref()
                && let Err(message) = cache.disconnect_vault_snapshot(*vault_id)
            {
                error!(%vault_id, %message, "failed to disconnect retained disposable Vault snapshot");
                retirement_error = Some(message);
            }
        }
        // A prior retirement failure can leave rows behind. Re-enabling must
        // explicitly remove their participation before activation can expose
        // the reconstructed control block or queue its fresh Index turn.
        for (vault_id, runtime) in &active {
            if !previously_disabled.contains(vault_id) {
                continue;
            }
            match self
                .snapshot_cache
                .as_deref()
                .map(|cache| cache.disable_vault_snapshot(*vault_id))
            {
                Some(Ok(())) => {
                    let _ = runtime.set_search_status(VaultSearchStatus::Unavailable, None);
                }
                Some(Err(message)) => {
                    retirement_error = Some(message);
                    let _ = runtime.set_search_status(VaultSearchStatus::Unavailable, None);
                }
                None => {}
            }
        }
        // Reconciliation is idempotent for the current registry revision:
        // retries and restart reconstruction converge retained disabled rows
        // and disconnected orphans even when `reconcile()` made no state edit.
        if let Some(cache) = self.snapshot_cache.as_deref() {
            for definition in snapshot
                .definitions()
                .filter(|definition| !definition.enabled())
            {
                if let Err(message) = cache.disable_vault_snapshot(definition.vault_id()) {
                    retirement_error = Some(message);
                }
            }
            match cache.snapshot_vault_ids() {
                Ok(ids) => {
                    for vault_id in ids {
                        if snapshot.definition(vault_id).is_none()
                            && let Err(message) = cache.disconnect_vault_snapshot(vault_id)
                        {
                            retirement_error = Some(message);
                        }
                    }
                }
                Err(message) => retirement_error = Some(message),
            }
        }
        if let Some(mutation_boundary) = mutation_boundary {
            let _ = mutation_boundary.send(retirement_error.map_or(Ok(()), Err));
        }
        if !self.has_registry_revision(snapshot.revision()) {
            return;
        }
        for (vault_id, runtime) in &active {
            coordinator.activate_vault(*vault_id);
            if previously_active
                .get(vault_id)
                .is_some_and(|previous_runtime| {
                    Arc::ptr_eq(&previous_runtime.snapshot, &runtime.snapshot)
                })
            {
                continue;
            }
            let snapshot = runtime.snapshot();
            if snapshot.activation == VaultActivationStatus::Active
                && matches!(
                    snapshot.local_content,
                    LocalContentStatus::ReadWrite | LocalContentStatus::ReadOnly
                )
            {
                coordinator.request(*vault_id, VaultWorkKind::Index);
            }
            // Register/refresh the scheduler's per-Vault interval for every
            // newly (re)activated managed-Git definition, independent of
            // Git status: an edit that only changes `poll_interval_secs`
            // still produces a non-retained control block here (its
            // `VaultDefinition` compares unequal), but must not itself
            // request a Git turn — only a `Pending` status does that, below.
            if let Some(poll_interval) = runtime.definition().source().managed_git_poll_interval() {
                managed_git.activate(*vault_id, poll_interval);
            }
            if snapshot.git == VaultGitStatus::Pending {
                coordinator.request(*vault_id, VaultWorkKind::Git);
            }
        }
    }

    /// Revoke every Vault runtime, discard queued background work, and wait
    /// only for already-active background turns and foreground mutations to
    /// reach their safe boundaries.
    pub async fn shutdown(&self, coordinator: &VaultWorkCoordinator) {
        let runtimes = self
            .state
            .read()
            .expect("Vault collection runtime poisoned")
            .vaults
            .values()
            .filter_map(|entry| match entry {
                VaultCollectionEntry::Active(runtime) => Some(runtime.clone()),
                VaultCollectionEntry::Disabled(_) => None,
            })
            .collect::<Vec<_>>();
        for runtime in &runtimes {
            runtime.revoke();
        }
        coordinator.shutdown();
        for runtime in &runtimes {
            runtime.wait_for_mutation_safe_boundary().await;
        }
        coordinator.wait_for_shutdown_boundary().await;
    }

    pub fn runtime(&self, vault_id: VaultId) -> Option<VaultControlBlock> {
        let state = self
            .state
            .read()
            .expect("Vault collection runtime poisoned");
        match state.vaults.get(&vault_id) {
            Some(VaultCollectionEntry::Active(runtime)) => Some(runtime.clone()),
            Some(VaultCollectionEntry::Disabled(_)) | None => None,
        }
    }

    /// See `VaultControlBlock::notify_definition_changed`. No-op if
    /// `vault_id` is not currently an active Vault in the collection.
    pub fn notify_definition_changed(&self, vault_id: VaultId) {
        if let Some(runtime) = self.runtime(vault_id) {
            runtime.notify_definition_changed();
        }
    }

    pub fn active_vault_ids(&self) -> Vec<VaultId> {
        self.state
            .read()
            .expect("Vault collection runtime poisoned")
            .vaults
            .iter()
            .filter_map(|(vault_id, entry)| {
                matches!(entry, VaultCollectionEntry::Active(_)).then_some(*vault_id)
            })
            .collect()
    }

    fn active_runtimes(&self) -> BTreeMap<VaultId, VaultControlBlock> {
        self.state
            .read()
            .expect("Vault collection runtime poisoned")
            .vaults
            .iter()
            .filter_map(|(vault_id, entry)| match entry {
                VaultCollectionEntry::Active(runtime) => Some((*vault_id, runtime.clone())),
                VaultCollectionEntry::Disabled(_) => None,
            })
            .collect()
    }

    fn has_registry_revision(&self, registry_revision: u64) -> bool {
        self.state
            .read()
            .expect("Vault collection runtime poisoned")
            .registry_revision
            == registry_revision
    }

    pub fn subscribe_changes(&self) -> Option<tokio::sync::broadcast::Receiver<VaultId>> {
        self.watching
            .as_ref()
            .map(|watching| watching.changes.subscribe())
    }

    pub fn subscribe_revisions(
        &self,
    ) -> tokio::sync::watch::Receiver<VaultCollectionRevisionEvent> {
        self.revisions.subscribe()
    }

    pub fn snapshot(&self) -> VaultCollectionSnapshot {
        let state = self
            .state
            .read()
            .expect("Vault collection runtime poisoned");
        VaultCollectionSnapshot {
            registry_revision: state.registry_revision,
            collection_revision: state.collection_revision,
            vaults: collection_snapshots(&state.vaults),
        }
    }
}

impl Default for VaultCollectionRuntime {
    fn default() -> Self {
        Self::new()
    }
}

fn collection_snapshots(
    vaults: &BTreeMap<VaultId, VaultCollectionEntry>,
) -> BTreeMap<VaultId, CollectionVaultSnapshot> {
    vaults
        .iter()
        .map(|(vault_id, entry)| (*vault_id, entry.snapshot()))
        .collect()
}

/// `prior_git` carries a retiring control block's *actual* current Git
/// status (and its paired error, if any) into a freshly constructed
/// replacement for the *same* Vault — an in-place edit
/// (`reconcile()`'s non-retained-definition branch when `previous` held an
/// `Active` entry for this Vault) rather than a genuinely new activation.
/// Without this, every edit — not just a credential or interval change, any
/// field — would force `git` back to `Pending`, which the active loop below
/// treats as "needs an immediate first sync" and requests a real Git turn
/// for, bypassing whatever backoff a real transient failure had armed
/// (issue #97's reopening finding 1 exists specifically to prevent that
/// kind of forced immediate retry). `None` means "no prior control block to
/// carry over" — a genuinely new Vault or one transitioning from disabled
/// to enabled — where `Pending` (an immediate first sync) is correct.
fn activation_snapshot(
    definition: &VaultDefinition,
    vault_path: &Path,
    snapshot_cache: Option<&SqliteCache>,
    prior_git: Option<(VaultGitStatus, Option<VaultRuntimeError>)>,
) -> CollectionVaultSnapshot {
    let (local_content, activation_error) = stat_local_content(vault_path);
    let activation = if local_content == LocalContentStatus::Unavailable {
        VaultActivationStatus::Unavailable
    } else {
        VaultActivationStatus::Active
    };
    let (git, git_error) = match prior_git {
        Some((status, error)) => (status, error),
        None if matches!(definition.source(), RegistryVaultSource::Local { .. }) => {
            (VaultGitStatus::Disabled, None)
        }
        None => (VaultGitStatus::Pending, None),
    };
    let mut snapshot = CollectionVaultSnapshot {
        vault_id: definition.vault_id(),
        name: definition.name().to_string(),
        enabled: true,
        activation,
        local_content,
        search: retained_snapshot_search_status(snapshot_cache, definition.vault_id()),
        git,
        watcher: VaultWatcherStatus::Disabled,
        capabilities: VaultCapabilities::default(),
        activation_error,
        search_error: None,
        git_error,
        watcher_error: None,
    };
    snapshot.capabilities = collection_capabilities(definition, &snapshot);
    snapshot
}

/// A retained participating snapshot is immediately searchable after process
/// reconstruction, even while the coordinator has queued a fresh Index turn.
/// Cache read failure and nonparticipation remain conservatively unavailable.
fn retained_snapshot_search_status(
    snapshot_cache: Option<&SqliteCache>,
    vault_id: VaultId,
) -> VaultSearchStatus {
    match snapshot_cache.and_then(|cache| cache.snapshot_status(vault_id).ok().flatten()) {
        Some(status) if status.participating => match status.freshness {
            VaultSnapshotFreshness::Fresh => VaultSearchStatus::Ready,
            VaultSnapshotFreshness::Stale => VaultSearchStatus::Stale,
        },
        Some(_) | None => VaultSearchStatus::Unavailable,
    }
}

/// Stat `vault_path` and derive its current local-content availability. The
/// single source of truth for that derivation: `activation_snapshot` uses it
/// at `reconcile()` time, and `publish_local_content_after_git_success` uses
/// it again after a managed-Git checkout materializes, so the two call sites
/// can never drift on what "unavailable" means for a Vault path.
fn stat_local_content(vault_path: &Path) -> (LocalContentStatus, Option<VaultRuntimeError>) {
    match std::fs::metadata(vault_path) {
        Ok(metadata) if metadata.is_dir() => {
            match directory_content_status(vault_path, &metadata) {
                Ok(local_content) => (local_content, None),
                Err(error) => (LocalContentStatus::Unavailable, Some(error)),
            }
        }
        Ok(_) => (
            LocalContentStatus::Unavailable,
            Some(VaultRuntimeError {
                code: "vault_path_not_directory".to_string(),
                message: format!("Vault path '{}' is not a directory", vault_path.display()),
                retryable: false,
            }),
        ),
        Err(error) => (
            LocalContentStatus::Unavailable,
            Some(VaultRuntimeError {
                code: "vault_path_unavailable".to_string(),
                message: format!(
                    "Vault path '{}' is unavailable: {error}",
                    vault_path.display()
                ),
                retryable: true,
            }),
        ),
    }
}

fn directory_content_status(
    vault_path: &Path,
    metadata: &std::fs::Metadata,
) -> Result<LocalContentStatus, VaultRuntimeError> {
    std::fs::read_dir(vault_path).map_err(|error| VaultRuntimeError {
        code: "vault_path_unreadable".to_string(),
        message: format!(
            "Vault directory '{}' is not readable: {error}",
            vault_path.display()
        ),
        retryable: true,
    })?;
    if directory_is_writable(vault_path, metadata)? {
        Ok(LocalContentStatus::ReadWrite)
    } else {
        Ok(LocalContentStatus::ReadOnly)
    }
}

#[cfg(unix)]
fn directory_is_writable(
    vault_path: &Path,
    _metadata: &std::fs::Metadata,
) -> Result<bool, VaultRuntimeError> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let path = CString::new(vault_path.as_os_str().as_bytes()).map_err(|_| VaultRuntimeError {
        code: "vault_path_unavailable".to_string(),
        message: format!("Vault path '{}' contains a null byte", vault_path.display()),
        retryable: false,
    })?;
    // SAFETY: `path` is a live, null-terminated C string and `faccessat` does
    // not retain the pointer. AT_EACCESS checks the server's effective identity.
    let result =
        unsafe { libc::faccessat(libc::AT_FDCWD, path.as_ptr(), libc::W_OK, libc::AT_EACCESS) };
    if result == 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::PermissionDenied {
        Ok(false)
    } else {
        Err(VaultRuntimeError {
            code: "vault_path_unavailable".to_string(),
            message: format!(
                "Vault path '{}' availability check failed: {error}",
                vault_path.display()
            ),
            retryable: true,
        })
    }
}

#[cfg(not(unix))]
fn directory_is_writable(
    _vault_path: &Path,
    metadata: &std::fs::Metadata,
) -> Result<bool, VaultRuntimeError> {
    Ok(!metadata.permissions().readonly())
}

fn disabled_snapshot(definition: &VaultDefinition) -> CollectionVaultSnapshot {
    CollectionVaultSnapshot {
        vault_id: definition.vault_id(),
        name: definition.name().to_string(),
        enabled: false,
        activation: VaultActivationStatus::Disabled,
        local_content: LocalContentStatus::Unavailable,
        search: VaultSearchStatus::Unavailable,
        git: VaultGitStatus::Disabled,
        watcher: VaultWatcherStatus::Disabled,
        capabilities: VaultCapabilities::default(),
        activation_error: None,
        search_error: None,
        git_error: None,
        watcher_error: None,
    }
}

fn collection_capabilities(
    definition: &VaultDefinition,
    snapshot: &CollectionVaultSnapshot,
) -> VaultCapabilities {
    let browse = matches!(
        snapshot.local_content,
        LocalContentStatus::ReadWrite | LocalContentStatus::ReadOnly
    );
    let git_mode = match definition.source() {
        RegistryVaultSource::Local { .. } => None,
        RegistryVaultSource::ExistingGit { mode, .. }
        | RegistryVaultSource::ManagedGit { mode, .. } => Some(*mode),
    };
    let pull_only = git_mode == Some(VaultGitMode::PullOnly);
    VaultCapabilities {
        browse,
        search: matches!(
            snapshot.search,
            VaultSearchStatus::Ready | VaultSearchStatus::Stale
        ),
        mutate: snapshot.local_content == LocalContentStatus::ReadWrite && !pull_only,
        pull: snapshot.git == VaultGitStatus::Ready
            && matches!(
                git_mode,
                Some(VaultGitMode::PullOnly | VaultGitMode::TwoWay)
            ),
        push: snapshot.git == VaultGitStatus::Ready && git_mode == Some(VaultGitMode::TwoWay),
        retry: [
            snapshot.activation_error.as_ref(),
            snapshot.search_error.as_ref(),
            snapshot.git_error.as_ref(),
            snapshot.watcher_error.as_ref(),
        ]
        .into_iter()
        .flatten()
        .any(|error| error.retryable),
    }
}

/// Execute one `VaultWorkKind::Index` turn for exactly one active Vault.
///
/// The authoritative Markdown scan and disposable candidate-cache build run
/// off the async runtime. Publication replaces only this Vault's rows in the
/// shared read model, so readers either retain its prior complete snapshot or
/// observe the new complete snapshot. A failed scan or candidate build keeps a
/// prior snapshot available but marks it stale; without a prior snapshot the
/// Vault remains unavailable for search.
pub(crate) async fn dispatch_vault_index_turn(
    collection: &VaultCollectionRuntime,
    cache: Arc<SqliteCache>,
    embedder: Arc<dyn Embedder>,
    request: VaultWorkRequest,
) -> Result<(), VaultWorkError> {
    let vault_id = request.vault_id();
    let Some(control_block) = collection.runtime(vault_id) else {
        return Ok(());
    };

    control_block
        .set_search_status(VaultSearchStatus::Indexing, None)
        .map_err(vault_index_error)?;
    let (result, stale_mark_required) = {
        let _refresh = control_block
            .acquire_refresh()
            .await
            .map_err(vault_index_error)?;
        let indexing_control = control_block.clone();
        let indexing_cache = cache.clone();
        match tokio::task::spawn_blocking(move || {
            let index = indexing_control
                .authoritative_index()
                .map_err(|error| (vault_index_error(error), true))?;
            indexing_cache
                .replace_vault_snapshot(vault_id, &index, embedder.as_ref())
                .map_err(|message| {
                    (
                        VaultWorkError::new("vault_index_failed", message, true),
                        false,
                    )
                })
        })
        .await
        {
            Ok(Ok(())) => (Ok(()), false),
            Ok(Err((error, stale_mark_required))) => (Err(error), stale_mark_required),
            Err(error) => (
                Err(VaultWorkError::new(
                    "vault_index_task_panicked",
                    error.to_string(),
                    false,
                )),
                true,
            ),
        }
    };

    match &result {
        Ok(()) => {
            let _ = control_block.set_search_status(VaultSearchStatus::Ready, None);
        }
        Err(error) => {
            let stale_mark_error = stale_mark_required
                .then(|| cache.mark_vault_snapshot_stale(vault_id))
                .transpose()
                .err();
            let status = match cache.snapshot_status(vault_id) {
                Ok(Some(snapshot)) if snapshot.participating => VaultSearchStatus::Stale,
                Ok(Some(_)) | Ok(None) | Err(_) => VaultSearchStatus::Unavailable,
            };
            let message = match stale_mark_error {
                Some(mark_error) => format!(
                    "{} (also could not mark the retained snapshot stale: {mark_error})",
                    error.message()
                ),
                None => error.message().to_string(),
            };
            let _ = control_block.set_search_status(
                status,
                Some(VaultRuntimeError {
                    code: error.code().to_string(),
                    message,
                    retryable: error.retryable(),
                }),
            );
        }
    }
    result
}

fn vault_index_error(error: VaultRuntimeError) -> VaultWorkError {
    VaultWorkError::new("vault_index_failed", error.message, error.retryable)
}

/// Convert a [`VaultRuntimeError`] from [`VaultControlBlock::acquire_mutation`]
/// into the [`VaultWorkError`] a Git turn's dispatch returns. Distinct from
/// [`vault_index_error`] (Index-turn errors use `"vault_index_failed"`) so a
/// failure to acquire the mutation lock ahead of a Git turn is never
/// misreported as an indexing failure.
fn managed_git_mutation_error(error: VaultRuntimeError) -> VaultWorkError {
    VaultWorkError::new(
        "managed_git_mutation_unavailable",
        error.message,
        error.retryable,
    )
}

/// Execute one `VaultWorkKind::Git` turn for `request` and publish its
/// result: Git status always, and — since `activation_snapshot` only stats
/// `vault_path` once, at `reconcile()` time, before any managed checkout
/// exists — authoritative local-content availability whenever a turn
/// completes successfully. A Git failure never touches local-content status,
/// so a Vault that already has a usable checkout stays browsable through a
/// later sync failure.
///
/// A no-op returning `Ok(())` if the Vault has since been retired (its
/// runtime is gone) or is not managed-Git (defensive: the coordinator only
/// receives `Git` requests for managed-Git Vaults, but this seam does not
/// assume that holds forever).
pub async fn dispatch_managed_git_turn(
    collection: &VaultCollectionRuntime,
    registry: &VaultRegistryStore,
    coordinator: &VaultWorkCoordinator,
    managed_git: &ManagedGitScheduler,
    author_name: &str,
    author_email: &str,
    request: VaultWorkRequest,
) -> Result<(), VaultWorkError> {
    dispatch_managed_git_turn_with(
        collection,
        registry,
        coordinator,
        managed_git,
        author_name,
        author_email,
        request,
        run_managed_git_turn,
    )
    .await
}

/// [`dispatch_managed_git_turn`] with the actual `git2` turn injectable,
/// mirroring `git/task.rs`'s `SyncOps` dependency-injection pattern: `execute`
/// is production's `run_managed_git_turn` in the real dispatch loop, and a
/// deterministic fake in tests that need to drive a real failure through the
/// full async path (credential resolution, `spawn_blocking`, status
/// publishing, scheduler recording) without a reachable remote.
#[allow(clippy::too_many_arguments)] // Production arguments plus the test-only executor.
async fn dispatch_managed_git_turn_with<F>(
    collection: &VaultCollectionRuntime,
    registry: &VaultRegistryStore,
    coordinator: &VaultWorkCoordinator,
    managed_git: &ManagedGitScheduler,
    author_name: &str,
    author_email: &str,
    request: VaultWorkRequest,
    execute: F,
) -> Result<(), VaultWorkError>
where
    F: FnOnce(
            &ManagedGitTurnConfig,
            &ManagedCheckoutLease,
        ) -> Result<crate::git::ManagedGitOutcome, VaultWorkError>
        + Send
        + 'static,
{
    let vault_id = request.vault_id();
    let Some(control_block) = collection.runtime(vault_id) else {
        managed_git.deactivate(vault_id);
        return Ok(());
    };
    let (repository_url, branch, vault_subdirectory, mode) =
        match control_block.definition().source() {
            RegistryVaultSource::ManagedGit {
                repository_url,
                branch,
                vault_subdirectory,
                mode,
                poll_interval_secs: _,
            } => (
                repository_url.clone(),
                branch.clone(),
                vault_subdirectory.clone(),
                *mode,
            ),
            // An existing checkout under Local-history versioning has no
            // remote to sync: flush whatever Vault-subtree drift has
            // accumulated into a local commit, off the async runtime, then
            // publish through the exact same status/scheduler path a
            // managed-Git turn uses. `run_local_history_git_turn` resolves
            // its own placeholder `GitConfig` from `control_block.vault_path()`
            // alone, so nothing else needs to be read off `source()` here.
            RegistryVaultSource::ExistingGit {
                mode: VaultGitMode::LocalHistory,
                ..
            } => {
                let vault_path = control_block.vault_path().to_path_buf();
                let author_name = author_name.to_string();
                let author_email = author_email.to_string();
                let result = tokio::task::spawn_blocking(move || {
                    crate::git::run_local_history_git_turn(vault_path, author_name, author_email)
                })
                .await
                .unwrap_or_else(|join_error| {
                    Err(VaultWorkError::new(
                        "existing_git_local_history_task_panicked",
                        join_error.to_string(),
                        false,
                    ))
                });
                publish_managed_git_turn_outcome(
                    &control_block,
                    coordinator,
                    managed_git,
                    vault_id,
                    &result,
                );
                return result.map(|_: crate::git::ManagedGitOutcome| ());
            }
            // An existing checkout under Pull-only or Two-way versioning is
            // remote sync against the checkout that already exists at
            // `repository_path` — no managed-checkout acquisition or lease:
            // see `run_existing_git_remote_turn`'s doc comment for why
            // `ManagedCheckoutLease` does not apply to an `ExistingGit`
            // source. Holds the same per-Vault mutation lock a managed-Git
            // turn holds below (defect 2 of issue #96's reopening): without
            // it, a foreground Markdown write could race this turn's
            // fetch/integrate/reset phases.
            RegistryVaultSource::ExistingGit {
                mode: existing_mode @ (VaultGitMode::PullOnly | VaultGitMode::TwoWay),
                repository_path,
                branch,
                ..
            } => {
                let repository_path = repository_path.clone();
                let vault_path = control_block.vault_path().to_path_buf();
                let branch = branch.clone();
                let mode = *existing_mode;
                let credentials = match registry.https_credentials(vault_id) {
                    Ok(credentials) => credentials,
                    Err(error) => {
                        let result = Err(VaultWorkError::new(
                            "managed_git_registry_unavailable",
                            error.to_string(),
                            true,
                        ));
                        publish_managed_git_turn_outcome(
                            &control_block,
                            coordinator,
                            managed_git,
                            vault_id,
                            &result,
                        );
                        return result.map(|_: crate::git::ManagedGitOutcome| ());
                    }
                };
                let mutation_guard = match control_block.acquire_mutation().await {
                    Ok(guard) => guard,
                    Err(error) => {
                        let result = Err(managed_git_mutation_error(error));
                        publish_managed_git_turn_outcome(
                            &control_block,
                            coordinator,
                            managed_git,
                            vault_id,
                            &result,
                        );
                        return result.map(|_: crate::git::ManagedGitOutcome| ());
                    }
                };
                let author_name = author_name.to_string();
                let author_email = author_email.to_string();
                let result = tokio::task::spawn_blocking(move || {
                    run_existing_git_remote_turn(
                        repository_path,
                        vault_path,
                        branch,
                        mode,
                        credentials,
                        author_name,
                        author_email,
                    )
                })
                .await
                .unwrap_or_else(|join_error| {
                    Err(VaultWorkError::new(
                        "existing_git_remote_task_panicked",
                        join_error.to_string(),
                        false,
                    ))
                });
                drop(mutation_guard);
                publish_managed_git_turn_outcome(
                    &control_block,
                    coordinator,
                    managed_git,
                    vault_id,
                    &result,
                );
                return result.map(|_: crate::git::ManagedGitOutcome| ());
            }
            // `Local` has no Git turn at all.
            RegistryVaultSource::Local { .. } => {
                return Ok(());
            }
        };
    let credentials = match registry.https_credentials(vault_id) {
        Ok(credentials) => credentials,
        Err(error) => {
            let result = Err(VaultWorkError::new(
                "managed_git_registry_unavailable",
                error.to_string(),
                true,
            ));
            publish_managed_git_turn_outcome(
                &control_block,
                coordinator,
                managed_git,
                vault_id,
                &result,
            );
            return result.map(|_: crate::git::ManagedGitOutcome| ());
        }
    };
    let state_directory = registry
        .path()
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);

    let config = ManagedGitTurnConfig {
        vault_id,
        state_directory,
        repository_url,
        branch,
        vault_subdirectory,
        mode,
        credentials,
        author_name: author_name.to_string(),
        author_email: author_email.to_string(),
    };

    // Obtain this Vault's checkout lease — reused from a previous turn if
    // `ManagedGitScheduler` is already holding one, or freshly acquired
    // otherwise (only the first turn since activation pays that one-time,
    // local-filesystem-only cost; see
    // `ManagedGitScheduler::take_or_acquire_checkout_lease`). Extracted
    // *before* `spawn_blocking` — an owned `ManagedCheckoutLease` has no
    // lifetime tied to `managed_git`, so it (and `config`) can move into
    // the blocking closure below without borrowing `managed_git` there,
    // which `spawn_blocking`'s `'static` bound would otherwise forbid.
    let lease = match managed_git
        .take_or_acquire_checkout_lease(config.state_directory.clone(), vault_id)
    {
        Ok(lease) => lease,
        Err(error) => {
            let result = Err(crate::git::managed_task::classify_checkout_error(error));
            publish_managed_git_turn_outcome(
                &control_block,
                coordinator,
                managed_git,
                vault_id,
                &result,
            );
            return result.map(|_: crate::git::ManagedGitOutcome| ());
        }
    };

    // Hold the same per-Vault mutation lock a foreground Markdown write
    // acquires (`handlers::vault_write`/`mcp::tools::write`'s own
    // `acquire_mutation`) across this turn's blocking `git2` work (issue
    // #96's reopening defect 2): without it, a write could land mid-merge,
    // or this turn's checkout/reset could stomp a write mid-flight.
    // Acquired *after* the checkout lease so a lease-acquisition failure
    // above never blocks on it; the two locks are always acquired in this
    // same order for the same Vault, and nothing else in this codebase ever
    // acquires the checkout lease, so there is no risk of the mutation lock
    // and the checkout lease being acquired in opposite orders elsewhere.
    // Coarser than the legacy single-Vault path's fine-grained per-phase
    // locking (`git/task.rs::run_sync_phases` releases its lock across the
    // network-only fetch/push phases) — held for this whole turn instead,
    // including `synchronize_managed_checkout`'s network round-trip.
    // Reproducing the fine-grained scheme here would require splitting
    // `synchronize_managed_checkout`'s monolithic fetch+integrate+push call
    // into phases callable independently from this async dispatch layer, a
    // substantially larger change than this fix warrants on its own.
    let mutation_guard = match control_block.acquire_mutation().await {
        Ok(guard) => guard,
        Err(error) => {
            let result = Err(managed_git_mutation_error(error));
            publish_managed_git_turn_outcome(
                &control_block,
                coordinator,
                managed_git,
                vault_id,
                &result,
            );
            return result.map(|_: crate::git::ManagedGitOutcome| ());
        }
    };

    // The lease travels into the blocking task and back out again — it is
    // never dropped here, only borrowed by `execute` — so the scheduler can
    // hand it back to `keep_checkout_lease` afterward and keep holding it
    // across turns instead of releasing its OS-level lock at the end of
    // this one (issue #95).
    let outcome = tokio::task::spawn_blocking(move || {
        let result = execute(&config, &lease);
        (result, lease)
    })
    .await;
    drop(mutation_guard);
    let (result, lease) = match outcome {
        Ok((result, lease)) => (result, Some(lease)),
        Err(join_error) => (
            Err(VaultWorkError::new(
                "managed_git_task_panicked",
                join_error.to_string(),
                false,
            )),
            // The panicking task owned `lease`; it was dropped (releasing
            // the OS lock) during unwinding, so there is nothing to keep.
            None,
        ),
    };
    if let Some(lease) = lease {
        managed_git.keep_checkout_lease(vault_id, lease);
    }

    publish_managed_git_turn_outcome(&control_block, coordinator, managed_git, vault_id, &result);
    result.map(|_| ())
}

/// Publish one Git turn's result: Git status always, and — since
/// `activation_snapshot` only stats `vault_path` once, at `reconcile()`
/// time, before any managed checkout exists — authoritative local-content
/// availability whenever a turn completes successfully. A Git failure never
/// touches local-content status, so a Vault that already has a usable
/// checkout stays browsable through a later sync failure. Also feeds the
/// outcome back to the scheduler so it can arm the next attempt.
///
/// Separated from [`dispatch_managed_git_turn`] so this — the interesting
/// behavior — is testable against a fabricated result, without needing a
/// real `git2` clone/fetch against a reachable remote.
fn publish_managed_git_turn_outcome(
    control_block: &VaultControlBlock,
    coordinator: &VaultWorkCoordinator,
    managed_git: &ManagedGitScheduler,
    vault_id: VaultId,
    result: &Result<crate::git::ManagedGitOutcome, VaultWorkError>,
) {
    match result {
        Ok(_) => {
            let _ = control_block.set_git_status(VaultGitStatus::Ready, None);
            publish_local_content_after_git_success(control_block);
            if control_block.is_accepting_operations()
                && matches!(
                    control_block.snapshot().local_content,
                    LocalContentStatus::ReadWrite | LocalContentStatus::ReadOnly
                )
            {
                coordinator.request(vault_id, VaultWorkKind::Index);
            }
        }
        Err(error) => {
            let _ = control_block.set_git_status(
                VaultGitStatus::Unavailable,
                Some(VaultRuntimeError {
                    code: error.code().to_string(),
                    message: error.message().to_string(),
                    retryable: error.retryable(),
                }),
            );
        }
    }
    managed_git.record_outcome(vault_id, result);
}

/// Re-derive and publish local-content availability after a successful Git
/// turn, using the same directory check `activation_snapshot` uses at
/// `reconcile()` time (via [`stat_local_content`]). A managed Vault's
/// checkout may not have existed the last time that ran.
fn publish_local_content_after_git_success(control_block: &VaultControlBlock) {
    let (status, error) = stat_local_content(control_block.vault_path());
    let _ = control_block.set_local_content_status(status, error);
}

#[cfg(test)]
mod tests;
