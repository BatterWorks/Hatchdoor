//! Where one Vault background turn actually runs.
//!
//! The work coordinator (`crate::vault_work`) decides *which* Vault takes the
//! next turn and coalesces duplicate requests; this module decides what that
//! turn does, runs it, and publishes what it produced. Reading
//! [`VaultWorkExecutor`] and the two turn functions below explains a whole
//! Index turn or Git turn without following the composition root, the
//! collection runtime, and the Git scheduler in parallel.
//!
//! `server.rs` keeps only the loop that takes the next coordinator position
//! and hands it here: no readiness policy, no turn logic, no per-turn
//! dependency assembly.
//!
//! Per ADR-13 and ADR-18 this is a plain module with a small public surface —
//! no trait and no framework. Which turns may run at the same time is the
//! coordinator's decision (ADR-31), never a turn's.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tracing::{debug, error, info, warn};

use crate::app_state::AppState;
use crate::cache::vault_snapshots::{
    MutationGuardHandoff, SnapshotPublication, VaultSnapshotFreshness,
};
use crate::cache::{IndexYield, SqliteCache};
use crate::embed::Embedder;
use crate::git::{
    CommitCooldown, ManagedCheckoutLease, ManagedGitOutcome, ManagedGitScheduler,
    ManagedGitTurnConfig, RecoveryFailure, RecoveryResult, run_existing_git_commit_turn,
    run_existing_git_recovery_turn, run_existing_git_remote_turn, run_managed_git_commit_turn,
    run_managed_git_turn, run_managed_recovery_turn,
};
use crate::runtime_config::{ConfigSnapshot, RuntimeConfig};
use crate::startup::{IndexingParticipant, IndexingProgressSnapshot, StartupTracker};
use crate::vault_registry::{
    VaultGitMode, VaultId, VaultRegistryState, VaultRegistryStore,
    VaultSource as RegistryVaultSource,
};
use crate::vault_runtime::{
    CollectionVaultSnapshot, LocalContentStatus, RecoveryBranchStatus, VaultActivationStatus,
    VaultCollectionRuntime, VaultControlBlock, VaultGitStatus, VaultIndexTurn, VaultRuntimeError,
    VaultRuntimeErrorDetail, VaultSearchStatus, stat_local_content,
};
use crate::vault_runtime_state::format_timestamp;
use crate::vault_work::{
    IndexLaneState, TURN_PANICKED, VaultWorkCoordinator, VaultWorkError, VaultWorkKind,
    VaultWorkOutcome, VaultWorkRequest,
};

#[cfg(test)]
static INDEX_MUTATION_PROBE: Mutex<Option<(VaultId, Arc<tokio::sync::Notify>)>> = Mutex::new(None);

/// Test-only rendezvous for proving an Index turn has reached its foreground
/// mutation-lock attempt without relying on scheduler timing.
#[cfg(test)]
pub(crate) struct IndexMutationProbe {
    vault_id: VaultId,
    lock_attempted: Arc<tokio::sync::Notify>,
}

#[cfg(test)]
impl IndexMutationProbe {
    pub(crate) fn install(vault_id: VaultId) -> Self {
        let lock_attempted = Arc::new(tokio::sync::Notify::new());
        *INDEX_MUTATION_PROBE
            .lock()
            .expect("Index mutation probe poisoned") = Some((vault_id, lock_attempted.clone()));
        Self {
            vault_id,
            lock_attempted,
        }
    }

    pub(crate) async fn lock_attempted(&self) {
        self.lock_attempted.notified().await;
    }
}

#[cfg(test)]
impl Drop for IndexMutationProbe {
    fn drop(&mut self) {
        let mut installed = INDEX_MUTATION_PROBE
            .lock()
            .expect("Index mutation probe poisoned");
        if installed
            .as_ref()
            .is_some_and(|(vault_id, _)| *vault_id == self.vault_id)
        {
            *installed = None;
        }
    }
}

#[cfg(test)]
fn notify_index_mutation_lock_attempt(vault_id: VaultId) {
    let probe = INDEX_MUTATION_PROBE
        .lock()
        .expect("Index mutation probe poisoned")
        .as_ref()
        .filter(|(probed_vault_id, _)| *probed_vault_id == vault_id)
        .map(|(_, lock_attempted)| lock_attempted.clone());
    if let Some(lock_attempted) = probe {
        lock_attempted.notify_one();
    }
}

/// Everything one Vault background turn can need, assembled once at startup.
///
/// The per-turn settings snapshot is deliberately *not* a field: [`Self::run`]
/// takes it at the start of every turn, so an admitted operation observes one
/// immutable configuration view even if settings change later, while a saved
/// setting still reaches the *next* turn without a restart.
#[derive(Clone)]
pub(crate) struct VaultWorkExecutor {
    vaults: VaultCollectionRuntime,
    registry: VaultRegistryStore,
    work: VaultWorkCoordinator,
    managed_git: Arc<ManagedGitScheduler>,
    commit_cooldown: Arc<CommitCooldown>,
    cache: Arc<SqliteCache>,
    embedder: Arc<dyn Embedder>,
    runtime_config: RuntimeConfig,
    startup: StartupTracker,
    model_setup_started: Arc<AtomicBool>,
    index_retries: IndexRetries,
    /// How long an Index turn embeds before it offers its slot to another
    /// Vault's queued indexing. [`INDEX_TURN_SLICE`] in production; tests
    /// shorten it.
    index_slice: Duration,
}

/// How long an Index turn embeds before it checks whether another Vault is
/// waiting to index, and pauses for it if one is (ADR-35 decision 3). A
/// fixed constant, not a setting (ADR-14).
const INDEX_TURN_SLICE: Duration = Duration::from_secs(5 * 60);

/// How long the first automatic retry of a failed Index turn waits. Each
/// further consecutive failure of the same Vault doubles it.
const INDEX_RETRY_BASE_DELAY: Duration = Duration::from_secs(30);

/// How many automatic retries one Vault's run of consecutive Index failures
/// gets before it waits for something else (a change, a refresh, a restart)
/// to ask for another turn. Bounded so a Vault that cannot index at all does
/// not keep the one indexing lane busy forever.
const INDEX_RETRY_LIMIT: u32 = 5;

/// Each Vault's count of consecutive retryable Index failures, so the retry a
/// failure schedules backs off and stops. A successful Index turn clears it.
/// In memory only: a restart queues a fresh Index turn for every Vault anyway.
#[derive(Clone, Default)]
struct IndexRetries(Arc<Mutex<BTreeMap<VaultId, u32>>>);

impl IndexRetries {
    /// Count one more failure for `vault_id` and return how long to wait
    /// before retrying it, or `None` once its retries are spent.
    fn next_delay(&self, vault_id: VaultId) -> Option<Duration> {
        let mut failures = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let attempt = failures.entry(vault_id).or_insert(0);
        if *attempt >= INDEX_RETRY_LIMIT {
            return None;
        }
        let delay = INDEX_RETRY_BASE_DELAY.saturating_mul(1 << *attempt);
        *attempt += 1;
        Some(delay)
    }

    fn clear(&self, vault_id: VaultId) {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&vault_id);
    }
}

impl VaultWorkExecutor {
    /// Every field is one of `AppState`'s own, so the composition root has
    /// nothing to assemble: the executor is exactly the slice of shared
    /// runtime state a background turn is allowed to touch.
    ///
    /// Built once, for the one dispatch loop, which is why it is also where
    /// each Vault's status starts following the indexing lane.
    pub(crate) fn from_state(state: &AppState) -> Self {
        report_index_lane_on_vault_status(&state.vaults, &state.vault_work);
        Self {
            vaults: state.vaults.clone(),
            registry: state.vault_registry.clone(),
            work: state.vault_work.clone(),
            managed_git: state.managed_git.clone(),
            commit_cooldown: state.commit_cooldown.clone(),
            cache: state.startup_sqlite.clone(),
            embedder: state.embedder.clone(),
            runtime_config: state.runtime_config.clone(),
            startup: state.startup.clone(),
            model_setup_started: state.model_setup_started.clone(),
            index_retries: IndexRetries::default(),
            index_slice: INDEX_TURN_SLICE,
        }
    }

    /// Run exactly one admitted turn.
    pub(crate) async fn run(&self, request: VaultWorkRequest) -> Result<(), VaultWorkError> {
        // Bound once, at the start of the turn: every setting this turn reads
        // comes from the same immutable view.
        let snapshot = self.runtime_config.snapshot();
        match request.kind() {
            VaultWorkKind::Git => {
                // Read per turn, not once at startup, so saving a new author
                // name or email applies to the next Git turn of every Vault
                // without its own commit identity — no restart.
                let (author_name, author_email) = git_author_defaults(&snapshot);
                dispatch_git_turn(
                    &self.vaults,
                    &self.registry,
                    &self.work,
                    &self.managed_git,
                    &author_name,
                    &author_email,
                    request,
                )
                .await
            }
            VaultWorkKind::Commit => {
                // Read per turn for the same reason the Git arm above does.
                let (author_name, author_email) = git_author_defaults(&snapshot);
                dispatch_commit_turn(
                    &self.vaults,
                    &self.registry,
                    &self.managed_git,
                    &self.commit_cooldown,
                    &author_name,
                    &author_email,
                    request,
                )
                .await
            }
            VaultWorkKind::Recovery => {
                // Read per turn for the same reason the Git arm above does:
                // a publish commits pending saves before it pushes.
                let (author_name, author_email) = git_author_defaults(&snapshot);
                dispatch_recovery_turn(
                    &self.vaults,
                    &self.registry,
                    &self.managed_git,
                    &author_name,
                    &author_email,
                    request,
                )
                .await
            }
            VaultWorkKind::Index => {
                let embed_layers = snapshot
                    .setting("HATCHDOOR_EMBED_LAYERS")
                    .map(|setting| crate::runtime_config::is_truthy(&setting.value))
                    .unwrap_or(true);
                let progress_startup = self.startup.clone();
                let progress_vaults = self.vaults.clone();
                let vault_id = request.vault_id();
                dispatch_vault_index_turn_with_progress(
                    &self.vaults,
                    self.cache.clone(),
                    self.embedder.clone(),
                    embed_layers,
                    Some(Arc::new(move |progress| {
                        report_first_run_progress(
                            &progress_startup,
                            &progress_vaults,
                            vault_id,
                            progress,
                        );
                    })),
                    Some(IndexTurnSlicing {
                        work: self.work.clone(),
                        slice: self.index_slice,
                    }),
                    request,
                )
                .await
            }
            VaultWorkKind::Repair => Err(VaultWorkError::new(
                "vault_work_kind_not_yet_implemented",
                format!("{:?} dispatch is not implemented yet", request.kind()),
                false,
            )),
        }
    }

    /// Apply one completed turn's instance-wide consequences: the startup
    /// readiness rule, an Index failure's retry, and the operator-facing log
    /// line.
    ///
    /// Per-Vault status is already published by the turn itself; this is only
    /// what the *collection* concludes from a turn having finished. One
    /// Vault's failure is never the instance's: it stays on that Vault's own
    /// status, where every collection read already reports it (#326).
    ///
    /// It runs on the dispatch loop every Vault shares, so it contains its
    /// own panics instead of ending that loop.
    pub(crate) fn publish_outcome(&self, outcome: &VaultWorkOutcome) {
        let vault_id = outcome.request.vault_id();
        // Logged before anything touches the Vault's state, so a failure
        // below cannot swallow the line that explains it.
        if let Err(error) = &outcome.result {
            // Repair remains expected until its dedicated packet;
            // Index and Git failures are actionable per-Vault status.
            if error.code() == "vault_work_kind_not_yet_implemented" {
                debug!(
                    vault_id = %vault_id,
                    kind = ?outcome.request.kind(),
                    "Vault background work kind not yet implemented"
                );
            } else if error.code() == TURN_PANICKED {
                error!(
                    vault_id = %vault_id,
                    kind = ?outcome.request.kind(),
                    message = error.message(),
                    "Vault background work turn panicked; the turn was abandoned and \
                     background work continues"
                );
            } else {
                warn!(
                    vault_id = %vault_id,
                    kind = ?outcome.request.kind(),
                    code = error.code(),
                    message = error.message(),
                    "Vault background work turn failed"
                );
            }
        }
        // This runs on the dispatch loop every Vault shares, outside the
        // turn's own panic boundary. A panic here, e.g. while publishing the
        // status of a Vault whose turn just panicked, would end that loop
        // and stop every Vault's background work, so it is contained too.
        let consequences = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.apply_collection_consequences(outcome);
        }));
        if let Err(panic) = consequences {
            error!(
                vault_id = %vault_id,
                kind = ?outcome.request.kind(),
                message = crate::vault_work::panic_message(panic.as_ref()),
                "publishing a Vault background work outcome panicked; background work continues"
            );
        }
    }

    fn apply_collection_consequences(&self, outcome: &VaultWorkOutcome) {
        let vault_id = outcome.request.vault_id();
        if outcome.request.kind() != VaultWorkKind::Index {
            return;
        }
        match &outcome.result {
            Ok(()) => self.index_retries.clear(vault_id),
            Err(error) if error.code() == TURN_PANICKED => {
                // The turn never reached its own failure publication, so
                // it would otherwise read `Indexing` forever.
                if let Some(control_block) = self.vaults.runtime(vault_id) {
                    publish_index_failure(&control_block, &self.cache, error, true);
                }
            }
            // Deferred until the embedder is installed, which re-requests
            // every active Vault itself.
            Err(error) if error.code() == "embedder_not_ready" => {}
            Err(error) if error.retryable() => self.schedule_index_retry(vault_id),
            Err(_) => {}
        }
        if self.startup.collection_indexes_ready() {
            return;
        }
        // The finished turn's Vault now counts as done in the startup
        // reading, even if it failed partway.
        self.startup
            .refresh_indexing_participants(indexing_participants(&self.vaults));
        settle_startup(
            &self.startup,
            &self.vaults,
            &self.registry,
            &self.model_setup_started,
        );
    }

    /// Ask for another Index turn of `vault_id` after a backoff, unless this
    /// run of failures has used up its retries. Nothing else would: a failed
    /// turn otherwise waits for an unrelated change or a manual refresh.
    fn schedule_index_retry(&self, vault_id: VaultId) {
        let Some(delay) = self.index_retries.next_delay(vault_id) else {
            warn!(
                %vault_id,
                "Vault indexing keeps failing; automatic retries stopped until the next change \
                 or refresh"
            );
            return;
        };
        info!(%vault_id, delay_seconds = delay.as_secs(), "retrying Vault indexing after a backoff");
        let work = self.work.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            // Automatic, so it must not add a rerun to an Index turn some
            // other change has already queued or started. A drained or
            // shut-down Vault rejects it.
            work.request_if_idle(vault_id, VaultWorkKind::Index);
        });
    }
}

/// Latch startup `Ready` if the instance is ready now: the search model is
/// set up, the Vault registry loaded normally, and every active Vault's first
/// index has settled ([`collection_indexes_settled`]). Latching also releases
/// the model-setup claim, so a later retry can load the model again.
///
/// An Index turn's outcome asks this through
/// [`VaultWorkExecutor::publish_outcome`]. The composition root asks at the
/// moments no Index turn covers (#453): when model setup finishes, and,
/// through [`settle_startup_on_collection_changes`], when the collection
/// changes. With no active Vault no Index turn ever runs, so those are the
/// only moments an instance without Vaults can become ready.
pub(crate) fn settle_startup(
    startup: &StartupTracker,
    vaults: &VaultCollectionRuntime,
    registry: &VaultRegistryStore,
    model_setup_started: &AtomicBool,
) {
    if startup.collection_indexes_ready() || startup.model_setup_pending() {
        return;
    }
    if !collection_indexes_settled(vaults) {
        return;
    }
    // A registry awaiting operator recovery, or one that cannot be read,
    // activates no Vault. That is not an instance with no Vaults: it can
    // serve nothing until it is recovered. Active Vaults came from a
    // registry that loaded, so only their absence needs the question asked.
    if vaults.active_vault_ids().is_empty()
        && !matches!(registry.load(), Ok(VaultRegistryState::Ready(_)))
    {
        return;
    }
    // Model setup may have begun since the check above; the tracker refuses
    // the latch then, and the claim stays with that setup.
    if startup.settle_ready() {
        model_setup_started.store(false, Ordering::Release);
        info!("Startup ready: no active Vault is waiting on its first index");
    }
}

/// Ask [`settle_startup`] again whenever the Vault collection changes, until
/// the task is aborted. Disabling or disconnecting the last Vault still in
/// its first index leaves nothing to wait for, and no Index turn outcome is
/// sure to follow the change.
///
/// Subscribes when called, not when first polled, so no change between the
/// call and the task's first run is missed.
pub(crate) fn settle_startup_on_collection_changes(
    startup: StartupTracker,
    vaults: VaultCollectionRuntime,
    registry: VaultRegistryStore,
    model_setup_started: Arc<AtomicBool>,
) -> impl Future<Output = ()> {
    let mut revisions = vaults.subscribe_revisions();
    async move {
        while revisions.changed().await.is_ok() {
            settle_startup(&startup, &vaults, &registry, &model_setup_started);
        }
    }
}

/// Every active Vault's Index turn has *settled*: searchable (`Ready` or
/// `Stale`), or finished with a failure that Vault now reports as its own, or
/// with no local Markdown to index at all. A Vault that failed is settled,
/// not pending: waiting for it to succeed let one broken Vault, or one
/// without a directory, hold the whole instance out of readiness (#326). A
/// collection with no active Vault is settled: nothing is waiting on its
/// first index (#453).
fn collection_indexes_settled(vaults: &VaultCollectionRuntime) -> bool {
    indexing_participants(vaults)
        .iter()
        .all(|participant| participant.settled)
}

/// Every active Vault with its settled state, by the rule
/// [`collection_indexes_settled`] uses, for the startup reading (#373).
fn indexing_participants(vaults: &VaultCollectionRuntime) -> Vec<IndexingParticipant> {
    vaults
        .active_vault_ids()
        .into_iter()
        .map(|vault_id| IndexingParticipant {
            vault_id,
            settled: vaults
                .runtime(vault_id)
                .is_some_and(|runtime| index_settled(&runtime.snapshot())),
        })
        .collect()
}

/// Report one Index turn's progress to the startup tracker, with the whole
/// collection it belongs to, while first-run indexing is still under way.
///
/// The first report of a pass also starts counting the other Vaults' notes,
/// so the reading can weigh the ones still queued. That count runs on its own
/// thread and reads directory entries only: no note content, and none of the
/// Vault's mutation guards, so it cannot block a write or the turn reporting
/// here.
fn report_first_run_progress(
    startup: &StartupTracker,
    vaults: &VaultCollectionRuntime,
    vault_id: VaultId,
    progress: IndexingProgressSnapshot,
) {
    // Routine reindexing after `Ready` is reported per Vault, not here, so it
    // need not read the collection on every progress tick (#326).
    if startup.collection_indexes_ready() {
        return;
    }
    startup.report_indexing_progress(vault_id, progress, indexing_participants(vaults));
    let Some(generation) = startup.claim_note_counts() else {
        return;
    };
    let queued: Vec<(VaultId, PathBuf, Vec<String>)> = vaults
        .active_vault_ids()
        .into_iter()
        .filter(|queued_id| *queued_id != vault_id)
        .filter_map(|queued_id| {
            let runtime = vaults.runtime(queued_id)?;
            Some((
                queued_id,
                runtime.vault_path().to_path_buf(),
                runtime.definition().exclude_patterns().to_vec(),
            ))
        })
        .collect();
    let startup = startup.clone();
    let spawned = std::thread::Builder::new()
        .name("hatchdoor-note-count".to_string())
        .spawn(move || {
            for (queued_id, path, exclude_patterns) in queued {
                let notes = count_markdown_notes(&path, &exclude_patterns);
                startup.record_note_count(generation, queued_id, notes);
            }
        });
    if let Err(error) = spawned {
        // The reading weighs uncounted Vaults at the average instead.
        warn!(%error, "could not start counting queued Vaults' notes");
    }
}

/// An approximate count of the Markdown notes under `root`, honouring the
/// Vault's exclude patterns, or `None` when the directory cannot be read.
/// Walks directory entries only; no file is opened.
fn count_markdown_notes(root: &Path, exclude_patterns: &[String]) -> Option<usize> {
    let exclude = crate::vault::ExcludeMatcher::new(exclude_patterns).ok()?;
    let mut entries = walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            entry.depth() == 0
                || entry.path().strip_prefix(root).map_or(true, |relative| {
                    !exclude.is_excluded(relative, entry.file_type().is_dir())
                })
        });
    // An unreadable root is a Vault that cannot be counted; an unreadable
    // entry deeper down only makes the count approximate.
    if entries.next()?.is_err() {
        return None;
    }
    Some(
        entries
            .filter_map(Result::ok)
            .filter(|entry| {
                entry.file_type().is_file()
                    && entry.path().extension().and_then(|ext| ext.to_str()) == Some("md")
            })
            .count(),
    )
}

fn index_settled(snapshot: &CollectionVaultSnapshot) -> bool {
    matches!(
        snapshot.search,
        VaultSearchStatus::Ready | VaultSearchStatus::Stale
    ) || snapshot.search_error.is_some()
        || snapshot.activation != VaultActivationStatus::Active
}

/// The instance-wide default commit identity for a Git turn, read from the
/// settings snapshot bound to that turn. A Vault's own configured identity
/// still overrides this (see `crate::git::config::resolve_commit_identity`).
fn git_author_defaults(snapshot: &ConfigSnapshot) -> (String, String) {
    (
        crate::git::config::non_empty_setting(snapshot, "HATCHDOOR_GIT_AUTHOR_NAME")
            .unwrap_or_else(|| "Hatchdoor".to_string()),
        crate::git::config::non_empty_setting(snapshot, "HATCHDOOR_GIT_AUTHOR_EMAIL")
            .unwrap_or_else(|| "hatchdoor@localhost".to_string()),
    )
}

/// Publish each Vault's place in the indexing lane on its own status, so the
/// web UI and MCP can tell a Vault that is indexing from one waiting for its
/// turn (ADR-35 decision 5). Every change the lane makes reaches the Vault,
/// whichever producer queued the work.
pub(crate) fn report_index_lane_on_vault_status(
    vaults: &VaultCollectionRuntime,
    work: &VaultWorkCoordinator,
) {
    let vaults = vaults.clone();
    work.observe_index_lane(move |work, vault_id| {
        if let Some(control_block) = vaults.runtime(vault_id) {
            control_block.refresh_index_turn(|| {
                work.index_lane_state(vault_id).map(|state| match state {
                    IndexLaneState::Running => VaultIndexTurn::Running,
                    IndexLaneState::Waiting => VaultIndexTurn::Waiting,
                })
            });
        }
    });
}

/// What an Index turn needs to take turns with other Vaults (ADR-35
/// decision 3): where to ask whether another Vault is waiting, and how long
/// to embed before asking.
pub(crate) struct IndexTurnSlicing {
    pub(crate) work: VaultWorkCoordinator,
    pub(crate) slice: Duration,
}

/// Execute one `VaultWorkKind::Index` turn for exactly one active Vault.
///
/// The authoritative Markdown scan and disposable candidate-cache build run
/// off the async runtime. Publication replaces only this Vault's rows in the
/// shared read model, so readers either retain its prior complete snapshot or
/// observe the new complete snapshot. A failed scan or candidate build keeps a
/// prior snapshot available but marks it stale; without a prior snapshot the
/// Vault remains unavailable for search. A retained snapshot is also marked
/// stale for the duration of the scan/build itself (not just after a
/// failure): collection-shaped reads (`vault_read.rs`'s `collection` helper,
/// `search/vault_scoped.rs`) derive participant freshness solely from this
/// cache-published status, so without this the authoritative Markdown could
/// already differ from a snapshot those reads keep reporting as fresh.
#[cfg(test)]
pub(crate) async fn dispatch_vault_index_turn(
    collection: &VaultCollectionRuntime,
    cache: Arc<SqliteCache>,
    embedder: Arc<dyn Embedder>,
    request: VaultWorkRequest,
) -> Result<(), VaultWorkError> {
    dispatch_vault_index_turn_with_embed_layers(collection, cache, embedder, true, request).await
}

/// Execute one Index turn using the immutable embed-layer setting bound by
/// the executor at the turn's start.
#[cfg(test)]
pub(crate) async fn dispatch_vault_index_turn_with_embed_layers(
    collection: &VaultCollectionRuntime,
    cache: Arc<SqliteCache>,
    embedder: Arc<dyn Embedder>,
    embed_layers: bool,
    request: VaultWorkRequest,
) -> Result<(), VaultWorkError> {
    dispatch_vault_index_turn_with_progress(
        collection,
        cache,
        embedder,
        embed_layers,
        None,
        None,
        request,
    )
    .await
}

/// The Index turn itself. With `slicing`, a turn that has embedded for the
/// slice while another Vault waits stops at the next chunk boundary, puts its
/// Vault at the back of the indexing lane, and returns `Ok`: a pause is not a
/// failure, and the next turn resumes from the progress it saved.
pub(crate) async fn dispatch_vault_index_turn_with_progress(
    collection: &VaultCollectionRuntime,
    cache: Arc<SqliteCache>,
    embedder: Arc<dyn Embedder>,
    embed_layers: bool,
    on_progress: Option<Arc<dyn Fn(crate::startup::IndexingProgressSnapshot) + Send + Sync>>,
    slicing: Option<IndexTurnSlicing>,
    request: VaultWorkRequest,
) -> Result<(), VaultWorkError> {
    let vault_id = request.vault_id();

    // Lifecycle reconstruction queues Index work the moment a Vault activates,
    // which can be well before first-run model setup has downloaded and
    // installed the embedder. Running the turn against an empty embedder slot
    // compares the cache's stored identity against a placeholder — wiping a
    // valid cache and forcing a full reindex on every restart — and then panics
    // in the chunker's tokenizer. Defer instead; the model-load path re-requests
    // every active Vault once the embedder is installed.
    if !embedder.is_ready() {
        return Err(VaultWorkError::new(
            "embedder_not_ready",
            "The search model is still being set up; indexing resumes when setup completes.",
            true,
        ));
    }

    let Some(control_block) = collection.runtime(vault_id) else {
        return Ok(());
    };

    // HTTP and MCP Markdown mutations hold this exact per-Vault guard across
    // their filesystem transaction. Hold it through the authoritative scan and
    // every per-note content read, so an Index turn cannot observe a mixed
    // multi-file foreground mutation — and release it there. The embedding
    // pass that follows opens no Vault path, and on a CPU-only host it runs
    // for minutes: holding the guard across it parked every write behind the
    // turn until the caller's transport gave up on a write that had already
    // landed (issue #223). The turn retakes the guard to publish, and reports
    // the generation stale if a write or a pulling sync landed while it was
    // released. A Git turn that only held the guard does not count (#549).
    #[cfg(test)]
    notify_index_mutation_lock_attempt(vault_id);
    let (read_guard, read_phase_generation) = control_block
        .acquire_mutation_for_index_reads()
        .await
        .map_err(vault_index_error)?;
    // Set inside the publication below, read back here only to report what was
    // published. The verdict itself is decided under the guard that publishes
    // it, never from this flag.
    let published_stale = Arc::new(AtomicBool::new(false));
    let requeue_on = slicing.as_ref().map(|slicing| slicing.work.clone());
    let (result, stale_mark_required) = {
        let _refresh = control_block
            .acquire_refresh()
            .await
            .map_err(vault_index_error)?;
        if let Err(message) = cache.mark_vault_snapshot_stale(vault_id) {
            error!(
                %vault_id,
                %message,
                "failed to mark the retained Vault snapshot stale for an active rebuild"
            );
        }
        // Published after the stale mark, so the status never leaves `Ready`
        // while the snapshot still reads fresh (#483). A mark that failed is
        // logged above and leaves that pair standing until publication.
        let opening_status = opening_search_status(&cache, vault_id);
        control_block
            .set_search_status(opening_status, None)
            .map_err(vault_index_error)?;
        let rebuilds_searchable = opening_status == VaultSearchStatus::Stale;
        let indexing_control = control_block.clone();
        let indexing_cache = cache.clone();
        let publication_stale = published_stale.clone();
        let index_yield = slicing.map(|IndexTurnSlicing { work, slice }| {
            IndexYield::new(
                slice,
                Box::new(move || work.another_vault_waits_to_index(vault_id)),
            )
        });
        match tokio::task::spawn_blocking(move || {
            let index = indexing_control
                .authoritative_index()
                .map_err(|error| (vault_index_error(error), true))?;
            // The scan is what a demo's asset check answers from until the
            // next turn, kept here rather than at publication so it does not
            // wait on the embedding pass (#377).
            indexing_control.retain_indexed_assets(&index);
            // Publish this Vault's structural rows before its vectors, so a
            // first index makes it browsable in seconds instead of holding
            // every read behind the embedding pass. A no-op for a Vault that
            // already has a searchable generation to keep serving.
            match indexing_cache.publish_vault_structure_snapshot(
                vault_id,
                &index,
                embedder.as_ref(),
                embed_layers,
            ) {
                Ok(true) => {
                    let _ = indexing_control.set_search_status(VaultSearchStatus::Browsable, None);
                }
                Ok(false) => {}
                // Browsing early is an improvement, not a precondition: a
                // failed structure pass falls through to the full build
                // rather than failing the turn.
                Err(message) => warn!(
                    %vault_id,
                    %message,
                    "could not publish the structure-only Vault snapshot; browsing waits for the full index"
                ),
            }
            // The structure pass is where a changed search model wipes the
            // cache. If the generation this turn opened on is gone and
            // nothing replaced it, stop reporting it searchable.
            if rebuilds_searchable
                && opening_search_status(&indexing_cache, vault_id) != VaultSearchStatus::Stale
                && indexing_control.snapshot().search == VaultSearchStatus::Stale
            {
                let _ = indexing_control.set_search_status(VaultSearchStatus::Indexing, None);
            }
            let publication_control = indexing_control.clone();
            indexing_cache
                .replace_vault_snapshot_with_embed_layers_and_progress(
                    vault_id,
                    &index,
                    embedder.as_ref(),
                    embed_layers,
                    on_progress,
                    Some(MutationGuardHandoff {
                        read_phase: read_guard,
                        freshness_at_publication: Box::new(move || {
                            let (mutated, guard) = publication_control
                                .blocking_retake_mutation_for_index(read_phase_generation);
                            publication_stale.store(mutated, Ordering::Release);
                            // A rebuild reports `Ready` ahead of the row that
                            // makes it true, under the guard that row is
                            // published under, so no reader finds a fresh
                            // snapshot on a Vault still reporting `Stale`
                            // (#483). A first build waits for its vectors: it
                            // has no search capability to keep until then.
                            if rebuilds_searchable && !mutated {
                                let _ = publication_control
                                    .set_search_status(VaultSearchStatus::Ready, None);
                            }
                            let freshness = if mutated {
                                // A write or a pulling sync landed while this
                                // turn was embedding, so what is about to be
                                // published is already behind the Markdown.
                                // Search keeps answering from it; the label is
                                // what stops it claiming to be current. The
                                // catch-up turn is armed by the watcher for a
                                // write and by the sync for its own pull.
                                VaultSnapshotFreshness::Stale
                            } else {
                                VaultSnapshotFreshness::Fresh
                            };
                            (freshness, guard)
                        }),
                    }),
                    index_yield.as_ref(),
                )
                .map_err(|message| {
                    (
                        VaultWorkError::new("vault_index_failed", message, true),
                        false,
                    )
                })
                .and_then(|publication| match publication {
                    SnapshotPublication::Published => Ok(IndexTurnEnd::Published),
                    SnapshotPublication::Paused => Ok(IndexTurnEnd::Paused),
                    // A newer snapshot attempt owns this Vault's row now, so
                    // this turn wrote nothing and must not report the Vault
                    // current. That attempt decides the row's freshness,
                    // which is why no stale mark follows.
                    SnapshotPublication::Superseded => Err((
                        VaultWorkError::new(
                            "vault_index_failed",
                            "a newer snapshot attempt superseded this Index turn before it \
                             published, so nothing was published",
                            true,
                        ),
                        false,
                    )),
                })
        })
        .await
        {
            Ok(Ok(end)) => (Ok(end), false),
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
        Ok(IndexTurnEnd::Paused) => {
            // Behind every Vault already waiting, so the one this turn paused
            // for goes first. A drained Vault is not put back: disabling or
            // removing it discards its paused work with the rest.
            if let Some(work) = requeue_on {
                work.requeue_paused_index_turn(vault_id);
            }
            info!(%vault_id, "Vault indexing paused so another Vault can index; it resumes when its turn comes round");
            // Nothing failed, so no error rides along: the Vault shows what
            // its retained generation supports, as an unfinished rebuild does.
            let _ = control_block.set_search_status(retained_search_status(&cache, vault_id), None);
        }
        Ok(IndexTurnEnd::Published) => {
            // A generation published stale reports itself stale here too, which
            // is exactly what `retained_snapshot_search_status` would derive
            // from the same row after a restart. `Stale` still grants the
            // search capability, so the Vault keeps answering; the turn that
            // makes it `Ready` is armed by whatever rewrote the Markdown.
            let status = if published_stale.load(Ordering::Acquire) {
                VaultSearchStatus::Stale
            } else {
                VaultSearchStatus::Ready
            };
            let _ = control_block.set_search_status(status, None);
        }
        Err(error) => publish_index_failure(&control_block, &cache, error, stale_mark_required),
    }
    result.map(|_| ())
}

/// How an Index turn that did not fail ended.
enum IndexTurnEnd {
    Published,
    Paused,
}

/// The search status a Vault's retained generation supports on its own,
/// with no Index turn running: searchable, browsable, or nothing.
///
/// A structure pass that succeeded before the embedding pass stopped leaves a
/// participating generation with no vectors. Reporting it `Stale` would grant
/// the search capability to a Vault that can only ever answer with nothing,
/// so the vectorless axis wins here exactly as it does in
/// `retained_snapshot_search_status`.
fn retained_search_status(cache: &SqliteCache, vault_id: VaultId) -> VaultSearchStatus {
    match cache.snapshot_status(vault_id) {
        Ok(Some(snapshot)) if snapshot.participating && snapshot.searchable => {
            VaultSearchStatus::Stale
        }
        Ok(Some(snapshot)) if snapshot.participating => VaultSearchStatus::Browsable,
        Ok(Some(_)) | Ok(None) | Err(_) => VaultSearchStatus::Unavailable,
    }
}

/// The search status an Index turn publishes as it starts. A Vault that
/// already answers search keeps answering from its retained generation for
/// the whole rebuild, so it reports `Stale` and keeps the search capability
/// (ADR-35 decision 2, #483). `Indexing` is for a Vault with nothing
/// searchable yet, a vectorless generation included.
fn opening_search_status(cache: &SqliteCache, vault_id: VaultId) -> VaultSearchStatus {
    match retained_search_status(cache, vault_id) {
        VaultSearchStatus::Stale => VaultSearchStatus::Stale,
        VaultSearchStatus::Browsable
        | VaultSearchStatus::Unavailable
        | VaultSearchStatus::Indexing
        | VaultSearchStatus::Ready => VaultSearchStatus::Indexing,
    }
}

/// Publish a failed Index turn's search status on its Vault: whatever the
/// Vault's retained generation still supports, with the failure attached.
fn publish_index_failure(
    control_block: &VaultControlBlock,
    cache: &SqliteCache,
    error: &VaultWorkError,
    stale_mark_required: bool,
) {
    let vault_id = control_block.definition().vault_id();
    let stale_mark_error = stale_mark_required
        .then(|| cache.mark_vault_snapshot_stale(vault_id))
        .transpose()
        .err();
    // The failure is not lost: it rides along as this status's error.
    let status = retained_search_status(cache, vault_id);
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
            detail: None,
        }),
    );
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

/// One Git turn's source-specific parts, resolved from a Vault's definition
/// before the shared turn shell below runs it.
///
/// The three source kinds that have a Git turn differ only in these fields
/// and in the blocking function they close over; everything else — the
/// mutation-lock hold, `spawn_blocking`, panic mapping, outcome publication —
/// is the shell's, once (issue #128).
struct GitTurnPlan<T = ManagedGitOutcome> {
    /// Whether the turn runs under the Vault's foreground mutation lock. Only
    /// `LocalHistory` runs without it: it commits already-settled drift in
    /// the working tree and never checks out, resets, or merges over a
    /// concurrent foreground write.
    holds_mutation_lock: bool,
    /// The error code a panic inside the blocking work is reported as.
    panic_code: &'static str,
    /// The blocking `git2` work itself, run off the async runtime.
    work: GitTurnWork<T>,
}

/// The blocking half of a Git turn, and whether it needs this Vault's checkout
/// lease. The two shapes are distinct types rather than one closure taking an
/// `Option<&ManagedCheckoutLease>`, so a turn that needs a lease cannot be
/// built — or run — without one.
enum GitTurnWork<T> {
    /// Runs against a checkout Hatchdoor does not own: an `ExistingGit`
    /// Vault's own working copy, in either remote-sync or Local-history mode.
    /// See `run_existing_git_remote_turn` for why `ManagedCheckoutLease` does
    /// not apply to an operator-owned checkout.
    Unleased(Box<dyn FnOnce() -> Result<T, VaultWorkError> + Send + 'static>),
    /// Runs against the managed checkout under `state_directory`, holding that
    /// Vault's lease for the whole turn (issue #95).
    #[allow(clippy::type_complexity)]
    Leased {
        state_directory: PathBuf,
        run: Box<dyn FnOnce(&ManagedCheckoutLease) -> Result<T, VaultWorkError> + Send + 'static>,
    },
}

/// [`GitTurnWork`] with its checkout lease, if any, already acquired — the
/// state between "the lease is obtained" and "the mutation lock is taken",
/// which is the order those two must always be acquired in.
enum PreparedGitTurn<T> {
    Unleased(Box<dyn FnOnce() -> Result<T, VaultWorkError> + Send + 'static>),
    #[allow(clippy::type_complexity)]
    Leased {
        lease: ManagedCheckoutLease,
        run: Box<dyn FnOnce(&ManagedCheckoutLease) -> Result<T, VaultWorkError> + Send + 'static>,
    },
}

/// Execute one `VaultWorkKind::Git` turn for `request` and publish its result
/// through [`publish_managed_git_turn_outcome`], which is where what gets
/// published, and why, is documented.
///
/// A no-op returning `Ok(())` if the Vault has since been retired (its
/// runtime is gone) or has no Git turn at all (a `Local` source).
///
/// `author_name`/`author_email` are the instance-wide default commit
/// identity; the Vault's own configured identity, if any, overrides them
/// (see [`crate::git::config::resolve_commit_identity`]).
pub(crate) async fn dispatch_git_turn(
    collection: &VaultCollectionRuntime,
    registry: &VaultRegistryStore,
    coordinator: &VaultWorkCoordinator,
    managed_git: &ManagedGitScheduler,
    author_name: &str,
    author_email: &str,
    request: VaultWorkRequest,
) -> Result<(), VaultWorkError> {
    dispatch_git_turn_with(
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

/// [`dispatch_git_turn`] with the actual managed-Git `git2` turn
/// injectable: `execute` is production's `run_managed_git_turn` in the real
/// dispatch loop, and a deterministic fake in tests that need to drive a real
/// failure through the full async path (credential resolution,
/// `spawn_blocking`, status publishing, scheduler recording) without a
/// reachable remote. Only the managed-Git source kind routes through it; the
/// two `ExistingGit` paths always run their own real turn.
#[allow(clippy::too_many_arguments)] // Production arguments plus the test-only executor.
async fn dispatch_git_turn_with<F>(
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
            &crate::git::WriteLedger,
        ) -> Result<ManagedGitOutcome, VaultWorkError>
        + Send
        + 'static,
{
    let vault_id = request.vault_id();
    let Some(control_block) = collection.runtime(vault_id) else {
        managed_git.deactivate(vault_id);
        return Ok(());
    };
    // The Vault's own configured commit identity, if any, overrides the
    // server-wide defaults for every source kind below (#130).
    let (author_name, author_email) = crate::git::config::resolve_commit_identity(
        control_block.definition().commit_identity(),
        author_name,
        author_email,
    );

    let plan = match plan_git_turn(
        &control_block,
        registry,
        vault_id,
        author_name,
        author_email,
        execute,
    ) {
        // `Local` has no Git turn at all.
        Ok(None) => return Ok(()),
        Ok(Some(plan)) => plan,
        Err(error) => {
            return finish_git_turn(
                &control_block,
                coordinator,
                managed_git,
                vault_id,
                Err(error),
            );
        }
    };

    // `Synchronized` covers every sync that pulled, and `finish_git_turn`
    // answers every success with an Index request, which is the catch-up turn
    // the advance owes. A sync that only pushed is counted too and costs one
    // Index turn that finds nothing changed. A failed sync is not counted:
    // nothing queues an Index turn after it, so a stale verdict would stand
    // with nothing behind it (#549). Where a failed sync did rewrite a note
    // (a merge that landed before its push was refused), the watcher reports
    // the changed files as it would any edit made outside Hatchdoor.
    let result = run_planned_turn(&control_block, managed_git, vault_id, plan, |result| {
        matches!(result, Ok(ManagedGitOutcome::Synchronized))
    })
    .await;
    finish_git_turn(&control_block, coordinator, managed_git, vault_id, result)
}

/// Run one already-planned Git or commit turn: obtain the checkout lease if
/// the plan needs one, take this Vault's mutation lock if the plan holds it,
/// run the blocking `git2` work off the async runtime, and hand the lease
/// back. Publication is the caller's, because a Git turn and a commit turn
/// conclude different things from the same result.
///
/// `rewrote_markdown` reads the finished work's result and answers whether it
/// rewrote the Vault's working-tree Markdown. Only then does the turn advance
/// the mutation generation an overlapping Index turn compares, and the caller
/// must then request the Index turn that catches up (#549).
async fn run_planned_turn<T: Send + 'static>(
    control_block: &VaultControlBlock,
    managed_git: &ManagedGitScheduler,
    vault_id: VaultId,
    plan: GitTurnPlan<T>,
    rewrote_markdown: fn(&Result<T, VaultWorkError>) -> bool,
) -> Result<T, VaultWorkError> {
    // Obtain this Vault's checkout lease — reused from a previous turn if
    // `ManagedGitScheduler` is already holding one, or freshly acquired
    // otherwise (only the first turn since activation pays that one-time,
    // local-filesystem-only cost; see
    // `ManagedGitScheduler::take_or_acquire_checkout_lease`). Extracted
    // *before* `spawn_blocking` — an owned `ManagedCheckoutLease` has no
    // lifetime tied to `managed_git`, so it can move into the blocking
    // closure below without borrowing `managed_git` there, which
    // `spawn_blocking`'s `'static` bound would otherwise forbid.
    let prepared = match plan.work {
        GitTurnWork::Unleased(run) => PreparedGitTurn::Unleased(run),
        GitTurnWork::Leased {
            state_directory,
            run,
        } => match managed_git.take_or_acquire_checkout_lease(state_directory, vault_id) {
            Ok(lease) => PreparedGitTurn::Leased { lease, run },
            Err(error) => {
                return Err(crate::git::managed_task::classify_checkout_error(error));
            }
        },
    };

    // Hold the same per-Vault mutation lock a foreground Markdown write
    // acquires (`handlers::vault_write`/`mcp::tools::write`'s own
    // `acquire_mutation`) across this turn's blocking `git2` work (issue
    // #96's reopening defect 2): without it, a write could land mid-merge,
    // or this turn's checkout/reset could stomp a write mid-flight. Taken
    // without the generation advance a write makes; see `rewrote_markdown`.
    // Acquired *after* the checkout lease so a lease-acquisition failure
    // above never blocks on it; the two locks are always acquired in this
    // same order for the same Vault, and nothing else in this codebase ever
    // acquires the checkout lease, so there is no risk of the mutation lock
    // and the checkout lease being acquired in opposite orders elsewhere.
    // Coarser than the retired single-Vault path's fine-grained per-phase
    // locking, which released its lock across the network-only fetch/push
    // phases (deleted with that lane in #185) — held for this whole turn
    // instead, including `synchronize_managed_checkout`'s network round-trip.
    // Reproducing the fine-grained scheme here would require splitting
    // `synchronize_managed_checkout`'s monolithic fetch+integrate+push call
    // into phases callable independently from this async dispatch layer, a
    // substantially larger change than issue #96's fix warranted on its own.
    let mut mutation_guard = None;
    if plan.holds_mutation_lock {
        match control_block.acquire_mutation_for_git_turn().await {
            Ok(guard) => mutation_guard = Some(guard),
            Err(error) => return Err(managed_git_mutation_error(error)),
        }
    }

    // The lease travels into the blocking task and back out again — it is
    // never dropped here, only borrowed by `run` — so the scheduler can hand
    // it back to `keep_checkout_lease` afterward and keep holding it across
    // turns instead of releasing its OS-level lock at the end of this one
    // (issue #95).
    let panic_code = plan.panic_code;
    let finished = tokio::task::spawn_blocking(move || match prepared {
        PreparedGitTurn::Unleased(run) => (run(), None),
        PreparedGitTurn::Leased { lease, run } => {
            let result = run(&lease);
            (result, Some(lease))
        }
    })
    .await;
    if let (Some(guard), Ok((result, _))) = (&mutation_guard, &finished)
        && rewrote_markdown(result)
    {
        control_block.record_markdown_rewrite(guard);
    }
    drop(mutation_guard);
    let (result, lease) = match finished {
        Ok((result, lease)) => (result, lease),
        Err(join_error) => (
            Err(VaultWorkError::new(
                panic_code,
                join_error.to_string(),
                false,
            )),
            // The panicking task owned the lease; it was dropped (releasing
            // the OS lock) during unwinding, so there is nothing to keep.
            None,
        ),
    };
    if let Some(lease) = lease {
        managed_git.keep_checkout_lease(vault_id, lease);
    }
    result
}

/// Execute one `VaultWorkKind::Commit` turn for `request`: commit whatever
/// has changed in this Vault's own subtree, and stop there.
///
/// A commit is local, costs nothing but disk, and cannot fail for a reason
/// outside this machine, which is why the watcher can ask for one on every
/// change. Talking to a remote is the opposite on all three counts and stays
/// on the Vault's configured sync schedule, in `dispatch_git_turn` (#267).
///
/// A no-op returning `Ok(())` when the Vault has since been retired or its
/// mode makes no local commits.
pub(crate) async fn dispatch_commit_turn(
    collection: &VaultCollectionRuntime,
    registry: &VaultRegistryStore,
    managed_git: &ManagedGitScheduler,
    commit_cooldown: &CommitCooldown,
    author_name: &str,
    author_email: &str,
    request: VaultWorkRequest,
) -> Result<(), VaultWorkError> {
    let vault_id = request.vault_id();
    let Some(control_block) = collection.runtime(vault_id) else {
        return Ok(());
    };
    let (author_name, author_email) = crate::git::config::resolve_commit_identity(
        control_block.definition().commit_identity(),
        author_name,
        author_email,
    );
    let Some(plan) = plan_commit_turn(
        &control_block,
        registry,
        vault_id,
        author_name,
        author_email,
    ) else {
        return Ok(());
    };
    // A commit stages and commits what is already on disk. It moves `HEAD`
    // and never writes the working tree, so no Index turn is behind it.
    let result = run_planned_turn(&control_block, managed_git, vault_id, plan, |_| false).await;
    finish_commit_turn(&control_block, commit_cooldown, vault_id, result)
}

/// Execute one `VaultWorkKind::Recovery` turn: publish this Vault's side of
/// a sync conflict to its recovery branch, and report the outcome on the
/// Vault's `recovery_branch` status (ADR-30).
///
/// Runs through the same shell as every Git turn, so it holds the checkout
/// lease and the Vault's mutation lock and never overlaps a sync or commit.
/// It publishes nothing to the Vault's Git status and does not feed the
/// managed-Git scheduler: a publish is not a check of the remote, and the
/// conflict it is about stays the Vault's failure until a sync resolves it.
///
/// A no-op returning `Ok(())` when the Vault has since been retired or has
/// no recovery branch to publish.
pub(crate) async fn dispatch_recovery_turn(
    collection: &VaultCollectionRuntime,
    registry: &VaultRegistryStore,
    managed_git: &ManagedGitScheduler,
    author_name: &str,
    author_email: &str,
    request: VaultWorkRequest,
) -> Result<(), VaultWorkError> {
    let vault_id = request.vault_id();
    let Some(control_block) = collection.runtime(vault_id) else {
        return Ok(());
    };
    // Admission checked this too, but a sync that ran while the request
    // waited may have resolved the conflict or changed it.
    if !control_block.snapshot().capabilities.publish_recovery {
        return finish_recovery_turn(
            &control_block,
            vault_id,
            Err(VaultWorkError::new(
                "capability_unavailable",
                "This Vault's sync is no longer stopped on a conflict, so there is nothing to \
                 publish",
                false,
            )
            .into()),
        );
    }
    let (author_name, author_email) = crate::git::config::resolve_commit_identity(
        control_block.definition().commit_identity(),
        author_name,
        author_email,
    );
    let plan = match plan_recovery_turn(
        &control_block,
        registry,
        vault_id,
        author_name,
        author_email,
    ) {
        Ok(Some(plan)) => plan,
        Ok(None) => return Ok(()),
        Err(error) => return finish_recovery_turn(&control_block, vault_id, Err(error.into())),
    };
    // Publishing commits pending drift and pushes one branch. Like a commit
    // turn it leaves the working tree as it found it.
    let result = run_planned_turn(&control_block, managed_git, vault_id, plan, |_| false)
        .await
        .unwrap_or_else(|error| Err(error.into()));
    finish_recovery_turn(&control_block, vault_id, result)
}

/// Resolve the source-specific parts of one recovery turn, or `Ok(None)` for
/// a Vault with no recovery branch: anything that is not Two-way.
fn plan_recovery_turn(
    control_block: &VaultControlBlock,
    registry: &VaultRegistryStore,
    vault_id: VaultId,
    author_name: String,
    author_email: String,
) -> Result<Option<GitTurnPlan<RecoveryResult>>, VaultWorkError> {
    let write_ledger = control_block.write_ledger();
    match control_block.definition().source() {
        RegistryVaultSource::ExistingGit {
            mode: VaultGitMode::TwoWay,
            repository_path,
            repository_url,
            branch,
            ..
        } => {
            let repository_path = repository_path.clone();
            let repository_url = repository_url.clone();
            let branch = branch.clone();
            let vault_path = control_block.vault_path().to_path_buf();
            let credentials = git_credentials(registry, vault_id)?;
            Ok(Some(GitTurnPlan {
                holds_mutation_lock: true,
                panic_code: "existing_git_recovery_task_panicked",
                work: GitTurnWork::Unleased(Box::new(move || {
                    Ok(run_existing_git_recovery_turn(
                        repository_path,
                        vault_path,
                        repository_url,
                        branch,
                        credentials,
                        author_name,
                        author_email,
                        vault_id,
                        &write_ledger,
                    ))
                })),
            }))
        }
        RegistryVaultSource::ManagedGit {
            repository_url,
            branch,
            vault_subdirectory,
            mode: VaultGitMode::TwoWay,
            poll_interval_secs: _,
        } => {
            let credentials = git_credentials(registry, vault_id)?;
            let state_directory = managed_state_directory(registry);
            let config = ManagedGitTurnConfig {
                vault_id,
                state_directory: state_directory.clone(),
                repository_url: repository_url.clone(),
                branch: branch.clone(),
                vault_subdirectory: vault_subdirectory.clone(),
                mode: VaultGitMode::TwoWay,
                credentials,
                author_name,
                author_email,
            };
            Ok(Some(GitTurnPlan {
                holds_mutation_lock: true,
                panic_code: "managed_git_recovery_task_panicked",
                work: GitTurnWork::Leased {
                    state_directory,
                    run: Box::new(move |lease| {
                        Ok(run_managed_recovery_turn(&config, lease, &write_ledger))
                    }),
                },
            }))
        }
        RegistryVaultSource::ExistingGit { .. }
        | RegistryVaultSource::ManagedGit { .. }
        | RegistryVaultSource::Local { .. } => Ok(None),
    }
}

/// Publish one recovery turn's outcome on the Vault's `recovery_branch`
/// status. A refusal keeps the earlier publication's fields when it was for
/// the same branch, because that branch still stands on the remote.
fn finish_recovery_turn(
    control_block: &VaultControlBlock,
    vault_id: VaultId,
    result: RecoveryResult,
) -> Result<(), VaultWorkError> {
    match result {
        Ok(publication) => {
            info!(
                %vault_id,
                branch = %publication.branch,
                commit = %publication.published_commit,
                "Vault recovery branch published"
            );
            let _ = control_block.set_recovery_branch(Some(RecoveryBranchStatus {
                branch: Some(publication.branch),
                published_commit: Some(publication.published_commit),
                conflicting_commit: publication.conflicting_commit,
                published_at: Some(format_timestamp(std::time::SystemTime::now())),
                error: None,
            }));
            Ok(())
        }
        Err(RecoveryFailure { branch, error }) => {
            let mut status = control_block
                .snapshot()
                .recovery_branch
                .filter(|previous| branch.is_none() || previous.branch == branch)
                .unwrap_or_default();
            if branch.is_some() {
                status.branch = branch;
            }
            status.error = Some(VaultRuntimeError {
                code: error.code().to_string(),
                message: error.message().to_string(),
                retryable: error.retryable(),
                detail: error.detail().map(VaultRuntimeErrorDetail::from),
            });
            let _ = control_block.set_recovery_branch(Some(status));
            Err(error)
        }
    }
}

/// Resolve the source-specific parts of one commit turn, or `None` when this
/// Vault's mode makes no local commits: a Pull-only Vault, which refuses
/// writes and must leave its operator's own drift alone, or a plain local
/// folder, which has no Git at all.
fn plan_commit_turn(
    control_block: &VaultControlBlock,
    registry: &VaultRegistryStore,
    vault_id: VaultId,
    author_name: String,
    author_email: String,
) -> Option<GitTurnPlan> {
    let write_ledger = control_block.write_ledger();
    match control_block.definition().source() {
        // Identical to the Local-history arm of `plan_git_turn`, because for
        // a Vault with no remote the Git turn always was a commit and
        // nothing else. It runs without the mutation lock for the reason
        // documented on `GitTurnPlan::holds_mutation_lock`: it commits
        // already-settled drift and must not park foreground writes behind
        // itself.
        RegistryVaultSource::ExistingGit {
            mode: VaultGitMode::LocalHistory,
            ..
        } => {
            let vault_path = control_block.vault_path().to_path_buf();
            Some(GitTurnPlan {
                holds_mutation_lock: false,
                panic_code: "existing_git_local_history_task_panicked",
                work: GitTurnWork::Unleased(Box::new(move || {
                    crate::git::run_local_history_git_turn(
                        vault_path,
                        author_name,
                        author_email,
                        &write_ledger,
                    )
                })),
            })
        }
        // Two-way against the operator's own checkout. Unlike Local history
        // this one does hold the mutation lock: `prepare_two_way_worktree`
        // reads the whole checkout's status and stages from it, so a write
        // landing mid-stage would be committed half-applied.
        RegistryVaultSource::ExistingGit {
            mode: VaultGitMode::TwoWay,
            repository_path,
            branch,
            ..
        } => {
            let repository_path = repository_path.clone();
            let branch = branch.clone();
            let vault_path = control_block.vault_path().to_path_buf();
            Some(GitTurnPlan {
                holds_mutation_lock: true,
                panic_code: "existing_git_commit_task_panicked",
                work: GitTurnWork::Unleased(Box::new(move || {
                    run_existing_git_commit_turn(
                        repository_path,
                        vault_path,
                        branch,
                        VaultGitMode::TwoWay,
                        author_name,
                        author_email,
                        &write_ledger,
                    )
                })),
            })
        }
        // Two-way against the managed checkout. Takes the same lease a sync
        // turn takes, and no credentials: `run_managed_git_commit_turn`
        // reuses the checkout that is already there rather than cloning one,
        // so there is nothing to authenticate against.
        RegistryVaultSource::ManagedGit {
            repository_url,
            branch,
            vault_subdirectory,
            mode: VaultGitMode::TwoWay,
            poll_interval_secs: _,
        } => {
            let state_directory = managed_state_directory(registry);
            let config = ManagedGitTurnConfig {
                vault_id,
                state_directory: state_directory.clone(),
                repository_url: repository_url.clone(),
                branch: branch.clone(),
                vault_subdirectory: vault_subdirectory.clone(),
                mode: VaultGitMode::TwoWay,
                credentials: None,
                author_name,
                author_email,
            };
            Some(GitTurnPlan {
                holds_mutation_lock: true,
                panic_code: "managed_git_commit_task_panicked",
                work: GitTurnWork::Leased {
                    state_directory,
                    run: Box::new(move |lease| {
                        run_managed_git_commit_turn(&config, lease, &write_ledger)
                    }),
                },
            })
        }
        RegistryVaultSource::ExistingGit {
            mode: VaultGitMode::PullOnly,
            ..
        }
        | RegistryVaultSource::ManagedGit {
            mode: VaultGitMode::PullOnly | VaultGitMode::LocalHistory,
            ..
        }
        | RegistryVaultSource::Local { .. } => None,
    }
}

/// Publish one commit turn's outcome, and arm or clear this Vault's commit
/// cooldown.
///
/// Deliberately *not* [`finish_git_turn`], on three counts. It does not feed
/// the managed-Git scheduler, because a commit is not a check of the remote
/// and must not move the schedule that governs one. It does not request an
/// Index turn, because the watcher change that asked for this commit already
/// requested one, which is also what keeps a Vault whose Git is broken
/// indexing normally. And a failure arms the cooldown, because every way a
/// commit can fail needs a human, and without it a Vault in that state would
/// fail a turn on every save.
fn finish_commit_turn(
    control_block: &VaultControlBlock,
    commit_cooldown: &CommitCooldown,
    vault_id: VaultId,
    result: Result<ManagedGitOutcome, VaultWorkError>,
) -> Result<(), VaultWorkError> {
    match &result {
        Ok(outcome) => {
            info!(%vault_id, ?outcome, "Vault Git commit turn completed");
            commit_cooldown.clear(vault_id);
            // A commit proves the local half of Git healthy and nothing
            // about the remote. A standing remote-only failure (a conflict
            // with the remote, a refused push) stays published: syncs run
            // on the poll interval, a day by default, and a commit fires on
            // every save, so clearing it here would hide the one failure
            // that needs a human almost as soon as it appeared (#323).
            // Anything else is a failure this turn just disproved.
            let remote_failure_stands = control_block
                .snapshot()
                .git_error
                .is_some_and(|error| crate::git::managed_task::is_remote_only_failure(&error.code));
            if !remote_failure_stands {
                let _ = control_block.set_git_status(VaultGitStatus::Ready, None);
            }
        }
        Err(error) => {
            commit_cooldown.arm(vault_id);
            let _ = control_block.set_git_status(
                VaultGitStatus::Unavailable,
                Some(VaultRuntimeError {
                    code: error.code().to_string(),
                    message: error.message().to_string(),
                    retryable: error.retryable(),
                    detail: error.detail().map(VaultRuntimeErrorDetail::from),
                }),
            );
        }
    }
    result.map(|_| ())
}

/// Resolve the source-specific parts of one Git turn, or `Ok(None)` when this
/// Vault's source has no Git turn. An `Err` is a failure that must still be
/// published through the shared outcome path (a credential read that could
/// not reach the registry).
fn plan_git_turn<F>(
    control_block: &VaultControlBlock,
    registry: &VaultRegistryStore,
    vault_id: VaultId,
    author_name: String,
    author_email: String,
    execute: F,
) -> Result<Option<GitTurnPlan>, VaultWorkError>
where
    F: FnOnce(
            &ManagedGitTurnConfig,
            &ManagedCheckoutLease,
            &crate::git::WriteLedger,
        ) -> Result<ManagedGitOutcome, VaultWorkError>
        + Send
        + 'static,
{
    // Every Git turn that can commit names its commit from this Vault's
    // pending write records (#249). Each branch that needs it takes its own
    // clone to move into its blocking closure.
    match control_block.definition().source() {
        // An existing checkout under Local-history versioning has no remote
        // to sync, so its Git turn is its commit turn and nothing else, with
        // one implementation, in `plan_commit_turn`. Reachable only if
        // something still asks for `VaultWorkKind::Git` on such a Vault;
        // since #267 activation, the watcher and a manual control all ask
        // for `VaultWorkKind::Commit` instead.
        RegistryVaultSource::ExistingGit {
            mode: VaultGitMode::LocalHistory,
            ..
        } => Ok(plan_commit_turn(
            control_block,
            registry,
            vault_id,
            author_name,
            author_email,
        )),
        // An existing checkout under Pull-only or Two-way versioning is
        // remote sync against the checkout that already exists at
        // `repository_path` — no managed-checkout acquisition or lease: see
        // `run_existing_git_remote_turn`'s doc comment for why
        // `ManagedCheckoutLease` does not apply to an `ExistingGit` source.
        // Holds the same per-Vault mutation lock a managed-Git turn holds
        // (defect 2 of issue #96's reopening): without it, a foreground
        // Markdown write could race this turn's fetch/integrate/reset phases.
        RegistryVaultSource::ExistingGit {
            mode: existing_mode @ (VaultGitMode::PullOnly | VaultGitMode::TwoWay),
            repository_path,
            repository_url,
            branch,
            ..
        } => {
            let repository_path = repository_path.clone();
            let repository_url = repository_url.clone();
            let vault_path = control_block.vault_path().to_path_buf();
            let branch = branch.clone();
            let mode = *existing_mode;
            let credentials = git_credentials(registry, vault_id)?;
            let write_ledger = control_block.write_ledger();
            Ok(Some(GitTurnPlan {
                holds_mutation_lock: true,
                panic_code: "existing_git_remote_task_panicked",
                work: GitTurnWork::Unleased(Box::new(move || {
                    run_existing_git_remote_turn(
                        repository_path,
                        vault_path,
                        repository_url,
                        branch,
                        mode,
                        credentials,
                        author_name,
                        author_email,
                        &write_ledger,
                    )
                })),
            }))
        }
        RegistryVaultSource::ManagedGit {
            repository_url,
            branch,
            vault_subdirectory,
            mode,
            poll_interval_secs: _,
        } => {
            let credentials = git_credentials(registry, vault_id)?;
            let write_ledger = control_block.write_ledger();
            let state_directory = managed_state_directory(registry);
            let config = ManagedGitTurnConfig {
                vault_id,
                state_directory: state_directory.clone(),
                repository_url: repository_url.clone(),
                branch: branch.clone(),
                vault_subdirectory: vault_subdirectory.clone(),
                mode: *mode,
                credentials,
                author_name,
                author_email,
            };
            Ok(Some(GitTurnPlan {
                holds_mutation_lock: true,
                panic_code: "managed_git_task_panicked",
                work: GitTurnWork::Leased {
                    state_directory,
                    run: Box::new(move |lease| execute(&config, lease, &write_ledger)),
                },
            }))
        }
        // `Local` has no Git turn at all.
        RegistryVaultSource::Local { .. } => Ok(None),
    }
}

/// The directory managed checkouts live under: the registry file's own.
fn managed_state_directory(registry: &VaultRegistryStore) -> PathBuf {
    registry
        .path()
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

/// Read a Vault's stored HTTPS credentials, mapping an unreachable registry
/// into the retryable failure a Git turn reports for it.
///
/// The client-visible message is fixed: `VaultRegistryError`'s text embeds
/// the registry file's absolute host path, and this failure becomes the
/// Vault's published `git_error` (#323). The full error goes to the
/// operator's log instead.
fn git_credentials(
    registry: &VaultRegistryStore,
    vault_id: VaultId,
) -> Result<Option<crate::vault_registry::HttpsCredentials>, VaultWorkError> {
    registry.https_credentials(vault_id).map_err(|error| {
        tracing::warn!(%vault_id, %error, "Vault registry unavailable for a Git turn");
        VaultWorkError::new(
            "managed_git_registry_unavailable",
            "Hatchdoor could not read this Vault's stored Git settings",
            true,
        )
    })
}

/// Publish one Git turn's outcome and reduce it to the dispatch loop's
/// `Result<(), _>`. Every exit from [`dispatch_git_turn_with`] that
/// has a result to report goes through here, so no source kind can publish
/// differently from another.
fn finish_git_turn(
    control_block: &VaultControlBlock,
    coordinator: &VaultWorkCoordinator,
    managed_git: &ManagedGitScheduler,
    vault_id: VaultId,
    result: Result<ManagedGitOutcome, VaultWorkError>,
) -> Result<(), VaultWorkError> {
    publish_managed_git_turn_outcome(control_block, coordinator, managed_git, vault_id, &result);
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
/// Separated from [`dispatch_git_turn`] so this — the interesting
/// behavior — is testable against a fabricated result, without needing a
/// real `git2` clone/fetch against a reachable remote.
fn publish_managed_git_turn_outcome(
    control_block: &VaultControlBlock,
    coordinator: &VaultWorkCoordinator,
    managed_git: &ManagedGitScheduler,
    vault_id: VaultId,
    result: &Result<ManagedGitOutcome, VaultWorkError>,
) {
    match result {
        Ok(outcome) => {
            // One line per completed poll, so an operator can tell a Vault
            // that polled and found nothing from one that is not polling at
            // all. A Git turn's only other trace is a remote-side ref
            // update, which a fetch that brought nothing new never writes —
            // leaving `git reflog` unable to answer "is this Vault still on
            // its schedule?" Failures already carry their own `warn!` (see
            // `VaultWorkExecutor::publish_outcome`) plus per-Vault status.
            info!(%vault_id, ?outcome, "Vault Git turn completed");
            let _ = control_block.set_git_status(VaultGitStatus::Ready, None);
            // A sync that went through means the conflict a recovery branch
            // was published for is resolved. The branch stays on the remote
            // (ADR-30); only this Vault's report of it is done.
            let _ = control_block.set_recovery_branch(None);
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
                    detail: error.detail().map(VaultRuntimeErrorDetail::from),
                }),
            );
        }
    }
    managed_git.record_outcome(vault_id, result);
}

/// Re-derive and publish local-content availability after a successful Git
/// turn, using the same directory check `activation_snapshot` uses at
/// `reconcile()` time (via `vault_runtime::stat_local_content`). A managed
/// Vault's checkout may not have existed the last time that ran.
fn publish_local_content_after_git_success(control_block: &VaultControlBlock) {
    let (status, error) = stat_local_content(control_block.vault_path());
    let _ = control_block.set_local_content_status(status, error);
}

#[cfg(test)]
mod tests;
