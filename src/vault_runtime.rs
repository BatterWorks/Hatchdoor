use schemars::JsonSchema;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::path::PathBuf;
#[cfg(test)]
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock, Weak};

use serde::{Deserialize, Serialize};
use tracing::{error, warn};

use crate::cache::SqliteCache;
use crate::cache::vault_snapshots::VaultSnapshotFreshness;
use crate::git::ManagedGitScheduler;
use crate::startup::IndexingProgressSnapshot;
use crate::vault_registry::{
    VaultDefinition, VaultGitMode, VaultId, VaultRegistrySnapshot, VaultRegistryStore,
    VaultSource as RegistryVaultSource,
};
use crate::vault_watcher::{VaultWatcherHandle, spawn_vault_change_watcher};
use crate::vault_work::{VaultWorkCoordinator, VaultWorkErrorDetail, VaultWorkKind};

/// The server's own startup source. A Git-backed Vault is a registry Vault
/// (`vault_registry::VaultSource`), acquired and synchronized per Vault; the
/// process itself only ever holds the one local path it was started with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VaultSource {
    Local { vault_path: PathBuf },
}

impl VaultSource {
    pub fn kind(&self) -> VaultSourceKind {
        match self {
            Self::Local { .. } => VaultSourceKind::Local,
        }
    }

    pub fn mode(&self) -> VaultSourceMode {
        match self {
            Self::Local { .. } => VaultSourceMode::Local,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VaultSourceKind {
    Local,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VaultSourceMode {
    Local,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema, Deserialize)]
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, JsonSchema, Deserialize)]
pub struct VaultCapabilities {
    pub browse: bool,
    pub search: bool,
    pub mutate: bool,
    pub pull: bool,
    pub push: bool,
    pub retry: bool,
    /// Whether this Vault makes local Git commits of its own: Local history,
    /// or Two-way, whose commit is the half of its sync that needs no remote
    /// (#267). Derived from the definition alone, deliberately unlike `pull`
    /// and `push`: a console that labels its action from this must keep
    /// labelling it the same way while the Vault is failing, which is exactly
    /// when the operator reads it.
    pub commit: bool,
    /// Whether this Vault has a remote to synchronise with. What separates a
    /// console offering **Sync now** from one that can only offer **Commit
    /// now**, and definition-derived for the same reason as `commit`.
    pub sync: bool,
    /// Whether this Vault's side of a sync conflict can be published to its
    /// recovery branch now: a Two-way Vault whose Git status reports
    /// `managed_git_conflict` (ADR-30).
    pub publish_recovery: bool,
}

impl VaultCapabilities {
    /// Startup-level capabilities. Git capabilities are per-Vault and derived
    /// from the registry definition in `collection_capabilities`; the server's
    /// own local source pulls and pushes nothing.
    fn derive(source_mode: VaultSourceMode, phase: VaultPhase) -> Self {
        let ready = phase == VaultPhase::Ready;
        Self {
            browse: ready,
            search: ready,
            mutate: ready && source_mode == VaultSourceMode::Local,
            pull: false,
            push: false,
            retry: false,
            commit: false,
            sync: false,
            publish_recovery: false,
        }
    }

    /// What an unauthenticated visitor to a public read-only demo may do
    /// (#243).
    ///
    /// Both derivations above answer a question about the Vault: is this
    /// directory writable, does this source have a remote to pull, is there a
    /// retryable failure to clear. For an operator that is the right answer,
    /// because their request is admitted. On a demo it is not: `demo_guard`
    /// refuses every mutating route and every Vault-control route with `403
    /// demo_read_only` before the handler runs, so a derived `mutate`, `pull`,
    /// `push` or `retry` names a request that cannot succeed.
    ///
    /// `browse` and `search` survive derived, because those reads do work on a
    /// demo, and passing them through keeps a Vault that is indexing or
    /// unavailable as honest here as it is anywhere else.
    pub(crate) fn for_public_demo(self) -> Self {
        Self {
            mutate: false,
            pull: false,
            push: false,
            retry: false,
            commit: false,
            sync: false,
            publish_recovery: false,
            ..self
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema, Deserialize)]
pub struct VaultRuntimeError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    /// Structured detail for the handful of codes a caller genuinely cannot
    /// act on from `message` alone (see [`VaultRuntimeErrorDetail`]).
    /// `message` is unchanged by this addition and stays the authoritative
    /// human-readable text for every code. Omitted from serialized output
    /// rather than sent empty/null for every other code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<VaultRuntimeErrorDetail>,
}

/// The largest number of affected paths [`VaultRuntimeError::detail`] reports
/// by name. `total` always carries the true count, so a caller that hits the
/// cap can render "and N more" instead of an unbounded — or silently
/// truncated-looking — list.
const MAX_REPORTED_SYNC_ERROR_PATHS: usize = 50;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VaultRuntimeErrorDetail {
    /// Affected repository-relative paths for `managed_git_dirty_working_copy`
    /// or `managed_git_conflict`, capped at
    /// [`MAX_REPORTED_SYNC_ERROR_PATHS`] with `total` carrying the true count.
    AffectedPaths { paths: Vec<String>, total: usize },
    /// The local-commit count behind `managed_git_pull_only_local_commits`.
    LocalCommitsAhead { ahead: usize },
}

impl From<&VaultWorkErrorDetail> for VaultRuntimeErrorDetail {
    fn from(detail: &VaultWorkErrorDetail) -> Self {
        match detail {
            VaultWorkErrorDetail::AffectedPaths(paths) => Self::AffectedPaths {
                total: paths.len(),
                paths: paths
                    .iter()
                    .take(MAX_REPORTED_SYNC_ERROR_PATHS)
                    .cloned()
                    .collect(),
            },
            VaultWorkErrorDetail::LocalCommitsAhead(ahead) => {
                Self::LocalCommitsAhead { ahead: *ahead }
            }
        }
    }
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

    /// Unconditional; production reports progress through
    /// [`Self::set_indexing_unless_ready`].
    #[cfg(test)]
    pub(crate) fn set_indexing(&self, progress: IndexingProgressSnapshot) {
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

    /// Move the phase to `Indexing` with `progress`, unless it is already
    /// `Ready`. Decided under the one write lock, so a concurrent `set_ready`
    /// cannot be undone by a progress report that read the phase just before
    /// it.
    pub fn set_indexing_unless_ready(&self, progress: IndexingProgressSnapshot) {
        let mut snapshot = self
            .snapshot
            .write()
            .expect("vault runtime snapshot poisoned");
        if snapshot.phase == VaultPhase::Ready {
            return;
        }
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
            detail: None,
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultActivationStatus {
    Active,
    Disabled,
    Unavailable,
}

/// Whether authoritative local Markdown can currently be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalContentStatus {
    ReadWrite,
    ReadOnly,
    Unavailable,
}

/// Search availability is independent from local Markdown availability.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultSearchStatus {
    Unavailable,
    Indexing,
    /// Structural rows are published and current, but this generation has no
    /// vectors yet: the Vault can be browsed and its Notes opened, and it
    /// answers no semantic search. Reached only on a Vault's first successful
    /// index, between its structure pass and its embedding pass; a rebuild of
    /// an already-searchable Vault keeps serving the prior generation instead.
    Browsable,
    Ready,
    Stale,
}

/// Git status is kept separate so a Git failure cannot hide local Markdown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultGitStatus {
    Disabled,
    Pending,
    Ready,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultWatcherStatus {
    Running,
    Disabled,
    Unavailable,
}

/// What the last request to publish this Vault's recovery branch achieved
/// (ADR-30). In memory only: after a restart the Vault reports its conflict
/// again and the next publish fills this back in.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, JsonSchema, Deserialize)]
pub struct RecoveryBranchStatus {
    /// The branch on the remote, without `refs/heads/`. Absent only when a
    /// publish failed before the Vault's branch could be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// The local commit the branch was last published at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_commit: Option<String>,
    /// The remote commit on the configured branch that local history
    /// conflicts with, as of that publish.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflicting_commit: Option<String>,
    /// When the branch was last published, RFC 3339 UTC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_at: Option<String>,
    /// Why the latest request published nothing. A refusal leaves the
    /// earlier publication's fields in place, since that branch still stands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<VaultRuntimeError>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery_branch: Option<RecoveryBranchStatus>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VaultCollectionSnapshot {
    pub registry_revision: u64,
    /// See `VaultCollectionState::collection_revision`: collection status,
    /// never note content.
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

/// One Vault's write exclusion, as the set of handles that *are* its
/// identity: the mutation mutex every foreground write, Git turn and Index
/// read phase serializes on, the generation counter read under it, and the
/// refresh mutex.
///
/// It is a value of its own because its lifetime is not the control block's.
/// A control block is rebuilt on every definition edit — a renamed Vault, a
/// new poll interval, a re-entered credential — and a fresh mutex there would
/// admit a second writer to a directory an in-flight write, sync or index
/// turn still holds the old one for. So the exclusion travels across the
/// rotation and the edit rotates the definition only (issue #321, ADR-25).
#[derive(Clone)]
pub(crate) struct VaultWriteExclusion {
    mutation_lock: Arc<tokio::sync::Mutex<()>>,
    /// How many foreground mutations have taken `mutation_lock`. Written once
    /// per acquisition, by the acquirer, while it holds that lock — and read
    /// only by the two accessors on the control block, each of which also
    /// holds it. A holder therefore reads a value that cannot move until it
    /// releases, which is the only way to read it honestly. It travels with
    /// the lock: a generation taken before an edit must stay comparable with
    /// one taken after it, or an Index turn spanning the edit would conclude
    /// nothing had changed.
    mutations_taken: Arc<AtomicU64>,
    refresh_lock: Arc<tokio::sync::Mutex<()>>,
}

impl VaultWriteExclusion {
    fn fresh() -> Self {
        Self {
            mutation_lock: Arc::new(tokio::sync::Mutex::new(())),
            mutations_taken: Arc::new(AtomicU64::new(0)),
            refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    /// Whether these two handles are the same exclusion, rather than two
    /// mutexes that merely guard the same directory. A caller holding a guard
    /// taken from one of them is excluded from the other only when this is
    /// `true`.
    pub(crate) fn is_same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.mutation_lock, &other.mutation_lock)
    }
}

/// What a replacement control block inherits from the block it replaces. All
/// three are `None` for a genuinely new or re-enabled Vault, which has no
/// predecessor to inherit from.
#[derive(Default)]
struct CarriedOverState {
    /// The retiring block's Git status, so an in-place edit does not force a
    /// mid-backoff Vault back to `Pending`. See `activation_snapshot`.
    git: Option<(VaultGitStatus, Option<VaultRuntimeError>)>,
    /// The retiring block's write ledger, so writes already on disk and still
    /// waiting for a commit keep their summaries across the rotation (#249).
    writes: Option<Arc<crate::git::WriteLedger>>,
    /// The retiring block's write exclusion, so an edit can never put two
    /// live mutexes on one Vault directory (#321).
    exclusion: Option<VaultWriteExclusion>,
    /// The retiring block's recovery-branch status, which describes a branch
    /// on the remote that an edit to this Vault does not move (ADR-30).
    recovery: Option<RecoveryBranchStatus>,
}

/// Where a Vault's notes sit in its Git repository (#300).
pub(crate) enum HistoryLocation {
    /// A plain folder, which has no history.
    None,
    /// The repository's root, and the Vault's folder inside it,
    /// `/`-separated, when the Vault is not the whole repository.
    Repository {
        root: PathBuf,
        prefix: Option<String>,
    },
    /// A Git source whose folder cannot be spelled as repository paths.
    Unresolvable,
}

#[derive(Clone)]
pub struct VaultControlBlock {
    definition: Arc<VaultDefinition>,
    vault_path: Arc<PathBuf>,
    snapshot: Arc<RwLock<CollectionVaultSnapshot>>,
    /// This Vault's write exclusion. Outlives this block whenever a
    /// definition edit replaces it — see [`VaultWriteExclusion`].
    exclusion: VaultWriteExclusion,
    accepting_operations: Arc<AtomicBool>,
    cancellation: tokio::sync::watch::Sender<bool>,
    revisions: CollectionRevisionPublisher,
    watcher: Arc<RwLock<Option<VaultWatcherHandle>>>,
    /// What a watcher for this Vault is started with, kept so one can be
    /// started later when the Vault only becomes Active after activation (a
    /// managed checkout cloned by its first Git turn), and so a foreground
    /// write can report its change on the same intent channel (#324).
    watching: Option<WatcherContext>,
    /// The shared read model, so a foreground write can mark this Vault's
    /// published snapshot stale the moment it changes the Markdown (#324).
    snapshot_cache: Option<Arc<SqliteCache>>,
    /// This Vault's writes waiting to be named by their Git commit. The
    /// mutation core appends one record per successful write; the Vault's
    /// next Git turn takes the batch to build its commit message (#249).
    /// Lives here because those two are the only things that touch it and
    /// this is the one per-Vault handle both already hold.
    write_ledger: Arc<crate::git::WriteLedger>,
    /// When each of this Vault's notes first entered its Git history, walked
    /// once in the background and reused by every stats read until the
    /// branch moves (#300). A fresh block starts empty, which is right: a
    /// definition edit may point the Vault at another repository.
    note_history: Arc<crate::git::NoteHistory>,
}

/// Say once, when a Vault's runtime is established, that its filesystem cannot
/// commit a write as one atomic exchange.
///
/// A Vault there is still writable, through the check-then-rename path the
/// write layer falls back to, so this is not an error and does not stop
/// anything. It is said here rather than per write because the answer belongs
/// to the filesystem and does not change between saves, and because a line per
/// write would bury it (#345, ADR-26).
///
/// Returns whether it had anything to say, which is what its tests assert:
/// the message itself goes to `tracing` and only an operator reads it.
fn report_compare_and_swap_support(vault_id: VaultId, vault_path: &Path) -> bool {
    use crate::rename_flags::{FlagSupport, RenameFlag, support};

    if support(vault_path, RenameFlag::Exchange) != FlagSupport::Unsupported {
        return false;
    }
    warn!(
        vault_id = %vault_id,
        path = %vault_path.display(),
        "compare-and-swap is unavailable on this Vault's filesystem (renameat2 does not \
         support {} here); writes fall back to a non-atomic check-then-rename and are not \
         protected against a note being changed outside Hatchdoor",
        RenameFlag::Exchange.name()
    );
    true
}

/// Start one Vault's change watcher, and say what its status is.
///
/// Used where a Vault is activated already Active, and again where it only
/// becomes Active later (#324), so the two can never disagree on what a
/// failed start publishes.
fn start_watcher(
    definition: &VaultDefinition,
    vault_path: &Path,
    watching: &WatcherContext,
) -> (
    Option<VaultWatcherHandle>,
    VaultWatcherStatus,
    Option<VaultRuntimeError>,
) {
    let unavailable = |message: String| {
        (
            None,
            VaultWatcherStatus::Unavailable,
            Some(VaultRuntimeError {
                code: "vault_watcher_unavailable".to_string(),
                message,
                retryable: true,
                detail: None,
            }),
        )
    };
    let exclude = match crate::vault::ExcludeMatcher::new(definition.exclude_patterns()) {
        Ok(exclude) => exclude,
        Err(error) => return unavailable(error),
    };
    match spawn_vault_change_watcher(
        definition.vault_id(),
        vault_path.to_path_buf(),
        watching.cache_db_path.as_ref().clone(),
        exclude,
        watching.changes.clone(),
    ) {
        Ok(watcher) => (Some(watcher), VaultWatcherStatus::Running, None),
        Err(error) => unavailable(error),
    }
}

impl VaultControlBlock {
    fn activate(
        definition: VaultDefinition,
        vault_path: PathBuf,
        watching: Option<&WatcherContext>,
        snapshot_cache: Option<&Arc<SqliteCache>>,
        revisions: CollectionRevisionPublisher,
        // What this activation inherits when it replaces a live control
        // block, rather than starting a genuinely new or re-enabled Vault.
        carried_over: CarriedOverState,
    ) -> Self {
        let CarriedOverState {
            git: prior_git,
            writes: prior_writes,
            exclusion: prior_exclusion,
            recovery: prior_recovery,
        } = carried_over;
        let mut snapshot = activation_snapshot(
            &definition,
            &vault_path,
            snapshot_cache.map(Arc::as_ref),
            prior_git,
        );
        snapshot.recovery_branch = prior_recovery;
        let watcher = if snapshot.activation == VaultActivationStatus::Active {
            watching.and_then(|watching| {
                let (watcher, status, error) = start_watcher(&definition, &vault_path, watching);
                snapshot.watcher = status;
                snapshot.watcher_error = error;
                watcher
            })
        } else {
            None
        };
        snapshot.capabilities = collection_capabilities(&definition, &snapshot);
        if snapshot.activation == VaultActivationStatus::Active {
            let _ = report_compare_and_swap_support(definition.vault_id(), &vault_path);
        }
        let active = snapshot.activation == VaultActivationStatus::Active;
        let (cancellation, _) = tokio::sync::watch::channel(false);
        let block = Self {
            definition: Arc::new(definition),
            vault_path: Arc::new(vault_path),
            snapshot: Arc::new(RwLock::new(snapshot)),
            exclusion: prior_exclusion.unwrap_or_else(VaultWriteExclusion::fresh),
            accepting_operations: Arc::new(AtomicBool::new(true)),
            cancellation,
            revisions,
            watcher: Arc::new(RwLock::new(watcher)),
            watching: watching.cloned(),
            snapshot_cache: snapshot_cache.cloned(),
            write_ledger: prior_writes.unwrap_or_else(|| Arc::new(crate::git::WriteLedger::new())),
            note_history: Arc::new(crate::git::NoteHistory::default()),
        };
        if active {
            block.start_note_history();
        }
        block
    }

    /// Start dating this Vault's notes from its Git history in the
    /// background, so the first stats read after startup finds the walk done
    /// or under way rather than starting it (#300). Nothing for a plain
    /// folder, or a Git source whose repository is not there yet.
    fn start_note_history(&self) {
        let HistoryLocation::Repository { root, .. } = self.history_location() else {
            return;
        };
        let history = Arc::clone(&self.note_history);
        let _ = std::thread::Builder::new()
            .name("note-history-start".to_string())
            .spawn(move || {
                let _ = history.read(&root, std::time::Duration::ZERO);
            });
    }

    /// Where this Vault's notes sit in its Git repository, so their history
    /// can date them (#300).
    pub(crate) fn history_location(&self) -> HistoryLocation {
        use crate::vault_registry::VaultSource;

        let subdirectory = match self.definition.source() {
            VaultSource::Local { .. } => return HistoryLocation::None,
            VaultSource::ExistingGit {
                vault_subdirectory, ..
            }
            | VaultSource::ManagedGit {
                vault_subdirectory, ..
            } => vault_subdirectory.as_deref(),
        };
        let prefix = match subdirectory.map(Path::to_str) {
            None => None,
            Some(Some(text)) => Some(text.replace('\\', "/")),
            Some(None) => return HistoryLocation::Unresolvable,
        };
        // The Vault's folder is the repository joined with its subdirectory,
        // for both kinds of Git source, so the repository is that many
        // levels up.
        let depth = subdirectory.map_or(0, |subdirectory| subdirectory.components().count());
        match self.vault_path.ancestors().nth(depth) {
            Some(root) => HistoryLocation::Repository {
                root: root.to_path_buf(),
                prefix,
            },
            None => HistoryLocation::Unresolvable,
        }
    }

    pub fn definition(&self) -> &VaultDefinition {
        &self.definition
    }

    pub fn vault_path(&self) -> &Path {
        &self.vault_path
    }

    /// This Vault's write exclusion, so a caller holding a guard across a
    /// window in which the collection may reconcile can check that a freshly
    /// resolved control block still serializes against the guard it holds
    /// (the MCP `batch` tool does — issue #321).
    pub(crate) fn write_exclusion(&self) -> VaultWriteExclusion {
        self.exclusion.clone()
    }

    /// This Vault's pending write records. Cloned rather than borrowed so a
    /// Git turn can carry it into `spawn_blocking` without borrowing the
    /// control block there.
    pub fn write_ledger(&self) -> Arc<crate::git::WriteLedger> {
        Arc::clone(&self.write_ledger)
    }

    /// This Vault's cached Git history walk, for dating its notes.
    pub(crate) fn note_history(&self) -> Arc<crate::git::NoteHistory> {
        Arc::clone(&self.note_history)
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
                detail: None,
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
            detail: None,
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
                detail: None,
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
            detail: None,
        })
    }

    /// The published status survives a panic raised while its lock was held:
    /// every writer replaces whole fields and recomputes the capabilities from
    /// them, so the value behind a poisoned lock is still a coherent status.
    /// Refusing it would turn one panicked turn into a panic on every later
    /// read, status publication and readiness check of that Vault, including
    /// the shared dispatch loop's publication of the panic itself (#326).
    fn write_snapshot(&self) -> std::sync::RwLockWriteGuard<'_, CollectionVaultSnapshot> {
        self.snapshot
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Run `during` while holding this Vault's status write lock, so a test
    /// can poison it the way a turn panicking mid-publication would.
    #[cfg(test)]
    pub(crate) fn while_holding_status_lock<R>(&self, during: impl FnOnce() -> R) -> R {
        let _guard = self.write_snapshot();
        during()
    }

    pub fn snapshot(&self) -> CollectionVaultSnapshot {
        self.snapshot
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub async fn acquire_mutation(
        &self,
    ) -> Result<tokio::sync::OwnedMutexGuard<()>, VaultRuntimeError> {
        let guard = self.acquire_mutation_exclusion().await?;
        // Under the lock, and only once admission is confirmed, so the count
        // moves exactly once per mutation that actually gets to run and only
        // while one holder can see it move. Counted on acquisition rather than
        // release: by the time any other holder can read it, this mutation's
        // filesystem work has finished and its guard is gone.
        self.exclusion
            .mutations_taken
            .fetch_add(1, Ordering::Relaxed);
        Ok(guard)
    }

    /// Take this Vault's foreground mutation guard for a background Index
    /// turn's read phase, and report the mutation generation observed under
    /// it. The same exclusion a Markdown write gets, so a turn can never
    /// observe half of a multi-file foreground mutation, minus the generation
    /// advance: an Index turn reads the Vault rather than mutating it, and
    /// counting itself would make every turn conclude a mutation had
    /// intervened and report its own publication stale (issue #223).
    ///
    /// The generation is read here rather than exposed, because it is only
    /// meaningful under this lock: pass it back to
    /// [`Self::blocking_retake_mutation_for_index`], which decides and hands
    /// back the guard to act under.
    pub(crate) async fn acquire_mutation_for_index_reads(
        &self,
    ) -> Result<(tokio::sync::OwnedMutexGuard<()>, u64), VaultRuntimeError> {
        let guard = self.acquire_mutation_exclusion().await?;
        let generation = self.exclusion.mutations_taken.load(Ordering::Relaxed);
        Ok((guard, generation))
    }

    /// The exclusion itself: everything both acquisitions above have in common,
    /// minus what makes one a mutation and the other a read.
    async fn acquire_mutation_exclusion(
        &self,
    ) -> Result<tokio::sync::OwnedMutexGuard<()>, VaultRuntimeError> {
        self.ensure_accepting_operations()?;
        let guard = self.exclusion.mutation_lock.clone().lock_owned().await;
        self.ensure_accepting_operations()?;
        Ok(guard)
    }

    /// Retake the foreground mutation guard from a blocking thread and answer,
    /// under that one acquisition, whether a foreground mutation completed
    /// since `since` (a generation from
    /// [`Self::acquire_mutation_for_index_reads`]).
    ///
    /// An Index turn releases the guard before its embedding pass and calls
    /// this to publish afterwards: the verdict and the publication it labels
    /// share the returned guard, so no mutation can land between deciding and
    /// acting (the rule
    /// [`crate::vault_work::VaultWorkCoordinator::request_if_idle`] records for
    /// issue #127).
    ///
    /// Blocking by construction — call it only from a thread that is not
    /// driving the async runtime, such as inside `spawn_blocking`; Tokio panics
    /// otherwise. The wait is bounded by one in-flight foreground mutation, not
    /// by a turn, which matters because the Index turn holds the cache's
    /// process-wide model epoch across this call.
    pub(crate) fn blocking_retake_mutation_for_index(
        &self,
        since: u64,
    ) -> (bool, tokio::sync::OwnedMutexGuard<()>) {
        let guard = self.exclusion.mutation_lock.clone().blocking_lock_owned();
        let mutated = self.exclusion.mutations_taken.load(Ordering::Relaxed) != since;
        (mutated, guard)
    }

    /// Wait until a mutation that was admitted before this control block was
    /// retired has completed. Callers must revoke operation admission first,
    /// so a queued mutation re-checks that state and cannot begin afterwards.
    async fn wait_for_mutation_safe_boundary(&self) {
        let _guard = self.exclusion.mutation_lock.clone().lock_owned().await;
    }

    pub async fn acquire_refresh(
        &self,
    ) -> Result<tokio::sync::OwnedMutexGuard<()>, VaultRuntimeError> {
        self.ensure_accepting_operations()?;
        let guard = self.exclusion.refresh_lock.clone().lock_owned().await;
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
            detail: None,
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
        let mut snapshot = self.write_snapshot();
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
        let mut snapshot = self.write_snapshot();
        let previous = snapshot.clone();
        snapshot.local_content = status;
        snapshot.activation = if status == LocalContentStatus::Unavailable {
            VaultActivationStatus::Unavailable
        } else {
            VaultActivationStatus::Active
        };
        snapshot.activation_error = error;
        // A managed Git Vault has no directory to probe when its runtime is
        // established, because its checkout has not landed yet. This seam is
        // where it becomes Active, so it is the second place the filesystem
        // report can first be made, and the transition is what keeps it to
        // one line rather than one per status publish (#345). It is also the
        // second place a watcher can start: activation skipped it for want of
        // a directory, and a watcher left on a checkout that was lost and
        // re-cloned watches a directory that no longer exists (#324).
        let became_active = snapshot.activation == VaultActivationStatus::Active
            && previous.activation != VaultActivationStatus::Active;
        if became_active {
            self.rearm_watcher(&mut snapshot);
        }
        snapshot.capabilities = collection_capabilities(&self.definition, &snapshot);
        let changed = *snapshot != previous;
        drop(snapshot);
        if became_active {
            let _ = report_compare_and_swap_support(self.definition.vault_id(), &self.vault_path);
        }
        if changed {
            self.revisions
                .bump(self.definition.vault_id(), VaultChangeCategory::Status);
        }
        Ok(())
    }

    /// Replace this Vault's watcher with a fresh one over its current
    /// directory, publishing the result into `snapshot`. A no-op for a
    /// collection that does not watch, outside a Tokio runtime (the watcher
    /// task needs one), or once the block has been revoked: the watcher slot
    /// is taken before admission is re-checked, and `revoke` reads that slot
    /// only after withdrawing admission, so a retiring block can never be
    /// left holding a watcher nobody cancels.
    fn rearm_watcher(&self, snapshot: &mut CollectionVaultSnapshot) {
        let Some(watching) = self.watching.as_ref() else {
            return;
        };
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        let mut slot = self.watcher.write().expect("Vault watcher handle poisoned");
        if !self.is_accepting_operations() {
            return;
        }
        if let Some(previous) = slot.take() {
            previous.cancel();
        }
        let (watcher, status, error) = start_watcher(&self.definition, &self.vault_path, watching);
        *slot = watcher;
        snapshot.watcher = status;
        snapshot.watcher_error = error;
    }

    /// Label this Vault's published snapshot stale because a foreground write
    /// has just changed its Markdown, so a collection read between the write
    /// and the Index turn it asks for does not call the old generation fresh
    /// (#324). The write path does this itself rather than waiting on a
    /// watcher, which may not exist or may have lost the event.
    ///
    /// Blocking (it writes SQLite): call it from the blocking pool, while
    /// holding this Vault's foreground mutation guard. A failure is logged
    /// rather than returned, because the write it follows has already landed.
    pub(crate) fn mark_snapshot_behind_write(&self) {
        let Some(cache) = self.snapshot_cache.as_ref() else {
            return;
        };
        let vault_id = self.definition.vault_id();
        if let Err(error) = cache.mark_vault_snapshot_behind_write(vault_id) {
            warn!(%vault_id, %error, "could not mark the Vault snapshot stale after a write");
        }
    }

    /// Ask for this Vault's commit and Index turns after a foreground write,
    /// on the same intent channel its watcher reports on, so the write is
    /// indexed and committed whether or not a watcher saw it (#324). The
    /// forwarder in `server.rs` applies the commit mode and cooldown, and the
    /// coordinator coalesces this with the watcher's own report of the same
    /// change.
    pub(crate) fn report_write(&self) {
        if let Some(watching) = self.watching.as_ref() {
            let _ = watching.changes.send(self.definition.vault_id());
        }
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
        let mut snapshot = self.write_snapshot();
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

    /// Publish the outcome of a recovery-branch request, or clear it once
    /// the conflict it was about is gone (ADR-30).
    pub fn set_recovery_branch(
        &self,
        status: Option<RecoveryBranchStatus>,
    ) -> Result<(), VaultRuntimeError> {
        self.ensure_accepting_operations()?;
        let mut snapshot = self.write_snapshot();
        let changed = snapshot.recovery_branch != status;
        snapshot.recovery_branch = status;
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
    /// Counts changes to the Vault collection: a Vault added, edited,
    /// enabled, disabled or disconnected, or one Vault's search, Git,
    /// watcher or local-file status moving. It does not count note
    /// content. A note write never advances it; the Index turn that write
    /// arms usually does, twice, but only because the Vault's search status
    /// passes through `indexing` and back. Neither its moving nor its
    /// holding still says whether a read includes a given write: that is
    /// what a collection read's `participants[].state` answers (#259).
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
    ///
    /// The replacement control blocks are built before the collection's write
    /// lock is taken, not under it. Activating one stats its directory, reads
    /// its retained snapshot from SQLite and registers a recursive watcher on
    /// every directory of the Vault, and every HTTP and MCP read waits on that
    /// lock: holding it across the walk froze all of them for the length of a
    /// large Vault's watcher registration on every registry mutation (#326).
    /// The lock is then held only to install the result, and only if no other
    /// reconciliation installed a revision in between; if one did, this one's
    /// unadopted blocks are revoked and it starts over from what is live.
    pub fn reconcile(
        &self,
        registry: &VaultRegistryStore,
        snapshot: &VaultRegistrySnapshot,
    ) -> bool {
        let revision_publisher = CollectionRevisionPublisher {
            state: Arc::downgrade(&self.state),
            revisions: self.revisions.clone(),
        };
        let (mut state, previous, next) = loop {
            let (observed_revision, observed) = {
                let state = self
                    .state
                    .read()
                    .expect("Vault collection runtime poisoned");
                if snapshot.revision() <= state.registry_revision {
                    return false;
                }
                (state.registry_revision, state.vaults.clone())
            };
            let next = self.next_entries(registry, snapshot, &observed, &revision_publisher);
            let mut state = self
                .state
                .write()
                .expect("Vault collection runtime poisoned");
            if state.registry_revision == observed_revision {
                // Only `reconcile` replaces the map, and it always advances
                // the revision, so an unchanged revision is an unchanged map.
                let previous = std::mem::take(&mut state.vaults);
                break (state, previous, next);
            }
            drop(state);
            revoke_unadopted(&observed, &next);
        };

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

    /// The collection `snapshot` describes, built from `previous`: retained
    /// control blocks where a definition and path are unchanged, freshly
    /// activated ones everywhere else. Called without the collection lock.
    fn next_entries(
        &self,
        registry: &VaultRegistryStore,
        snapshot: &VaultRegistrySnapshot,
        previous: &BTreeMap<VaultId, VaultCollectionEntry>,
        revision_publisher: &CollectionRevisionPublisher,
    ) -> BTreeMap<VaultId, VaultCollectionEntry> {
        let mut next = BTreeMap::new();
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
                        //
                        // The retiring block's pending write records move
                        // across for the same reason: they describe writes
                        // already on disk and still uncommitted, so dropping
                        // them here would lose exactly the commit-message
                        // lines #249 exists to deliver.
                        //
                        // So does its write exclusion, and that one is a
                        // correctness requirement rather than a convenience.
                        // This replacement becomes visible to `open()` the
                        // moment the state lock below is dropped, while the
                        // retiring block's in-flight write, Git turn or Index
                        // read phase still holds its guard and is only waited
                        // for afterwards. A fresh mutex here would admit a
                        // second writer to the same directory for the whole
                        // of that window — issue #96's defect, reintroduced
                        // by every benign edit (name, interval, exclude
                        // patterns, credentials). Carrying the exclusion
                        // across makes an edit rotate the definition and
                        // nothing else (#321, ADR-25).
                        let carried_over = if let Some(VaultCollectionEntry::Active(runtime)) =
                            previous_entry
                        {
                            let prior_snapshot = runtime.snapshot();
                            CarriedOverState {
                                git: Some((prior_snapshot.git, prior_snapshot.git_error.clone())),
                                recovery: prior_snapshot.recovery_branch.clone(),
                                writes: Some(runtime.write_ledger()),
                                exclusion: Some(runtime.write_exclusion()),
                            }
                        } else {
                            CarriedOverState::default()
                        };
                        #[cfg(test)]
                        observe_activation();
                        VaultCollectionEntry::Active(VaultControlBlock::activate(
                            definition,
                            vault_path,
                            self.watching.as_ref(),
                            self.snapshot_cache.as_ref(),
                            revision_publisher.clone(),
                            carried_over,
                        ))
                    }
                }
            };
            next.insert(vault_id, entry);
        }
        next
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
        // A Vault in `previously_present` but not `currently_present` has left
        // the collection: prune the durable Git-turn record along with its
        // disposable cache rows, so nothing that reconnects under this Vault
        // ID inherits its countdown. Disabling keeps both — the Vault is
        // still in the collection and resumes its schedule when re-enabled.
        for vault_id in previously_present.difference(&currently_present) {
            managed_git.forget_persisted_state(*vault_id);
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
            // `VaultDefinition` compares unequal). Such an edit does not
            // request a turn *of its own* — the due-check below is still the
            // only thing that starts one — but it can leave the Vault due,
            // and then that check starts a turn: `activate` brings an armed
            // attempt forward when the new interval is shorter, and an
            // interval shortened past the time already elapsed is due at
            // once. That is the point of shortening it, not a reset.
            let scheduled = runtime.definition().source().managed_git_poll_interval();
            if let Some(poll_interval) = scheduled {
                managed_git.activate(*vault_id, poll_interval);
                // A fresh process publishes `Pending` for every Vault, which
                // is only true of one that has never completed a turn. Now
                // that a restart no longer forces an immediate turn, leaving
                // it at `Pending` would report nothing wrong about a Vault
                // that is in fact failing, for up to a whole poll interval.
                // Republishing the remembered outcome carries the previous
                // process's conclusion across the restart that erased it.
                //
                // Guarded on `Pending` so an in-process definition edit keeps
                // whatever status it was carrying (`reconcile` preserves it
                // through `prior_git`) — a Vault mid-backoff from a transient
                // failure must not be reset to the last *interval-arming*
                // outcome, which is older.
                if snapshot.git == VaultGitStatus::Pending
                    && let Some(remembered) = managed_git.remembered_turn(*vault_id)
                {
                    let (status, error) = remembered_git_status(&remembered);
                    let _ = runtime.set_git_status(status, error);
                }
                // Due now — a Vault that has never synced, or one already
                // past its interval — starts its turn here rather than
                // waiting out a tick. One still inside its interval is left
                // to the schedule its last turn armed.
                managed_git.request_if_due(*vault_id);
            }
            // A scheduler-tracked Vault has already had its due-check above,
            // which is the only thing that should start one of its turns.
            // Requesting here on `Pending` as well would undo it: activation
            // publishes `Pending` on every fresh process, so every restart
            // would re-sync and then re-arm the interval from the restart —
            // exactly how a deployment redeployed more often than its poll
            // interval never reached a scheduled turn.
            //
            // A Git-capable source the scheduler does *not* track — an
            // `ExistingGit` Vault in `LocalHistory` mode, which has no remote
            // to poll, needs its activation turn from here: nothing else
            // publishes a first Git status for it. A commit turn, because a
            // commit is the whole of what such a Vault's Git does; the
            // watcher and a manual control ask for the same kind (#267).
            if snapshot.git == VaultGitStatus::Pending && scheduled.is_none() {
                coordinator.request(*vault_id, VaultWorkKind::Commit);
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

/// Revoke every control block `next` activated rather than carried over from
/// `previous`, for a reconciliation that lost the race to install it: each
/// may already be watching its directory, and nothing else would stop it.
fn revoke_unadopted(
    previous: &BTreeMap<VaultId, VaultCollectionEntry>,
    next: &BTreeMap<VaultId, VaultCollectionEntry>,
) {
    for (vault_id, entry) in next {
        let VaultCollectionEntry::Active(runtime) = entry else {
            continue;
        };
        let carried_over = matches!(
            previous.get(vault_id),
            Some(VaultCollectionEntry::Active(previous_runtime))
                if Arc::ptr_eq(&previous_runtime.snapshot, &runtime.snapshot)
        );
        if !carried_over {
            runtime.revoke();
        }
    }
}

#[cfg(test)]
thread_local! {
    /// Called on the reconciling thread just before each control block is
    /// activated, so a test can observe what that thread holds at the time.
    pub(crate) static ACTIVATION_OBSERVER: std::cell::RefCell<Option<Box<dyn Fn()>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn observe_activation() {
    ACTIVATION_OBSERVER.with(|observer| {
        if let Some(observer) = observer.borrow().as_ref() {
            observer();
        }
    });
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
        recovery_branch: None,
    };
    snapshot.capabilities = collection_capabilities(definition, &snapshot);
    snapshot
}

/// The Git status a restarted instance should publish for a Vault whose last
/// interval-arming turn is remembered. Only non-retryable failures are ever
/// remembered as failures, so a remembered failure is never retryable.
fn remembered_git_status(
    remembered: &crate::vault_runtime_state::GitTurnRecord,
) -> (VaultGitStatus, Option<VaultRuntimeError>) {
    match &remembered.outcome {
        crate::vault_runtime_state::GitTurnOutcome::Failed { code, message } => (
            VaultGitStatus::Unavailable,
            Some(VaultRuntimeError {
                code: code.clone(),
                message: message.clone(),
                retryable: false,
                detail: None,
            }),
        ),
        crate::vault_runtime_state::GitTurnOutcome::UpToDate
        | crate::vault_runtime_state::GitTurnOutcome::Synchronized => (VaultGitStatus::Ready, None),
    }
}

/// A retained participating snapshot is immediately searchable after process
/// reconstruction, even while the coordinator has queued a fresh Index turn.
/// Cache read failure and nonparticipation remain conservatively unavailable.
///
/// A structure-only generation is the exception: it is participating and
/// fresh, but has no vectors, so reconstruction must report it `Browsable`
/// rather than `Ready`. Reading freshness alone would have a restart mid-first-
/// index claim search works and then answer every query with nothing.
fn retained_snapshot_search_status(
    snapshot_cache: Option<&SqliteCache>,
    vault_id: VaultId,
) -> VaultSearchStatus {
    match snapshot_cache.and_then(|cache| cache.snapshot_status(vault_id).ok().flatten()) {
        Some(status) if status.participating => match (status.freshness, status.searchable) {
            (VaultSnapshotFreshness::Fresh, true) => VaultSearchStatus::Ready,
            (VaultSnapshotFreshness::Stale, true) => VaultSearchStatus::Stale,
            // Vectorless outranks stale. `Stale` grants the search capability
            // (a stale generation still answers from its retained vectors),
            // which a generation with no vectors at all must never do.
            (_, false) => VaultSearchStatus::Browsable,
        },
        Some(_) | None => VaultSearchStatus::Unavailable,
    }
}

/// Stat `vault_path` and derive its current local-content availability. The
/// single source of truth for that derivation: `activation_snapshot` uses it
/// at `reconcile()` time, and `publish_local_content_after_git_success` uses
/// it again after a managed-Git checkout materializes, so the two call sites
/// can never drift on what "unavailable" means for a Vault path.
pub(crate) fn stat_local_content(
    vault_path: &Path,
) -> (LocalContentStatus, Option<VaultRuntimeError>) {
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
                detail: None,
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
                detail: None,
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
        detail: None,
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
        detail: None,
    })?;
    // SAFETY: `path` is a live, null-terminated C string and `faccessat` does
    // not retain the pointer. AT_EACCESS checks the server's effective identity.
    let result =
        unsafe { libc::faccessat(libc::AT_FDCWD, path.as_ptr(), libc::W_OK, libc::AT_EACCESS) };
    if result == 0 {
        return Ok(true);
    }
    classify_write_probe_failure(vault_path, &std::io::Error::last_os_error())
}

#[cfg(not(unix))]
fn directory_is_writable(
    _vault_path: &Path,
    metadata: &std::fs::Metadata,
) -> Result<bool, VaultRuntimeError> {
    Ok(!metadata.permissions().readonly())
}

/// Did the `W_OK` probe fail because the Vault is present but refuses writes,
/// or because it is unreachable? The former is `Ok(false)`, which
/// `directory_content_status` turns into `LocalContentStatus::ReadOnly`; the
/// latter keeps surfacing as an unavailable path.
///
/// Issue #178: a Docker `:ro` bind mount answers the probe with `EROFS`, not
/// `EACCES`. Both mean the same thing to us - browse and index the Vault,
/// refuse mutations - so both must classify as read-only.
#[cfg(unix)]
fn classify_write_probe_failure(
    vault_path: &Path,
    error: &std::io::Error,
) -> Result<bool, VaultRuntimeError> {
    if matches!(
        error.kind(),
        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ReadOnlyFilesystem
    ) {
        return Ok(false);
    }
    Err(VaultRuntimeError {
        code: "vault_path_unavailable".to_string(),
        message: format!(
            "Vault path '{}' availability check failed: {error}",
            vault_path.display()
        ),
        retryable: true,
        detail: None,
    })
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
        recovery_branch: None,
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
    let source = definition.source();
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
        commit: crate::git::source_commits(source),
        sync: crate::git::source_syncs_remote(source),
        publish_recovery: git_mode == Some(VaultGitMode::TwoWay)
            && snapshot
                .git_error
                .as_ref()
                .is_some_and(|error| error.code == crate::git::CONFLICT_CODE),
    }
}

#[cfg(test)]
mod tests;
