use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock, Weak};

use serde::Serialize;

use crate::startup::IndexingProgressSnapshot;
use crate::vault_registry::{
    VaultDefinition, VaultGitMode, VaultId, VaultRegistrySnapshot, VaultRegistryStore,
    VaultSource as RegistryVaultSource,
};
use crate::vault_watcher::{VaultWatcherHandle, spawn_vault_change_watcher};

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
        revisions: CollectionRevisionPublisher,
    ) -> Self {
        let mut snapshot = activation_snapshot(&definition, &vault_path);
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

    fn ensure_accepting_operations(&self) -> Result<(), VaultRuntimeError> {
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
            self.revisions.bump();
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
            self.revisions.bump();
        }
        Ok(())
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

#[derive(Clone)]
pub struct VaultCollectionRuntime {
    state: Arc<RwLock<VaultCollectionState>>,
    revisions: tokio::sync::watch::Sender<u64>,
    watching: Option<WatcherContext>,
}

#[derive(Clone)]
struct CollectionRevisionPublisher {
    state: Weak<RwLock<VaultCollectionState>>,
    revisions: tokio::sync::watch::Sender<u64>,
}

impl CollectionRevisionPublisher {
    fn bump(&self) {
        let Some(state) = self.state.upgrade() else {
            return;
        };
        let revision = {
            let mut state = state.write().expect("Vault collection runtime poisoned");
            state.collection_revision = state.collection_revision.saturating_add(1);
            state.collection_revision
        };
        self.revisions.send_replace(revision);
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
    pub fn new() -> Self {
        let (revisions, _) = tokio::sync::watch::channel(0);
        Self {
            state: Arc::new(RwLock::new(VaultCollectionState {
                registry_revision: 0,
                collection_revision: 0,
                vaults: BTreeMap::new(),
            })),
            revisions,
            watching: None,
        }
    }

    pub fn with_watching(cache_db_path: PathBuf) -> Self {
        let (changes, _) = tokio::sync::broadcast::channel(64);
        let (revisions, _) = tokio::sync::watch::channel(0);
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
        }
    }

    /// Reconcile live control blocks to one authoritative registry snapshot.
    /// Existing enabled runtimes are retained when their definition and path
    /// are unchanged, so an unrelated Vault update cannot replace their locks
    /// or in-memory status.
    pub fn reconcile(&self, registry: &VaultRegistryStore, snapshot: &VaultRegistrySnapshot) {
        let mut state = self
            .state
            .write()
            .expect("Vault collection runtime poisoned");
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
                    _ => VaultCollectionEntry::Active(VaultControlBlock::activate(
                        definition,
                        vault_path,
                        self.watching.as_ref(),
                        revision_publisher.clone(),
                    )),
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

        let changed = collection_snapshots(&previous) != collection_snapshots(&next);
        state.registry_revision = snapshot.revision();
        let collection_revision = if changed {
            state.collection_revision = state.collection_revision.saturating_add(1);
            Some(state.collection_revision)
        } else {
            None
        };
        state.vaults = next;
        drop(state);
        if let Some(collection_revision) = collection_revision {
            self.revisions.send_replace(collection_revision);
        }
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

    pub fn subscribe_changes(&self) -> Option<tokio::sync::broadcast::Receiver<VaultId>> {
        self.watching
            .as_ref()
            .map(|watching| watching.changes.subscribe())
    }

    pub fn subscribe_revisions(&self) -> tokio::sync::watch::Receiver<u64> {
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

fn activation_snapshot(definition: &VaultDefinition, vault_path: &Path) -> CollectionVaultSnapshot {
    let (activation, local_content, activation_error) = match std::fs::metadata(vault_path) {
        Ok(metadata) if metadata.is_dir() => {
            match directory_content_status(vault_path, &metadata) {
                Ok(local_content) => (VaultActivationStatus::Active, local_content, None),
                Err(error) => (
                    VaultActivationStatus::Unavailable,
                    LocalContentStatus::Unavailable,
                    Some(error),
                ),
            }
        }
        Ok(_) => (
            VaultActivationStatus::Unavailable,
            LocalContentStatus::Unavailable,
            Some(VaultRuntimeError {
                code: "vault_path_not_directory".to_string(),
                message: format!("Vault path '{}' is not a directory", vault_path.display()),
                retryable: false,
            }),
        ),
        Err(error) => (
            VaultActivationStatus::Unavailable,
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
    };
    let git = if matches!(definition.source(), RegistryVaultSource::Local { .. }) {
        VaultGitStatus::Disabled
    } else {
        VaultGitStatus::Pending
    };
    let mut snapshot = CollectionVaultSnapshot {
        vault_id: definition.vault_id(),
        name: definition.name().to_string(),
        enabled: true,
        activation,
        local_content,
        search: VaultSearchStatus::Unavailable,
        git,
        watcher: VaultWatcherStatus::Disabled,
        capabilities: VaultCapabilities::default(),
        activation_error,
        search_error: None,
        git_error: None,
        watcher_error: None,
    };
    snapshot.capabilities = collection_capabilities(definition, &snapshot);
    snapshot
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    use crate::vault_registry::{
        NewVaultDefinition, VaultGitMode, VaultRegistrySnapshot, VaultRegistryStore,
        VaultSource as RegistryVaultSource,
    };

    fn managed(mode: ManagedGitMode) -> VaultSource {
        VaultSource::ManagedGit(ManagedGitSource {
            repository_url: "https://example.test/vault.git".to_string(),
            checkout_path: PathBuf::from("/data/vault"),
            branch: None,
            vault_subdirectory: None,
            mode,
        })
    }

    #[test]
    fn pull_only_ready_state_never_allows_mutation_or_push() {
        let runtime = VaultRuntime::ready(managed(ManagedGitMode::PullOnly));
        let capabilities = runtime.snapshot().capabilities;
        assert!(capabilities.browse);
        assert!(capabilities.search);
        assert!(capabilities.pull);
        assert!(!capabilities.mutate);
        assert!(!capabilities.push);
    }

    #[test]
    fn unavailable_state_has_no_ready_vault_capabilities() {
        let runtime = VaultRuntime::new(managed(ManagedGitMode::Bidirectional));
        runtime.set_unavailable("not_acquired", "Vault has not been acquired");
        let snapshot = runtime.snapshot();
        assert_eq!(snapshot.phase, VaultPhase::Unavailable);
        assert!(!snapshot.capabilities.browse);
        assert!(!snapshot.capabilities.search);
        assert!(!snapshot.capabilities.mutate);
        assert!(!snapshot.capabilities.retry);
    }

    #[test]
    fn local_history_ready_state_never_exposes_remote_capabilities() {
        let directory = tempdir().expect("temporary state directory");
        let repository_path = directory.path().join("repository");
        git2::Repository::init(&repository_path).expect("initialize repository");
        let vault_path = repository_path.join("notes");
        std::fs::create_dir(&vault_path).expect("create Vault subdirectory");
        let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
        let snapshot = registry
            .add(
                0,
                NewVaultDefinition {
                    name: "Local history".to_string(),
                    enabled: true,
                    source: RegistryVaultSource::ExistingGit {
                        repository_path,
                        repository_url: None,
                        branch: None,
                        vault_subdirectory: Some(PathBuf::from("notes")),
                        mode: VaultGitMode::LocalHistory,
                    },
                    exclude_patterns: Vec::new(),
                    https_credentials: None,
                },
            )
            .expect("add local-history Vault");
        let vault_id = vault_id_named(&snapshot, "Local history");
        let collection = VaultCollectionRuntime::new();
        collection.reconcile(&registry, &snapshot);
        let runtime = collection.runtime(vault_id).expect("active runtime");

        runtime
            .set_git_status(VaultGitStatus::Ready, None)
            .expect("publish ready Git status");

        let capabilities = runtime.snapshot().capabilities;
        assert!(capabilities.mutate);
        assert!(!capabilities.pull);
        assert!(!capabilities.push);
    }

    fn add_local_vault(
        registry: &VaultRegistryStore,
        snapshot: &VaultRegistrySnapshot,
        name: &str,
        path: PathBuf,
    ) -> VaultRegistrySnapshot {
        registry
            .add(
                snapshot.revision(),
                NewVaultDefinition {
                    name: name.to_string(),
                    enabled: true,
                    source: RegistryVaultSource::Local { path },
                    exclude_patterns: Vec::new(),
                    https_credentials: None,
                },
            )
            .expect("add local Vault")
    }

    fn vault_id_named(snapshot: &VaultRegistrySnapshot, name: &str) -> VaultId {
        snapshot
            .definitions()
            .find(|definition| definition.name() == name)
            .expect("named Vault definition")
            .vault_id()
    }

    #[test]
    fn activates_zero_one_and_many_enabled_vaults_from_registry_snapshots() {
        let directory = tempdir().expect("temporary state directory");
        let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
        let empty = match registry.load().expect("load empty registry") {
            crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
            crate::vault_registry::VaultRegistryState::Recovery(_) => {
                panic!("new registry entered recovery")
            }
        };
        let collection = VaultCollectionRuntime::new();
        collection.reconcile(&registry, &empty);
        assert!(collection.snapshot().vaults.is_empty());

        let first_path = directory.path().join("first");
        std::fs::create_dir_all(&first_path).expect("first Vault directory");
        let one = add_local_vault(&registry, &empty, "First", first_path);
        collection.reconcile(&registry, &one);
        assert_eq!(collection.snapshot().vaults.len(), 1);
        assert_eq!(collection.active_vault_ids().len(), 1);

        let second_path = directory.path().join("second");
        std::fs::create_dir_all(&second_path).expect("second Vault directory");
        let many = add_local_vault(&registry, &one, "Second", second_path);
        collection.reconcile(&registry, &many);
        assert_eq!(collection.snapshot().vaults.len(), 2);
        assert_eq!(collection.active_vault_ids().len(), 2);
        assert!(
            collection
                .snapshot()
                .vaults
                .values()
                .all(|vault| vault.capabilities.browse)
        );
    }

    #[test]
    fn activation_failure_is_isolated_from_healthy_local_markdown() {
        let directory = tempdir().expect("temporary state directory");
        let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
        let empty = match registry.load().expect("load empty registry") {
            crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
            crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
        };
        let healthy_path = directory.path().join("healthy");
        std::fs::create_dir_all(&healthy_path).expect("healthy Vault directory");
        let one = add_local_vault(&registry, &empty, "Healthy", healthy_path);
        let two = registry
            .add(
                one.revision(),
                NewVaultDefinition {
                    name: "Unavailable managed".to_string(),
                    enabled: true,
                    source: RegistryVaultSource::ManagedGit {
                        repository_url: "https://example.test/vault.git".to_string(),
                        branch: Some("main".to_string()),
                        vault_subdirectory: None,
                        mode: VaultGitMode::TwoWay,
                    },
                    exclude_patterns: Vec::new(),
                    https_credentials: None,
                },
            )
            .expect("add managed Vault before acquisition");

        let collection = VaultCollectionRuntime::new();
        collection.reconcile(&registry, &two);
        let snapshot = collection.snapshot();
        let healthy = &snapshot.vaults[&vault_id_named(&two, "Healthy")];
        let unavailable = &snapshot.vaults[&vault_id_named(&two, "Unavailable managed")];
        assert_eq!(healthy.activation, VaultActivationStatus::Active);
        assert!(healthy.capabilities.browse);
        assert_eq!(unavailable.activation, VaultActivationStatus::Unavailable);
        assert!(!unavailable.capabilities.browse);
        assert_eq!(
            unavailable
                .activation_error
                .as_ref()
                .map(|error| error.code.as_str()),
            Some("vault_path_unavailable")
        );
    }

    #[test]
    fn read_only_and_stale_statuses_keep_usable_local_markdown_honest() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempdir().expect("temporary state directory");
        let vault_path = directory.path().join("read-only");
        std::fs::create_dir_all(&vault_path).expect("Vault directory");
        let original_mode = std::fs::metadata(&vault_path)
            .expect("Vault metadata")
            .permissions()
            .mode();
        std::fs::set_permissions(&vault_path, std::fs::Permissions::from_mode(0o555))
            .expect("make Vault read-only");
        let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
        let empty = match registry.load().expect("load empty registry") {
            crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
            crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
        };
        let one = add_local_vault(&registry, &empty, "Read only", vault_path.clone());
        let vault_id = vault_id_named(&one, "Read only");
        let collection = VaultCollectionRuntime::new();
        collection.reconcile(&registry, &one);
        let runtime = collection.runtime(vault_id).expect("enabled runtime");

        let read_only = runtime.snapshot();
        assert_eq!(read_only.local_content, LocalContentStatus::ReadOnly);
        assert!(read_only.capabilities.browse);
        assert!(!read_only.capabilities.mutate);
        assert!(!read_only.capabilities.search);

        runtime
            .set_search_status(VaultSearchStatus::Stale, None)
            .expect("publish stale search status");
        let stale = runtime.snapshot();
        assert_eq!(stale.search, VaultSearchStatus::Stale);
        assert!(stale.capabilities.browse);
        assert!(stale.capabilities.search);
        assert!(!stale.capabilities.mutate);

        runtime
            .set_git_status(
                VaultGitStatus::Unavailable,
                Some(VaultRuntimeError {
                    code: "git_temporarily_unavailable".to_string(),
                    message: "Git is temporarily unavailable".to_string(),
                    retryable: true,
                }),
            )
            .expect("publish unavailable Git status");
        let git_degraded = runtime.snapshot();
        assert!(git_degraded.capabilities.browse);
        assert!(git_degraded.capabilities.search);
        assert!(!git_degraded.capabilities.mutate);
        assert!(git_degraded.capabilities.retry);
        assert_eq!(
            git_degraded
                .git_error
                .as_ref()
                .map(|error| error.code.as_str()),
            Some("git_temporarily_unavailable")
        );
        assert!(git_degraded.search_error.is_none());

        std::fs::set_permissions(&vault_path, std::fs::Permissions::from_mode(original_mode))
            .expect("restore Vault permissions");
    }

    #[tokio::test]
    async fn status_changes_advance_and_publish_collection_revisions() {
        let directory = tempdir().expect("temporary state directory");
        let vault_path = directory.path().join("vault");
        std::fs::create_dir_all(&vault_path).expect("Vault directory");
        let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
        let empty = match registry.load().expect("load empty registry") {
            crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
            crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
        };
        let one = add_local_vault(&registry, &empty, "Vault", vault_path);
        let vault_id = vault_id_named(&one, "Vault");
        let collection = VaultCollectionRuntime::new();
        collection.reconcile(&registry, &one);
        let mut revisions = collection.subscribe_revisions();
        let before = collection.snapshot().collection_revision;

        collection
            .runtime(vault_id)
            .expect("enabled runtime")
            .set_search_status(VaultSearchStatus::Ready, None)
            .expect("publish ready search status");

        revisions.changed().await.expect("revision event");
        let after = collection.snapshot().collection_revision;
        assert_eq!(after, before + 1);
        assert_eq!(*revisions.borrow_and_update(), after);
    }

    #[tokio::test]
    async fn disabled_runtime_rejects_operations_through_preexisting_handles() {
        let directory = tempdir().expect("temporary state directory");
        let vault_path = directory.path().join("vault");
        std::fs::create_dir_all(&vault_path).expect("Vault directory");
        let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
        let empty = match registry.load().expect("load empty registry") {
            crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
            crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
        };
        let one = add_local_vault(&registry, &empty, "Vault", vault_path);
        let vault_id = vault_id_named(&one, "Vault");
        let collection = VaultCollectionRuntime::new();
        collection.reconcile(&registry, &one);
        let runtime = collection.runtime(vault_id).expect("enabled runtime");
        let mut cancellation = runtime.subscribe_cancellation();

        let disabled = registry
            .disable(one.revision(), vault_id)
            .expect("disable Vault");
        collection.reconcile(&registry, &disabled);

        cancellation.changed().await.expect("cancellation event");
        assert!(*cancellation.borrow_and_update());
        assert!(*runtime.subscribe_cancellation().borrow());
        assert!(!runtime.is_accepting_operations());
        assert_eq!(
            runtime
                .acquire_mutation()
                .await
                .expect_err("disabled runtime must reject mutation")
                .code,
            "vault_runtime_not_active"
        );
        assert_eq!(
            runtime
                .set_search_status(VaultSearchStatus::Ready, None)
                .expect_err("disabled runtime must reject status changes")
                .code,
            "vault_runtime_not_active"
        );
    }

    #[test]
    fn unreadable_directory_is_not_reported_as_browseable() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempdir().expect("temporary state directory");
        let vault_path = directory.path().join("unreadable");
        std::fs::create_dir_all(&vault_path).expect("Vault directory");
        let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
        let empty = match registry.load().expect("load empty registry") {
            crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
            crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
        };
        let one = add_local_vault(&registry, &empty, "Unreadable", vault_path.clone());
        std::fs::set_permissions(&vault_path, std::fs::Permissions::from_mode(0o077))
            .expect("deny the owning process access after registration");
        let collection = VaultCollectionRuntime::new();
        collection.reconcile(&registry, &one);
        let status = &collection.snapshot().vaults[&vault_id_named(&one, "Unreadable")];

        assert_eq!(status.local_content, LocalContentStatus::Unavailable);
        assert!(!status.capabilities.browse);
        std::fs::set_permissions(&vault_path, std::fs::Permissions::from_mode(0o700))
            .expect("restore Vault permissions");
    }

    #[tokio::test]
    async fn disable_enable_and_disconnect_only_replace_the_target_runtime() {
        let directory = tempdir().expect("temporary state directory");
        let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
        let empty = match registry.load().expect("load empty registry") {
            crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
            crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
        };
        let first_path = directory.path().join("first");
        let second_path = directory.path().join("second");
        std::fs::create_dir_all(&first_path).expect("first Vault directory");
        std::fs::create_dir_all(&second_path).expect("second Vault directory");
        let one = add_local_vault(&registry, &empty, "First", first_path);
        let two = add_local_vault(&registry, &one, "Second", second_path);
        let first_id = vault_id_named(&two, "First");
        let second_id = vault_id_named(&two, "Second");
        let collection =
            VaultCollectionRuntime::with_watching(directory.path().join("cache.sqlite3"));
        collection.reconcile(&registry, &two);
        let first_runtime = collection.runtime(first_id).expect("first runtime");
        let first_lock = first_runtime.mutation_lock.clone();
        let second_lock = collection
            .runtime(second_id)
            .expect("second runtime")
            .mutation_lock
            .clone();
        assert!(!Arc::ptr_eq(&first_lock, &second_lock));
        assert_eq!(
            first_runtime.snapshot().watcher,
            VaultWatcherStatus::Running
        );

        let disabled = registry
            .disable(two.revision(), first_id)
            .expect("disable first Vault");
        collection.reconcile(&registry, &disabled);
        assert!(collection.runtime(first_id).is_none());
        assert!(first_runtime.watcher_cancelled());
        assert!(Arc::ptr_eq(
            &second_lock,
            &collection
                .runtime(second_id)
                .expect("second runtime retained")
                .mutation_lock
        ));
        let disabled_status = &collection.snapshot().vaults[&first_id];
        assert_eq!(disabled_status.activation, VaultActivationStatus::Disabled);
        assert_eq!(disabled_status.watcher, VaultWatcherStatus::Disabled);
        assert_eq!(disabled_status.capabilities, VaultCapabilities::default());

        let enabled = registry
            .enable(disabled.revision(), first_id)
            .expect("enable first Vault");
        collection.reconcile(&registry, &enabled);
        assert!(collection.runtime(first_id).is_some());
        assert!(Arc::ptr_eq(
            &second_lock,
            &collection
                .runtime(second_id)
                .expect("second runtime still retained")
                .mutation_lock
        ));

        let disconnected = registry
            .disconnect(enabled.revision(), first_id)
            .expect("disconnect first Vault");
        collection.reconcile(&registry, &disconnected);
        assert!(!collection.snapshot().vaults.contains_key(&first_id));
        assert!(Arc::ptr_eq(
            &second_lock,
            &collection
                .runtime(second_id)
                .expect("second runtime survives disconnect")
                .mutation_lock
        ));
    }
}
