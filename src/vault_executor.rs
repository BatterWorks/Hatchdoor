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
//! no trait, no framework, and no second execution lane.

use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(test)]
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use tracing::{debug, error, info, warn};

use crate::app_state::AppState;
use crate::cache::SqliteCache;
use crate::cache::vault_snapshots::{MutationGuardHandoff, VaultSnapshotFreshness};
use crate::embed::Embedder;
use crate::git::{
    CommitCooldown, ManagedCheckoutLease, ManagedGitOutcome, ManagedGitScheduler,
    ManagedGitTurnConfig, run_existing_git_commit_turn, run_existing_git_remote_turn,
    run_managed_git_commit_turn, run_managed_git_turn,
};
use crate::runtime_config::{ConfigSnapshot, RuntimeConfig};
use crate::startup::StartupTracker;
use crate::vault_registry::{
    VaultGitMode, VaultId, VaultRegistryStore, VaultSource as RegistryVaultSource,
};
use crate::vault_runtime::{
    LocalContentStatus, VaultCollectionRuntime, VaultControlBlock, VaultGitStatus,
    VaultRuntimeError, VaultRuntimeErrorDetail, VaultSearchStatus, stat_local_content,
};
use crate::vault_work::{
    VaultWorkCoordinator, VaultWorkError, VaultWorkKind, VaultWorkOutcome, VaultWorkRequest,
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
}

impl VaultWorkExecutor {
    /// Every field is one of `AppState`'s own, so the composition root has
    /// nothing to assemble: the executor is exactly the slice of shared
    /// runtime state a background turn is allowed to touch.
    pub(crate) fn from_state(state: &AppState) -> Self {
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
            VaultWorkKind::Index => {
                let embed_layers = snapshot
                    .setting("HATCHDOOR_EMBED_LAYERS")
                    .map(|setting| crate::runtime_config::is_truthy(&setting.value))
                    .unwrap_or(true);
                let progress_startup = self.startup.clone();
                dispatch_vault_index_turn_with_progress(
                    &self.vaults,
                    self.cache.clone(),
                    self.embedder.clone(),
                    embed_layers,
                    Some(Arc::new(move |progress| {
                        progress_startup.set_indexing(progress);
                    })),
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
    /// readiness rule, and the operator-facing log line.
    ///
    /// Per-Vault status is already published by the turn itself; this is only
    /// what the *collection* concludes from a turn having finished.
    pub(crate) fn publish_outcome(&self, outcome: &VaultWorkOutcome) {
        if outcome.request.kind() == VaultWorkKind::Index {
            match &outcome.result {
                Ok(()) if collection_indexes_ready(&self.vaults) => {
                    self.startup.set_ready();
                    self.model_setup_started.store(false, Ordering::Release);
                    info!("Vault collection indexing complete");
                }
                Err(error) if error.code() != "embedder_not_ready" => {
                    self.startup.set_failed();
                    self.model_setup_started.store(false, Ordering::Release);
                }
                _ => {}
            }
        }
        if let Err(error) = &outcome.result {
            // Repair remains expected until its dedicated packet;
            // Index and Git failures are actionable per-Vault status.
            if error.code() == "vault_work_kind_not_yet_implemented" {
                debug!(
                    vault_id = %outcome.request.vault_id(),
                    kind = ?outcome.request.kind(),
                    "Vault background work kind not yet implemented"
                );
            } else {
                warn!(
                    vault_id = %outcome.request.vault_id(),
                    kind = ?outcome.request.kind(),
                    code = error.code(),
                    message = error.message(),
                    "Vault background work turn failed"
                );
            }
        }
    }
}

/// Startup is Ready once every active Vault's Index turn has settled Ready.
/// An empty collection is never Ready: there is nothing that could have
/// finished indexing.
fn collection_indexes_ready(vaults: &VaultCollectionRuntime) -> bool {
    let active = vaults.active_vault_ids();
    !active.is_empty()
        && active.into_iter().all(|vault_id| {
            vaults
                .runtime(vault_id)
                .is_some_and(|runtime| runtime.snapshot().search == VaultSearchStatus::Ready)
        })
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
        request,
    )
    .await
}

pub(crate) async fn dispatch_vault_index_turn_with_progress(
    collection: &VaultCollectionRuntime,
    cache: Arc<SqliteCache>,
    embedder: Arc<dyn Embedder>,
    embed_layers: bool,
    on_progress: Option<Arc<dyn Fn(crate::startup::IndexingProgressSnapshot) + Send + Sync>>,
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
    // the generation stale if a mutation landed while it was released.
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
    control_block
        .set_search_status(VaultSearchStatus::Indexing, None)
        .map_err(vault_index_error)?;
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
        let indexing_control = control_block.clone();
        let indexing_cache = cache.clone();
        let publication_stale = published_stale.clone();
        match tokio::task::spawn_blocking(move || {
            let index = indexing_control
                .authoritative_index()
                .map_err(|error| (vault_index_error(error), true))?;
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
                            let freshness = if mutated {
                                // A write landed while this turn was
                                // embedding, so what is about to be published
                                // is already behind the Markdown. Search keeps
                                // answering from it; the label is what stops it
                                // claiming to be current. The watcher has
                                // already armed the catch-up turn.
                                VaultSnapshotFreshness::Stale
                            } else {
                                VaultSnapshotFreshness::Fresh
                            };
                            (freshness, guard)
                        }),
                    }),
                )
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
            // A generation published stale reports itself stale here too, which
            // is exactly what `retained_snapshot_search_status` would derive
            // from the same row after a restart. `Stale` still grants the
            // search capability, so the Vault keeps answering; the watcher's
            // change intent has already armed the turn that makes it `Ready`.
            let status = if published_stale.load(Ordering::Acquire) {
                VaultSearchStatus::Stale
            } else {
                VaultSearchStatus::Ready
            };
            let _ = control_block.set_search_status(status, None);
        }
        Err(error) => {
            let stale_mark_error = stale_mark_required
                .then(|| cache.mark_vault_snapshot_stale(vault_id))
                .transpose()
                .err();
            // A structure pass that succeeded before the embedding pass failed
            // leaves a participating generation with no vectors. Reporting it
            // `Stale` would grant the search capability to a Vault that can
            // only ever answer with nothing, so the vectorless axis wins here
            // exactly as it does in `retained_snapshot_search_status`. The
            // failure is not lost: it rides along as this status's error.
            let status = match cache.snapshot_status(vault_id) {
                Ok(Some(snapshot)) if snapshot.participating && snapshot.searchable => {
                    VaultSearchStatus::Stale
                }
                Ok(Some(snapshot)) if snapshot.participating => VaultSearchStatus::Browsable,
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
                    detail: None,
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

/// One Git turn's source-specific parts, resolved from a Vault's definition
/// before the shared turn shell below runs it.
///
/// The three source kinds that have a Git turn differ only in these fields
/// and in the blocking function they close over; everything else — the
/// mutation-lock hold, `spawn_blocking`, panic mapping, outcome publication —
/// is the shell's, once (issue #128).
struct GitTurnPlan {
    /// Whether the turn runs under the Vault's foreground mutation lock. Only
    /// `LocalHistory` runs without it: it commits already-settled drift in
    /// the working tree and never checks out, resets, or merges over a
    /// concurrent foreground write.
    holds_mutation_lock: bool,
    /// The error code a panic inside the blocking work is reported as.
    panic_code: &'static str,
    /// The blocking `git2` work itself, run off the async runtime.
    work: GitTurnWork,
}

/// The blocking half of a Git turn, and whether it needs this Vault's checkout
/// lease. The two shapes are distinct types rather than one closure taking an
/// `Option<&ManagedCheckoutLease>`, so a turn that needs a lease cannot be
/// built — or run — without one.
enum GitTurnWork {
    /// Runs against a checkout Hatchdoor does not own: an `ExistingGit`
    /// Vault's own working copy, in either remote-sync or Local-history mode.
    /// See `run_existing_git_remote_turn` for why `ManagedCheckoutLease` does
    /// not apply to an operator-owned checkout.
    Unleased(Box<dyn FnOnce() -> Result<ManagedGitOutcome, VaultWorkError> + Send + 'static>),
    /// Runs against the managed checkout under `state_directory`, holding that
    /// Vault's lease for the whole turn (issue #95).
    #[allow(clippy::type_complexity)]
    Leased {
        state_directory: PathBuf,
        run: Box<
            dyn FnOnce(&ManagedCheckoutLease) -> Result<ManagedGitOutcome, VaultWorkError>
                + Send
                + 'static,
        >,
    },
}

/// [`GitTurnWork`] with its checkout lease, if any, already acquired — the
/// state between "the lease is obtained" and "the mutation lock is taken",
/// which is the order those two must always be acquired in.
enum PreparedGitTurn {
    Unleased(Box<dyn FnOnce() -> Result<ManagedGitOutcome, VaultWorkError> + Send + 'static>),
    #[allow(clippy::type_complexity)]
    Leased {
        lease: ManagedCheckoutLease,
        run: Box<
            dyn FnOnce(&ManagedCheckoutLease) -> Result<ManagedGitOutcome, VaultWorkError>
                + Send
                + 'static,
        >,
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

    let result = run_planned_turn(&control_block, managed_git, vault_id, plan).await;
    finish_git_turn(&control_block, coordinator, managed_git, vault_id, result)
}

/// Run one already-planned Git or commit turn: obtain the checkout lease if
/// the plan needs one, take this Vault's mutation lock if the plan holds it,
/// run the blocking `git2` work off the async runtime, and hand the lease
/// back. Publication is the caller's, because a Git turn and a commit turn
/// conclude different things from the same result.
async fn run_planned_turn(
    control_block: &VaultControlBlock,
    managed_git: &ManagedGitScheduler,
    vault_id: VaultId,
    plan: GitTurnPlan,
) -> Result<ManagedGitOutcome, VaultWorkError> {
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
    // or this turn's checkout/reset could stomp a write mid-flight.
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
        match control_block.acquire_mutation().await {
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
    let result = run_planned_turn(&control_block, managed_git, vault_id, plan).await;
    finish_commit_turn(&control_block, commit_cooldown, vault_id, result)
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
            let state_directory = registry
                .path()
                .parent()
                .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
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
            // The same status a successful sync publishes. A remote failure
            // this clears is republished by that Vault's next scheduled sync
            // turn: the commit half being healthy is the honest report of
            // what this turn actually proved.
            let _ = control_block.set_git_status(VaultGitStatus::Ready, None);
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
            let state_directory = registry
                .path()
                .parent()
                .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
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

/// Read a Vault's stored HTTPS credentials, mapping an unreachable registry
/// into the retryable failure a Git turn reports for it.
fn git_credentials(
    registry: &VaultRegistryStore,
    vault_id: VaultId,
) -> Result<Option<crate::vault_registry::HttpsCredentials>, VaultWorkError> {
    registry.https_credentials(vault_id).map_err(|error| {
        VaultWorkError::new("managed_git_registry_unavailable", error.to_string(), true)
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
