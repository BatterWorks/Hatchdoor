//! Per-Vault managed-Git scheduling: each Vault polls on its own configured
//! interval (defaulting to daily), plus manual Sync/Retry now, and bounded
//! exponential backoff on transient failure — plus the concrete
//! acquire-or-reuse-then-synchronize turn `VaultWorkKind::Git` executes.
//!
//! This boundary decides *when* to request a Git turn for a Vault and *what*
//! one turn does. It does not know about `VaultControlBlock`, the Vault
//! registry, or the coordinator's worker loop: the Vault work executor
//! (`src/vault_executor.rs`) resolves a Vault's current configuration, runs
//! the turn, and publishes the result.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use tracing::warn;

use crate::vault_registry::{HttpsCredentials, VaultGitMode, VaultId};
use crate::vault_runtime_state::{GitTurnOutcome, GitTurnRecord, VaultRuntimeStateStore};
use crate::vault_work::{
    ScheduleResult, VaultWorkCoordinator, VaultWorkError, VaultWorkErrorDetail, VaultWorkKind,
};

use super::managed_checkout::{
    ManagedCheckoutError, ManagedCheckoutLease, ManagedCheckoutRequest, ManagedHttpsCredentials,
    acquire_or_reuse,
};
use super::managed_sync::{
    ManagedSyncConfig, ManagedSyncError, ManagedSyncMode, ManagedSyncOutcome,
    synchronize_managed_checkout,
};

/// The default interval a managed Vault waits before its next scheduled Git
/// turn after a success or non-retryable failure, absent an explicitly
/// configured `poll_interval_secs` (issue #97's reopening finding 2: the
/// interval is now per-Vault, carried by `VaultSource::ManagedGit` and set
/// on `ManagedGitScheduler` at [`ManagedGitScheduler::activate`] time — see
/// `vault_registry::DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS`, which mirrors
/// this same 24h value for the registry's own serde default).
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// The longest interval this scheduler will actually arm.
///
/// `poll_interval_secs` has a minimum but no maximum (`vault_registry`), and
/// every deadline this module arms is `Instant::now() + interval`, which
/// panics rather than saturating. [`ManagedGitScheduler::record_outcome`] does
/// that addition *while holding the `entries` lock*, so a single absurd value
/// would not just kill one Vault: it would poison the mutex and stop polling
/// for every Vault in the process, and the value survives a restart to do it
/// again. [`ManagedGitScheduler::polling_clock`] then renders the armed
/// deadline as a timestamp, which panics past chrono's year ceiling.
///
/// Clamping at the one boundary every interval enters this module through
/// ([`ManagedGitScheduler::activate`], the only writer of
/// `VaultScheduleEntry::poll_interval`) is what lets all of those stay plain
/// additions. It is deliberately *not* validation: nothing is rejected, and
/// nothing persisted changes, so a registry already holding such a value still
/// loads and still reads back unchanged. Bounding the value at the registry —
/// which would reject that registry at load, a wire-contract change — remains
/// a separate decision.
///
/// Ten years is "never" for a poll schedule, while staying far inside what an
/// `Instant`, a `SystemTime`, and chrono can all represent.
const MAX_POLL_INTERVAL: Duration = Duration::from_secs(10 * 365 * 24 * 60 * 60);

/// Backoff bounds for re-attempting a turn that failed for a retryable
/// (transient) reason, loosely mirroring the retired single-Vault task's
/// bounds (30s base, 60s ceiling; deleted in #185) but capped by
/// `vault_registry::MIN_MANAGED_GIT_POLL_INTERVAL_SECS`: a "normal" schedule
/// shorter than this bound would make it meaningless (see that constant's
/// doc), so the two move together. `BACKOFF_MAX` must equal that floor.
const BACKOFF_BASE: Duration = Duration::from_secs(30);
const BACKOFF_MAX: Duration = Duration::from_secs(60);

/// How often [`spawn_scheduler_tick`] checks for due Vaults in production.
/// Well below `BACKOFF_BASE`, so a transient failure's backoff resolves with
/// reasonable promptness rather than only on the next daily tick.
pub const DEFAULT_TICK_INTERVAL: Duration = Duration::from_secs(15);

/// The tick is a *sampling* interval: a deadline can only be noticed at its
/// resolution, so it has to be well under the shortest one the scheduler
/// arms, which is `BACKOFF_BASE`. At one sample per backoff base every
/// "30 second" retry would land at 60, collapsing `BACKOFF_BASE` into
/// `BACKOFF_MAX` and quietly making the distinction meaningless. Two samples
/// is the floor, and lowering either constant without the other is the
/// mistake this catches — the relationship was prose until it earned a
/// compile error.
const _: () = assert!(
    DEFAULT_TICK_INTERVAL.as_secs() * 2 <= BACKOFF_BASE.as_secs(),
    "the managed-Git scheduler tick must sample at least twice per backoff base"
);

/// Everything one managed-Git turn needs. Redaction-safe to hold and log:
/// `Debug` never reveals `repository_url` or `credentials`.
#[derive(Clone)]
pub struct ManagedGitTurnConfig {
    pub vault_id: VaultId,
    pub state_directory: PathBuf,
    pub repository_url: String,
    pub branch: Option<String>,
    pub vault_subdirectory: Option<PathBuf>,
    pub mode: VaultGitMode,
    pub credentials: Option<HttpsCredentials>,
    pub author_name: String,
    pub author_email: String,
}

impl std::fmt::Debug for ManagedGitTurnConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ManagedGitTurnConfig")
            .field("vault_id", &self.vault_id)
            .field("state_directory", &self.state_directory)
            .field("repository_url", &"[REDACTED]")
            .field("branch", &self.branch)
            .field("vault_subdirectory", &self.vault_subdirectory)
            .field("mode", &self.mode)
            .field(
                "credentials",
                &self.credentials.as_ref().map(|_| "[REDACTED]"),
            )
            .field("author_name", &self.author_name)
            .field("author_email", &self.author_email)
            .finish()
    }
}

/// The outcome of one successful managed-Git turn. Deliberately coarse: the
/// richer `ManagedSyncOutcome` distinctions (fast-forward vs. merge vs.
/// commit) are an implementation detail this seam does not need to publish.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManagedGitOutcome {
    UpToDate,
    Synchronized,
}

/// Run one acquire-or-reuse-then-synchronize turn for a managed-Git Vault.
///
/// This is the concrete operation `VaultWorkKind::Git` executes through the
/// coordinator. It performs blocking `git2`/filesystem I/O and must be run
/// from `spawn_blocking` by its async caller.
///
/// Takes `lease` rather than acquiring its own: the process-lifetime
/// ownership boundary this documents (see [`classify_checkout_error`]'s
/// `OwnershipUnavailable` arm) is held by the caller — in production,
/// [`super::managed_task::ManagedGitScheduler`] — across every turn for a
/// Vault for as long as it stays active in this process, not just for the
/// duration of one turn. This function only borrows it.
pub fn run_managed_git_turn(
    config: &ManagedGitTurnConfig,
    lease: &ManagedCheckoutLease,
) -> Result<ManagedGitOutcome, VaultWorkError> {
    let sync_mode = match config.mode {
        VaultGitMode::PullOnly => ManagedSyncMode::PullOnly,
        VaultGitMode::TwoWay => ManagedSyncMode::TwoWay,
        VaultGitMode::LocalHistory => {
            // Local history has no remote; scheduling a managed turn for it
            // is a caller bug, not a transient condition worth retrying.
            return Err(VaultWorkError::new(
                "managed_git_not_remote",
                "managed Git turn requested for a Local history Vault",
                false,
            ));
        }
    };

    let credentials = config
        .credentials
        .as_ref()
        .map(|credentials| ManagedHttpsCredentials {
            username: credentials.username.clone(),
            token: credentials.token.clone(),
        });
    let request = ManagedCheckoutRequest {
        state_directory: config.state_directory.clone(),
        vault_id: config.vault_id,
        repository_url: config.repository_url.clone(),
        branch: config.branch.clone(),
        vault_subdirectory: config.vault_subdirectory.clone(),
        credentials: credentials.clone(),
    };
    let checkout = acquire_or_reuse(lease, &request).map_err(classify_checkout_error)?;

    let sync_config = ManagedSyncConfig {
        repository_path: checkout.repository_path,
        vault_path: checkout.vault_path,
        repository_url: config.repository_url.clone(),
        branch: checkout.resolved_branch,
        mode: sync_mode,
        credentials,
        author_name: config.author_name.clone(),
        author_email: config.author_email.clone(),
    };
    let outcome = synchronize_managed_checkout(&sync_config).map_err(classify_sync_error)?;
    Ok(match outcome {
        ManagedSyncOutcome::UpToDate => ManagedGitOutcome::UpToDate,
        ManagedSyncOutcome::PullOnlyFastForwarded
        | ManagedSyncOutcome::TwoWaySynchronized { .. } => ManagedGitOutcome::Synchronized,
    })
}

/// Run one remote-sync turn for an `ExistingGit` Vault in `PullOnly` or
/// `TwoWay` mode against its already-existing checkout at `repository_path`.
///
/// Unlike [`run_managed_git_turn`], there is no managed-checkout acquisition
/// here: `repository_path` is the operator's own pre-existing directory, not
/// a Hatchdoor-owned clone placed under a `state_directory`, so there is
/// nothing to clone, lease, or track via `ManagedCheckoutLease`'s receipt
/// file — that machinery exists specifically for Hatchdoor-managed clones
/// into Hatchdoor-owned state directories (see its own doc comments), and
/// `ExistingGit` was never routed through it even for its already-implemented
/// `LocalHistory` mode (`super::sync::run_local_history_git_turn` opens the
/// checkout directly, the same way this function does).
///
/// `branch` mirrors `ExistingGit`'s registry field: unlike `ManagedGit`,
/// which always has a resolved branch by the time it reaches a turn (either
/// operator-configured, or resolved once from the remote default and
/// persisted in the managed checkout's receipt file at first acquisition),
/// an `ExistingGit` Vault's `branch` may be genuinely unconfigured — the
/// registry does not require one for `PullOnly`/`TwoWay` (only
/// `repository_url` is required; see `vault_registry.rs`'s
/// `normalize_structural_source`). When `None`, this falls back to whatever
/// branch is currently checked out at `repository_path` — mirroring
/// `super::sync::validate_local_repo`'s Local-history policy ("local history
/// follows whatever branch the operator has checked out") extended to the
/// remote-sync target.
///
/// Must run from `spawn_blocking`.
#[allow(clippy::too_many_arguments)] // Existing checkout identity plus sync policy and commit identity.
pub fn run_existing_git_remote_turn(
    repository_path: PathBuf,
    vault_path: PathBuf,
    repository_url: Option<String>,
    branch: Option<String>,
    mode: VaultGitMode,
    credentials: Option<HttpsCredentials>,
    author_name: String,
    author_email: String,
) -> Result<ManagedGitOutcome, VaultWorkError> {
    let Some(repository_url) = repository_url else {
        return Err(classify_sync_error(ManagedSyncError::Validation));
    };
    let sync_mode = match mode {
        VaultGitMode::PullOnly => ManagedSyncMode::PullOnly,
        VaultGitMode::TwoWay => ManagedSyncMode::TwoWay,
        VaultGitMode::LocalHistory => {
            // Defensive, mirroring `run_managed_git_turn`'s own defensive
            // rejection: callers only reach this function for `PullOnly`/
            // `TwoWay`, but this seam does not assume that holds forever.
            return Err(VaultWorkError::new(
                "managed_git_not_remote",
                "existing Git remote-sync turn requested for a Local history Vault",
                false,
            ));
        }
    };

    let branch = match branch {
        Some(branch) => branch,
        None => resolve_checked_out_branch(&repository_path).map_err(|_| {
            VaultWorkError::new(
                "existing_git_branch_unresolved",
                format!(
                    "cannot determine the currently checked-out branch of '{}'",
                    repository_path.display()
                ),
                false,
            )
        })?,
    };

    let credentials = credentials.map(|credentials| ManagedHttpsCredentials {
        username: credentials.username,
        token: credentials.token,
    });
    let sync_config = ManagedSyncConfig {
        repository_path,
        vault_path,
        repository_url,
        branch,
        mode: sync_mode,
        credentials,
        author_name,
        author_email,
    };
    let outcome = synchronize_managed_checkout(&sync_config).map_err(classify_sync_error)?;
    Ok(match outcome {
        ManagedSyncOutcome::UpToDate => ManagedGitOutcome::UpToDate,
        ManagedSyncOutcome::PullOnlyFastForwarded
        | ManagedSyncOutcome::TwoWaySynchronized { .. } => ManagedGitOutcome::Synchronized,
    })
}

/// The branch currently checked out at `repository_path`, used by
/// [`run_existing_git_remote_turn`] when an `ExistingGit` Vault has no
/// configured `branch`. Fails rather than guessing on a detached HEAD or an
/// unreadable repository — `repository_path` is a directory the operator
/// controls and could change at any time, not a Hatchdoor-owned clone whose
/// shape this process can assume.
fn resolve_checked_out_branch(repository_path: &Path) -> Result<String, ()> {
    let repository = git2::Repository::open(repository_path).map_err(|_| ())?;
    let head = repository.head().map_err(|_| ())?;
    if !head.is_branch() {
        return Err(());
    }
    head.shorthand().map(str::to_owned).map_err(|_| ())
}

/// Classify a checkout failure. `retryable` decides whether the scheduler
/// backs off and retries automatically or waits for the normal schedule, a
/// manual retry, a configuration change, or a restart.
///
/// `pub(crate)`: also used by `vault_executor::dispatch_git_turn_with`
/// to classify a lease-acquisition failure at dispatch time (before
/// `run_managed_git_turn` runs), so both classify the same
/// `ManagedCheckoutError` the same way instead of duplicating this table.
pub(crate) fn classify_checkout_error(error: ManagedCheckoutError) -> VaultWorkError {
    use ManagedCheckoutError::*;
    let (code, retryable) = match error {
        StateDirectoryUnavailable => ("managed_git_state_directory_unavailable", false),
        // Contention on the process-lifetime lock. `ManagedGitScheduler`
        // holds this Vault's lease for as long as it stays active in this
        // process (see `ManagedGitScheduler::take_or_acquire_checkout_lease`),
        // so reaching this from a normal turn means another process (or a
        // concurrent Hatchdoor instance) holds it; worth a bounded automatic
        // retry.
        OwnershipUnavailable => ("managed_git_checkout_busy", true),
        UnsafeRepositoryUrl => ("managed_git_unsafe_url", false),
        // A preserved-and-rejected structural mismatch (unknown directory,
        // escaping symlink, interrupted acquisition) — needs a human, not a
        // blind retry.
        DestinationInvalid => ("managed_git_destination_invalid", false),
        CloneFailed => ("managed_git_remote_unreachable", true),
        AuthenticationFailed => ("managed_git_authentication_failed", false),
        ValidationFailed => ("managed_git_validation_failed", false),
        AtomicInstallFailed => ("managed_git_install_failed", false),
    };
    VaultWorkError::new(code, error.to_string(), retryable)
}

/// Classify a synchronization failure. See [`classify_checkout_error`] for
/// the retryable/non-retryable split rationale.
///
/// `message` is computed from `error` before the match below moves its
/// `DirtyWorkingCopy`/`Conflict`/`LocalCommits` fields out into structured
/// `detail` — `Display` renders the same text either way, so `message` is
/// unaffected by carrying the same data a second time as `detail`.
fn classify_sync_error(error: ManagedSyncError) -> VaultWorkError {
    use ManagedSyncError::*;
    let message = error.to_string();
    let (code, retryable, detail) = match error {
        Validation => ("managed_git_validation_failed", false, None),
        DirtyWorkingCopy { files } => (
            "managed_git_dirty_working_copy",
            false,
            Some(VaultWorkErrorDetail::AffectedPaths(files)),
        ),
        LocalCommits { ahead } => (
            "managed_git_pull_only_local_commits",
            false,
            Some(VaultWorkErrorDetail::LocalCommitsAhead(ahead)),
        ),
        Conflict { files } => (
            "managed_git_conflict",
            false,
            Some(VaultWorkErrorDetail::AffectedPaths(files)),
        ),
        // The bounded fetch-integrate-push replay inside
        // `synchronize_managed_checkout` already absorbed a couple of
        // immediate retries; reaching here means the race is still live,
        // which is exactly the kind of transient condition backoff exists
        // for.
        PushRace => ("managed_git_push_race_exhausted", true, None),
        Authentication => ("managed_git_authentication_failed", false, None),
        Remote => ("managed_git_remote_unreachable", true, None),
    };
    let work_error = VaultWorkError::new(code, message, retryable);
    match detail {
        Some(detail) => work_error.with_detail(detail),
        None => work_error,
    }
}

#[derive(Clone, Copy)]
struct ScheduleState {
    next_attempt: Instant,
    backoff: Option<Duration>,
}

/// One tracked Vault's schedule and, once obtained, its held checkout
/// lease — kept in the *same* map entry, behind the *same* mutex, so a
/// removal ([`ManagedGitScheduler::deactivate`]) and a check-then-store of
/// the lease ([`ManagedGitScheduler::keep_checkout_lease`]) can never
/// interleave. Splitting these into two independently locked maps was
/// exactly issue #95's reopened race: a `deactivate` running between
/// `keep_checkout_lease`'s tracked-check and its insert could leave a
/// lease stored for a Vault `deactivate` had already stopped tracking,
/// leaking its OS-level lock indefinitely.
struct VaultScheduleEntry {
    schedule: ScheduleState,
    /// This Vault's own poll interval (issue #97's reopening finding 2),
    /// read from `VaultSource::ManagedGit::poll_interval_secs` at
    /// [`ManagedGitScheduler::activate`] time and clamped there to
    /// [`MAX_POLL_INTERVAL`] — that call is the only writer, which is what
    /// lets every `Instant::now() + poll_interval` in this module be a plain
    /// addition rather than a checked one.
    ///
    /// Mostly independent of `schedule`: an interval change never resets an
    /// in-progress backoff or a held checkout lease. It does reach
    /// `schedule.next_attempt`, but in one direction only — see
    /// [`ManagedGitScheduler::activate`] for why a shortened interval has to
    /// move the attempt already armed and a lengthened one must not.
    poll_interval: Duration,
    lease: Option<ManagedCheckoutLease>,
    /// This Vault's last interval-arming turn, restored from durable state at
    /// activation and refreshed by every later turn. Held here purely so a
    /// status read is a memory read: the file is consulted once per
    /// activation, never per request.
    last_completed: Option<GitTurnRecord>,
}

/// One Vault's polling clock, as wall-clock instants a status read can
/// render.
///
/// `last_checked_at` is when this Vault's last interval-arming turn
/// *finished*, whether it succeeded or failed — a failed check is still a
/// check, and the Vault's own Git status already says which it was. `None`
/// until the first one completes.
///
/// `next_attempt_at` is always known for a tracked Vault, because a Vault
/// that has never completed a turn is due immediately rather than
/// unscheduled. It is derived from the in-memory countdown, so it stays
/// honest for a manual sync or a backoff, neither of which is durable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GitPollingClock {
    pub last_checked_at: Option<SystemTime>,
    pub next_attempt_at: SystemTime,
}

/// Decides *when* to request `VaultWorkKind::Git` for each active managed-Git
/// Vault: that Vault's own configured poll interval (see
/// [`Self::activate`]), an immediate manual Sync/Retry-now, or a bounded
/// exponential backoff after a transient failure.
///
/// One instance drives every managed-Git Vault in the process — mirroring
/// the coordinator's own single-worker design, this adds no per-Vault
/// execution lane (ADR-13).
///
/// Also owns each active Vault's [`ManagedCheckoutLease`] for the Vault's
/// entire active lifetime in this process (issue #95): the lease is
/// obtained lazily by the first turn that needs it
/// ([`Self::take_or_acquire_checkout_lease`]), handed back after every turn
/// ([`Self::keep_checkout_lease`]), and dropped — releasing the underlying
/// OS-level lock — only by [`Self::deactivate`]. This is what makes the
/// checkout ownership boundary process-lifetime rather than turn-scoped: a
/// second process (or a concurrent Hatchdoor instance) cannot acquire the
/// same lock in the gap between two scheduled turns. The schedule and the
/// lease share one [`VaultScheduleEntry`] behind one `Mutex`, so a
/// concurrent `deactivate` (the coordinator's own doc comment on
/// `drain_vault` confirms an in-flight turn is never force-cancelled, so
/// this is the designed-for case, not an edge case) can never race a turn
/// handing its lease back — see [`VaultScheduleEntry`].
pub struct ManagedGitScheduler {
    coordinator: VaultWorkCoordinator,
    entries: Mutex<BTreeMap<VaultId, VaultScheduleEntry>>,
    /// Where a Vault's last interval-arming turn is remembered across
    /// restarts. `None` is the process-local schedule
    /// [`Self::without_durable_state`] builds.
    state: Option<Arc<VaultRuntimeStateStore>>,
}

impl ManagedGitScheduler {
    /// A scheduler that forgets every schedule when the process ends, for
    /// callers with nowhere durable to write: every restart re-arms each
    /// Vault immediately, which is precisely the behavior
    /// [`Self::with_state_store`] exists to remove. Named so no call site can
    /// opt out of remembering without saying so.
    pub fn without_durable_state(coordinator: VaultWorkCoordinator) -> Self {
        Self {
            coordinator,
            entries: Mutex::new(BTreeMap::new()),
            state: None,
        }
    }

    /// The production constructor: schedules survive a restart because each
    /// interval-arming turn is remembered in `state`.
    pub fn with_state_store(
        coordinator: VaultWorkCoordinator,
        state: Arc<VaultRuntimeStateStore>,
    ) -> Self {
        Self {
            coordinator,
            entries: Mutex::new(BTreeMap::new()),
            state: Some(state),
        }
    }

    /// Register a Vault so it participates in scheduled polling, due
    /// immediately, using `poll_interval` for its daily-equivalent re-arm
    /// (see [`Self::record_outcome`]). Re-activating an already-tracked Vault
    /// preserves its schedule state — an in-progress backoff and any held
    /// checkout lease are never disturbed — but `poll_interval` is always
    /// applied, even to an already-tracked Vault (issue #97's reopening
    /// finding 2), so the interval update and schedule-state preservation are
    /// handled as two independent concerns rather than both being gated by
    /// "already tracked."
    ///
    /// The interval also reaches the attempt already armed, but only ever to
    /// bring it forward, and only when that attempt is the poll interval's
    /// rather than a backoff's — see the comment on the re-arm itself below
    /// for why each of those halves is there. In practice only a definition
    /// edit gets that far: `reconcile()` skips an unchanged Vault before
    /// calling this at all, so the already-tracked path is the edit path, not
    /// something every reconcile pass runs.
    ///
    /// Not *quite* idempotent on the armed attempt, and deliberately not
    /// claimed to be: `record_outcome` stamps `completed_at` before taking the
    /// lock and arms `next_attempt` after it, so re-activating at an unchanged
    /// interval re-derives a deadline earlier by that gap and `min` adopts it.
    /// The shift is the width of a lock acquisition, and `sync_now`/`retry_now`
    /// take this path on every call; nothing downstream can resolve it, since
    /// the tick samples at [`DEFAULT_TICK_INTERVAL`].
    pub fn activate(&self, vault_id: VaultId, poll_interval: Duration) {
        // Bound the interval once, here, rather than checking every addition
        // it later feeds. See [`MAX_POLL_INTERVAL`]: this call is the only
        // writer of `VaultScheduleEntry::poll_interval`, so clamping on the
        // way in is what makes the rest of the module's `Instant + interval`
        // arithmetic safe — including `record_outcome`'s, which runs under the
        // `entries` lock and would poison it for every Vault on overflow.
        let poll_interval = poll_interval.min(MAX_POLL_INTERVAL);
        // Read and parse the state file before taking the lock. `tick` takes
        // the same lock on every pass, and activating a collection of N
        // Vaults would otherwise hold it across N whole-file parses at
        // startup — the same reason `record_outcome` writes outside it.
        // Wasted for an already-tracked Vault, which is the rarer path and
        // costs one read of a file measured in Vaults, not notes.
        let restored = self.restored_schedule(vault_id, poll_interval);
        let mut entries = self.entries.lock().expect("managed Git scheduler poisoned");
        match entries.entry(vault_id) {
            std::collections::btree_map::Entry::Occupied(mut occupied) => {
                let entry = occupied.get_mut();
                entry.poll_interval = poll_interval;
                // A shortened interval has to reach the attempt already
                // armed, not just the one after it: an operator shortens the
                // interval *for* the next check, and a Vault that had a long
                // interval armed would otherwise serve the whole of it out
                // before the edit had any visible effect. `min` is what keeps
                // this to a shortening — a lengthened interval leaves the
                // nearer deadline where it is rather than pushing the Vault
                // out — and the backoff guard is what keeps an in-progress
                // transient retry, which is not on the interval at all, from
                // being re-armed as though it were.
                if entry.schedule.backoff.is_none()
                    && let Some(record) = &entry.last_completed
                    && let Some(wait) = remaining_wait(record.completed_at, poll_interval)
                {
                    entry.schedule.next_attempt =
                        entry.schedule.next_attempt.min(Instant::now() + wait);
                }
            }
            std::collections::btree_map::Entry::Vacant(vacant) => {
                let (next_attempt, last_completed) = restored;
                vacant.insert(VaultScheduleEntry {
                    schedule: ScheduleState {
                        next_attempt,
                        backoff: None,
                    },
                    poll_interval,
                    lease: None,
                    last_completed,
                });
            }
        }
    }

    /// When a newly tracked Vault's first turn is due: one poll interval
    /// after the last interval-arming turn this Vault is remembered to have
    /// completed, or immediately when there is no usable record — a Vault
    /// that has never synced, a lost or unreadable state file, or a build
    /// that cannot read the one it found.
    ///
    /// The remembered time is wall clock, because that is the only clock that
    /// survives a restart; the deadline it produces is then held as an
    /// `Instant`, so the countdown itself cannot be disturbed by the host
    /// clock moving while the process runs.
    fn restored_schedule(
        &self,
        vault_id: VaultId,
        poll_interval: Duration,
    ) -> (Instant, Option<GitTurnRecord>) {
        let now = Instant::now();
        let Some(state) = &self.state else {
            return (now, None);
        };
        let Some(record) = state.last_git_turn(vault_id) else {
            return (now, None);
        };
        // A record whose time cannot be reasoned from is no record: it is not
        // one to publish a `last_checked_at` from either, so the Vault comes
        // back as one that has never checked rather than one reporting a time
        // from a clock nobody trusts. (A stamp in the future means the host
        // clock moved; treating it as unknown is what keeps this Vault's next
        // turn from being delayed by the skew.)
        let Some(wait) = remaining_wait(record.completed_at, poll_interval) else {
            return (now, None);
        };
        (now + wait, Some(record))
    }

    /// This Vault's last remembered interval-arming turn, if any. Activation
    /// uses it to republish the Git status the previous process had reached,
    /// which a fresh process would otherwise report as a blank `pending`
    /// until the next scheduled turn.
    pub fn remembered_turn(&self, vault_id: VaultId) -> Option<GitTurnRecord> {
        self.entries
            .lock()
            .expect("managed Git scheduler poisoned")
            .get(&vault_id)
            .and_then(|entry| entry.last_completed.clone())
    }

    /// This Vault's polling clock, or `None` when it is not tracked (a
    /// `Local` Vault, or one that is disabled or disconnected).
    pub fn polling_clock(&self, vault_id: VaultId) -> Option<GitPollingClock> {
        let entries = self.entries.lock().expect("managed Git scheduler poisoned");
        let entry = entries.get(&vault_id)?;
        Some(GitPollingClock {
            last_checked_at: entry
                .last_completed
                .as_ref()
                .map(|record| record.completed_at),
            next_attempt_at: SystemTime::now()
                + entry
                    .schedule
                    .next_attempt
                    .saturating_duration_since(Instant::now()),
        })
    }

    /// Stop tracking a Vault (disabled, disconnected, or retired). Any
    /// coordinator-side pending work is discarded separately by
    /// `VaultWorkCoordinator::drain_vault`. Removing the entry also drops
    /// any checkout lease held for this Vault, releasing its OS-level lock
    /// immediately so a later re-acquisition — a genuine process restart, or
    /// this same process reconnecting the Vault — succeeds. Because removal
    /// happens in the same locked critical section
    /// [`Self::keep_checkout_lease`] uses to check-then-store a lease, a
    /// turn's in-flight `keep_checkout_lease` call can never race this: it
    /// either finds the entry gone (and drops the lease it was holding
    /// instead of storing it) or completes its store before this removal
    /// runs (and the lease it stored is removed, and dropped, right here).
    pub fn deactivate(&self, vault_id: VaultId) {
        self.entries
            .lock()
            .expect("managed Git scheduler poisoned")
            .remove(&vault_id);
    }

    /// Drop this Vault's durable record, for a Vault leaving the collection
    /// (see [`VaultRuntimeStateStore::forget`]). Distinct from
    /// [`Self::deactivate`], which also runs when a Vault is merely disabled
    /// and must keep its schedule.
    ///
    /// A failure is logged rather than propagated: pruning is housekeeping,
    /// and a disconnect that already succeeded must not be reported as failed
    /// because a disposable file could not be rewritten.
    pub fn forget_persisted_state(&self, vault_id: VaultId) {
        let Some(state) = &self.state else {
            return;
        };
        if let Err(message) = state.forget(vault_id) {
            warn!(%vault_id, %message, "could not forget a disconnected Vault's remembered Git turn");
        }
    }

    /// Obtain the checkout lease a turn for `vault_id` should use: the lease
    /// already held from a previous turn in this process, if any, or a
    /// freshly acquired one otherwise (the first turn since this Vault was
    /// activated, or since a previous acquisition failed and was never
    /// stored).
    ///
    /// Takes the lease out of the tracked entry for the duration of the
    /// caller's turn — pass it back with [`Self::keep_checkout_lease`] once
    /// the turn completes so it stays held across turns instead of being
    /// dropped (and its OS-level lock released) at the end of each one.
    ///
    /// Reusing an already-held lease is a fast, non-blocking `Mutex`
    /// operation. Only the fallback first acquisition for a newly activated
    /// (or previously-failed-and-never-stored) Vault performs blocking,
    /// local-filesystem-only I/O (directory creation, opening and
    /// `flock`-ing the lock file) — bounded and one-time per Vault
    /// activation, so production calls this directly from the async
    /// dispatch path rather than requiring `spawn_blocking`.
    pub(crate) fn take_or_acquire_checkout_lease(
        &self,
        state_directory: PathBuf,
        vault_id: VaultId,
    ) -> Result<ManagedCheckoutLease, ManagedCheckoutError> {
        let held = {
            let mut entries = self.entries.lock().expect("managed Git scheduler poisoned");
            entries
                .get_mut(&vault_id)
                .and_then(|entry| entry.lease.take())
        };
        match held {
            Some(lease) => Ok(lease),
            None => ManagedCheckoutLease::acquire(state_directory, vault_id),
        }
    }

    /// Return a lease obtained from [`Self::take_or_acquire_checkout_lease`]
    /// so it stays held — and its OS-level lock stays exclusive to this
    /// process — across turns.
    ///
    /// Looks up and stores into the tracked entry inside one lock
    /// acquisition, atomically with [`Self::deactivate`]'s removal (both
    /// operate on the same `entries` mutex): if `vault_id` was deactivated
    /// while the turn holding this lease was in flight, the entry is gone by
    /// the time this runs, so `lease` is dropped here instead of stored,
    /// releasing the lock right away rather than leaking it for the rest of
    /// the process's life. There is no window between "check tracked" and
    /// "store the lease" for a concurrent `deactivate` to land in.
    pub(crate) fn keep_checkout_lease(&self, vault_id: VaultId, lease: ManagedCheckoutLease) {
        let mut entries = self.entries.lock().expect("managed Git scheduler poisoned");
        if let Some(entry) = entries.get_mut(&vault_id) {
            entry.lease = Some(lease);
        }
        // else: `vault_id` is no longer tracked — `lease` is dropped here,
        // while still holding `entries`' lock, releasing the OS lock.
    }

    /// Test-only observation of a tracked Vault's currently stored interval.
    /// `entries` is private and behind a `Mutex`, so a cross-module
    /// end-to-end test (e.g. `handlers/vaults.rs`'s edit-path regression
    /// test for issue #97's reopening finding 2) has no other way to prove
    /// an edit actually reached the live scheduler without either
    /// duplicating this module's internal state or waiting on real
    /// wall-clock scheduling.
    #[cfg(test)]
    pub(crate) fn poll_interval_for_test(&self, vault_id: VaultId) -> Option<Duration> {
        self.entries
            .lock()
            .expect("managed Git scheduler poisoned")
            .get(&vault_id)
            .map(|entry| entry.poll_interval)
    }

    /// Test-only observation of a tracked Vault's currently armed
    /// `next_attempt`, so a cross-module test can prove an interval update
    /// left an in-progress backoff untouched. See
    /// [`Self::poll_interval_for_test`].
    #[cfg(test)]
    pub(crate) fn next_attempt_for_test(&self, vault_id: VaultId) -> Option<Instant> {
        self.entries
            .lock()
            .expect("managed Git scheduler poisoned")
            .get(&vault_id)
            .map(|entry| entry.schedule.next_attempt)
    }

    /// Request an immediate turn for exactly one Vault, bypassing its
    /// schedule. Registers the Vault if it was not already tracked, so a
    /// manual sync can never be silently dropped — `poll_interval` is the
    /// value that registration (or an already-tracked Vault's refreshed
    /// interval, per [`Self::activate`]) uses; callers already have the
    /// Vault's current definition in hand (e.g. `handlers/vaults.rs`'s
    /// manual control endpoint, which reads it off the same definition it
    /// just validated is `ManagedGit`). Coalesces with any already-pending
    /// Git turn for that Vault.
    pub fn sync_now(&self, vault_id: VaultId, poll_interval: Duration) -> ScheduleResult {
        self.activate(vault_id, poll_interval);
        self.coordinator.request(vault_id, VaultWorkKind::Git)
    }

    /// Same admitted operation as [`Self::sync_now`]; kept as a distinctly
    /// named entry point because "retry a failed Vault" and "sync a healthy
    /// one" are different user intents even though both resolve to the same
    /// coordinator request.
    pub fn retry_now(&self, vault_id: VaultId, poll_interval: Duration) -> ScheduleResult {
        self.sync_now(vault_id, poll_interval)
    }

    /// Record one Git turn's outcome and arm the next attempt: the daily
    /// interval after a success or a non-retryable failure (including
    /// authentication — it waits for a configuration change, a manual retry,
    /// a restart, or the normal schedule, never a blind backoff retry), or a
    /// bounded exponential backoff after a retryable (transient) failure.
    ///
    /// A no-op for a Vault no longer tracked (deactivated between request and
    /// outcome).
    pub fn record_outcome(
        &self,
        vault_id: VaultId,
        result: &Result<ManagedGitOutcome, VaultWorkError>,
    ) {
        let completed_at = SystemTime::now();
        // Built once, before the lock, and then used for both halves of
        // remembering this turn: the in-memory stamp a status read renders,
        // and the durable record a restart resumes from. The two must
        // describe the same turn.
        let record = remembered_record(result, completed_at);
        let arms_interval = {
            let mut entries = self.entries.lock().expect("managed Git scheduler poisoned");
            let Some(entry) = entries.get_mut(&vault_id) else {
                return;
            };
            let poll_interval = entry.poll_interval;
            // Whether this outcome armed the poll interval, as opposed to a
            // transient failure's backoff. Decided here, once, and then used
            // to gate both halves of remembering it.
            let arms_interval = {
                let schedule = &mut entry.schedule;
                match result {
                    Ok(_) => {
                        schedule.backoff = None;
                        schedule.next_attempt = Instant::now() + poll_interval;
                        true
                    }
                    Err(error) if error.retryable() => {
                        let next_backoff = schedule
                            .backoff
                            .map_or(BACKOFF_BASE, |previous| (previous * 2).min(BACKOFF_MAX));
                        schedule.backoff = Some(next_backoff);
                        schedule.next_attempt = Instant::now() + next_backoff;
                        false
                    }
                    Err(_) => {
                        schedule.backoff = None;
                        schedule.next_attempt = Instant::now() + poll_interval;
                        true
                    }
                }
            };
            if arms_interval {
                entry.last_completed = Some(record.clone());
            }
            arms_interval
        };
        // Outside the entries lock: this writes a file, and `tick` takes the
        // same lock on every pass.
        if arms_interval {
            self.remember(vault_id, record);
        }
    }

    /// Persist the outcome that just armed this Vault's interval, so a
    /// restart resumes the countdown instead of beginning a new one.
    ///
    /// Only interval-arming outcomes are remembered. A transient failure's
    /// backoff is deliberately process-local: it exists to throttle a
    /// condition that is usually gone by the next start, and a restart should
    /// retry at once rather than serve out a stale backoff it cannot verify.
    ///
    /// A write failure is logged and dropped. The turn itself already
    /// happened, and the only cost of forgetting it is one extra turn after
    /// the next restart — never a reason to fail work that succeeded.
    fn remember(&self, vault_id: VaultId, record: GitTurnRecord) {
        let Some(state) = &self.state else {
            return;
        };
        if let Err(message) = state.record_git_turn(vault_id, record) {
            warn!(%vault_id, %message, "could not remember this Vault's Git turn; its schedule will restart from the next activation");
        }
    }

    /// Request a Git turn for every tracked Vault whose schedule is due as of
    /// `now`.
    ///
    /// Uses [`VaultWorkCoordinator::request_if_idle`] rather than
    /// [`VaultWorkCoordinator::request`], so a Vault whose Git turn is
    /// already active or already has a pending rerun queued is skipped.
    /// `request` only coalesces *duplicate* requests of an already-active
    /// turn into one guaranteed rerun — and that rerun would fire the instant
    /// the active turn's `execute` closure returns, before `record_outcome`
    /// (called from inside that same closure, once the turn's result is
    /// known) has a chance to arm backoff. A Git turn can easily outlast
    /// `DEFAULT_TICK_INTERVAL`, so an unconditional `request` here would
    /// pre-queue that zero-delay rerun on every tick during the active
    /// window, defeating backoff on every retryable failure. The skip is
    /// intentionally scoped to `tick()`'s own automatic due-check;
    /// [`Self::sync_now`]/[`Self::retry_now`] must still coalesce a manual
    /// request into the turn's one guaranteed rerun exactly as before — a
    /// user explicitly asking for a resync is not a case this skip should
    /// swallow.
    pub fn tick(&self, now: Instant) {
        let due = {
            let entries = self.entries.lock().expect("managed Git scheduler poisoned");
            entries
                .iter()
                .filter(|(_, entry)| entry.schedule.next_attempt <= now)
                .map(|(vault_id, _)| *vault_id)
                .collect::<Vec<_>>()
        };
        for vault_id in due {
            self.coordinator
                .request_if_idle(vault_id, VaultWorkKind::Git);
        }
    }

    /// The same due-check [`Self::tick`] applies, for exactly one Vault, run
    /// at the moment it is activated.
    ///
    /// Activation cannot simply wait for the next tick: a Vault that has
    /// never synced — one just added, or reconstructed with no remembered
    /// turn — would sit unsynced for up to `DEFAULT_TICK_INTERVAL` while the
    /// operator watches, and a newly created Vault has nothing to browse
    /// until its first turn lands. Nor can activation request
    /// unconditionally, which is what made every restart re-sync a Vault
    /// that was nowhere near due. Asking whether it is due answers both.
    ///
    /// Returns whether the turn was requested, so a caller can tell "started
    /// syncing" from "already up to date, waiting out its interval".
    pub fn request_if_due(&self, vault_id: VaultId) -> bool {
        let due = {
            let entries = self.entries.lock().expect("managed Git scheduler poisoned");
            entries
                .get(&vault_id)
                .is_some_and(|entry| entry.schedule.next_attempt <= Instant::now())
        };
        if due {
            self.coordinator
                .request_if_idle(vault_id, VaultWorkKind::Git);
        }
        due
    }
}

/// How long a Vault that last completed a turn at `completed_at` still has to
/// wait before its next one is due, under `poll_interval` — zero once the
/// interval has already elapsed.
///
/// The wall clock is the only clock that survives a restart or describes a
/// turn, so it is what the remembered record holds; a *deadline* held that way
/// would move with the host clock, so callers add this remaining wait to an
/// `Instant` instead. Returning the wait rather than the deadline is what lets
/// this bridge stay one function across both callers, which arm from different
/// `Instant`s: registration from the one it captured, the re-arm from `now`.
///
/// The addition itself is safe to leave unchecked because `poll_interval` is
/// clamped to [`MAX_POLL_INTERVAL`] on the way into the scheduler, and the
/// returned wait is never longer than the interval it came from.
///
/// `None` means `completed_at` is in the future, so the host clock moved and
/// the record's time cannot be reasoned from at all. Neither caller delays the
/// Vault by the skew: registration treats the record as unusable, and a
/// shortened interval leaves the armed attempt alone.
fn remaining_wait(completed_at: SystemTime, poll_interval: Duration) -> Option<Duration> {
    let elapsed = SystemTime::now().duration_since(completed_at).ok()?;
    Some(poll_interval.saturating_sub(elapsed))
}

/// One completed turn as it is remembered, in memory and on disk.
fn remembered_record(
    result: &Result<ManagedGitOutcome, VaultWorkError>,
    completed_at: SystemTime,
) -> GitTurnRecord {
    GitTurnRecord {
        completed_at,
        outcome: match result {
            Ok(ManagedGitOutcome::UpToDate) => GitTurnOutcome::UpToDate,
            Ok(ManagedGitOutcome::Synchronized) => GitTurnOutcome::Synchronized,
            Err(error) => GitTurnOutcome::Failed {
                code: error.code().to_string(),
                message: error.message().to_string(),
            },
        },
    }
}

/// Spawn the periodic tick that keeps every tracked Vault's own configured
/// schedule (and any armed backoff) moving forward. `tick_interval` controls
/// how often the schedule is *checked*, not how often a Vault actually syncs
/// — keep it well below every Vault's own poll interval and `BACKOFF_BASE`.
/// Aborting the returned
/// handle is sufficient to stop it: the task holds no resources of its own
/// and issues no destructive operations, only coordinator requests, which
/// have their own independent shutdown draining.
pub fn spawn_scheduler_tick(
    scheduler: std::sync::Arc<ManagedGitScheduler>,
    tick_interval: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tick_interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            scheduler.tick(Instant::now());
        }
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use git2::{Repository, Signature};
    use tempfile::TempDir;
    use tokio::sync::Notify;

    use super::*;

    fn commit(repository: &Repository, path: &str, contents: &str) {
        let workdir = repository.workdir().expect("workdir");
        std::fs::write(workdir.join(path), contents).expect("write");
        let mut index = repository.index().expect("index");
        index.add_path(Path::new(path)).expect("stage");
        index.write().expect("write index");
        let tree = repository
            .find_tree(index.write_tree().expect("tree id"))
            .expect("tree");
        let signature = Signature::now("Test", "test@example.test").expect("signature");
        let parent = repository
            .head()
            .ok()
            .and_then(|head| head.peel_to_commit().ok());
        let parents = parent.iter().collect::<Vec<_>>();
        repository
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                "initial",
                &tree,
                &parents,
            )
            .expect("commit");
    }

    fn fixture(mode: VaultGitMode) -> (TempDir, ManagedGitTurnConfig) {
        let root = tempfile::tempdir().expect("tempdir");
        let source = root.path().join("source");
        let source_repository = Repository::init(&source).expect("source repository");
        std::fs::create_dir(source.join("vault")).expect("vault directory");
        commit(&source_repository, "vault/Home.md", "# Home\n");

        let remote = root.path().join("remote.git");
        Repository::init_bare(&remote).expect("bare remote");
        let mut origin = source_repository
            .remote("origin", remote.to_str().expect("remote path"))
            .expect("origin");
        origin
            .push(&["refs/heads/master:refs/heads/master"], None)
            .expect("initial push");

        let state_directory = root.path().join("state");
        std::fs::create_dir(&state_directory).expect("state directory");
        (
            root,
            ManagedGitTurnConfig {
                vault_id: VaultId::generate().expect("Vault ID"),
                state_directory,
                repository_url: remote.to_string_lossy().into_owned(),
                branch: None,
                vault_subdirectory: Some(PathBuf::from("vault")),
                mode,
                credentials: None,
                author_name: "Hatchdoor".to_string(),
                author_email: "hatchdoor@example.test".to_string(),
            },
        )
    }

    #[test]
    fn run_managed_git_turn_acquires_then_synchronizes_a_fresh_pull_only_vault() {
        let (_root, config) = fixture(VaultGitMode::PullOnly);
        let lease = ManagedCheckoutLease::acquire(config.state_directory.clone(), config.vault_id)
            .expect("lease");

        let outcome = run_managed_git_turn(&config, &lease).expect("first turn acquires and syncs");

        assert_eq!(outcome, ManagedGitOutcome::UpToDate);
        assert!(
            config
                .state_directory
                .join("vaults")
                .join(config.vault_id.to_string())
                .join("repository")
                .join("vault/Home.md")
                .is_file()
        );
    }

    #[test]
    fn run_managed_git_turn_reuses_an_existing_checkout_on_a_later_turn() {
        let (_root, config) = fixture(VaultGitMode::TwoWay);
        // The same held lease serves both turns, exactly like
        // `ManagedGitScheduler` reusing one lease across a Vault's turns in
        // production (issue #95) — `run_managed_git_turn` itself no longer
        // cares whether the lease it is lent is freshly acquired or already
        // held.
        let lease = ManagedCheckoutLease::acquire(config.state_directory.clone(), config.vault_id)
            .expect("lease");
        run_managed_git_turn(&config, &lease).expect("first turn");

        let repository_root = config
            .state_directory
            .join("vaults")
            .join(config.vault_id.to_string())
            .join("repository");
        std::fs::write(repository_root.join("vault/Local.md"), "local\n").expect("local edit");

        let outcome =
            run_managed_git_turn(&config, &lease).expect("second turn reuses the checkout");

        assert_eq!(outcome, ManagedGitOutcome::Synchronized);
    }

    #[test]
    fn run_managed_git_turn_rejects_local_history_mode_without_touching_the_filesystem() {
        let (_root, config) = fixture(VaultGitMode::LocalHistory);
        // The Local-history rejection is evaluated before the lease is
        // used at all, so a lease from an unrelated scratch state
        // directory proves this without ever touching `config.state_directory`
        // (asserted below).
        let scratch = tempfile::tempdir().expect("scratch state directory");
        let lease = ManagedCheckoutLease::acquire(scratch.path().to_path_buf(), config.vault_id)
            .expect("scratch lease");

        let error = run_managed_git_turn(&config, &lease).expect_err("Local history has no remote");

        assert_eq!(error.code(), "managed_git_not_remote");
        assert!(!error.retryable());
        assert!(
            !config
                .state_directory
                .join("vaults")
                .join(config.vault_id.to_string())
                .exists()
        );
    }

    #[test]
    fn checkout_error_classification_matches_the_transient_retry_boundary() {
        let retryable = [
            ManagedCheckoutError::OwnershipUnavailable,
            ManagedCheckoutError::CloneFailed,
        ];
        for error in retryable {
            assert!(
                classify_checkout_error(error.clone()).retryable(),
                "{error:?} should be retryable"
            );
        }
        let non_retryable = [
            ManagedCheckoutError::StateDirectoryUnavailable,
            ManagedCheckoutError::UnsafeRepositoryUrl,
            ManagedCheckoutError::DestinationInvalid,
            ManagedCheckoutError::AuthenticationFailed,
            ManagedCheckoutError::ValidationFailed,
            ManagedCheckoutError::AtomicInstallFailed,
        ];
        for error in non_retryable {
            assert!(
                !classify_checkout_error(error.clone()).retryable(),
                "{error:?} should not be retryable"
            );
        }
        assert_eq!(
            classify_checkout_error(ManagedCheckoutError::AuthenticationFailed).code(),
            "managed_git_authentication_failed"
        );
    }

    #[test]
    fn sync_error_classification_matches_the_transient_retry_boundary() {
        let retryable = [ManagedSyncError::PushRace, ManagedSyncError::Remote];
        for error in retryable {
            assert!(
                classify_sync_error(error.clone()).retryable(),
                "{error:?} should be retryable"
            );
        }
        let non_retryable = [
            ManagedSyncError::Validation,
            ManagedSyncError::DirtyWorkingCopy {
                files: vec!["a.md".to_string()],
            },
            ManagedSyncError::LocalCommits { ahead: 1 },
            ManagedSyncError::Conflict {
                files: vec!["a.md".to_string()],
            },
            ManagedSyncError::Authentication,
        ];
        for error in non_retryable {
            assert!(
                !classify_sync_error(error.clone()).retryable(),
                "{error:?} should not be retryable"
            );
        }
        assert_eq!(
            classify_sync_error(ManagedSyncError::Authentication).code(),
            "managed_git_authentication_failed"
        );
    }

    /// Issue #132: `DirtyWorkingCopy`/`Conflict`/`LocalCommits` carry their
    /// affected-paths/count outward as structured `detail`, and `message`
    /// stays exactly what `Display` produces for those variants — the
    /// detail is additive, not a substitute.
    #[test]
    fn sync_error_classification_carries_structured_detail_for_select_codes() {
        let dirty = ManagedSyncError::DirtyWorkingCopy {
            files: vec!["a.md".to_string(), "b.md".to_string()],
        };
        let classified = classify_sync_error(dirty.clone());
        assert_eq!(classified.message(), dirty.to_string());
        assert_eq!(
            classified.detail(),
            Some(&VaultWorkErrorDetail::AffectedPaths(vec![
                "a.md".to_string(),
                "b.md".to_string()
            ]))
        );

        let conflict = ManagedSyncError::Conflict {
            files: vec!["c.md".to_string()],
        };
        assert_eq!(
            classify_sync_error(conflict).detail(),
            Some(&VaultWorkErrorDetail::AffectedPaths(vec![
                "c.md".to_string()
            ]))
        );

        let local_commits = ManagedSyncError::LocalCommits { ahead: 3 };
        assert_eq!(
            classify_sync_error(local_commits).detail(),
            Some(&VaultWorkErrorDetail::LocalCommitsAhead(3))
        );

        for error in [
            ManagedSyncError::Validation,
            ManagedSyncError::PushRace,
            ManagedSyncError::Authentication,
            ManagedSyncError::Remote,
        ] {
            assert_eq!(
                classify_sync_error(error.clone()).detail(),
                None,
                "{error:?} should carry no structured detail"
            );
        }
    }

    /// A test-only stand-in for a Vault's configured poll interval. Shorter
    /// than `DEFAULT_POLL_INTERVAL` so interval-independent assertions stay
    /// easy to reason about; tests that specifically exercise per-Vault
    /// interval differences (issue #97's reopening finding 2) use their own
    /// explicit values instead.
    const TEST_POLL_INTERVAL: Duration = Duration::from_secs(3600);

    fn scheduler() -> (VaultWorkCoordinator, ManagedGitScheduler) {
        let (coordinator, _worker) = VaultWorkCoordinator::new();
        let scheduler = ManagedGitScheduler::without_durable_state(coordinator.clone());
        (coordinator, scheduler)
    }

    fn vault_id(value: &str) -> VaultId {
        value.parse().expect("valid test Vault ID")
    }

    #[test]
    fn activate_requests_an_immediate_turn_on_the_next_tick() {
        let (coordinator, scheduler) = scheduler();
        let vault = vault_id("00000000-0000-4000-8000-000000000001");

        scheduler.activate(vault, TEST_POLL_INTERVAL);
        scheduler.tick(Instant::now());

        assert_eq!(
            coordinator.request(vault, VaultWorkKind::Git),
            ScheduleResult::Coalesced,
            "tick() must already have queued this Vault's Git turn"
        );
    }

    #[test]
    fn tick_does_not_request_work_for_a_vault_not_yet_due() {
        let (coordinator, scheduler) = scheduler();
        let vault = vault_id("00000000-0000-4000-8000-000000000001");
        scheduler.activate(vault, TEST_POLL_INTERVAL);
        // A recorded success re-arms the daily interval, so this Vault is no
        // longer due at `Instant::now()`.
        scheduler.record_outcome(vault, &Ok(ManagedGitOutcome::UpToDate));

        scheduler.tick(Instant::now());

        assert_eq!(
            coordinator.request(vault, VaultWorkKind::Git),
            ScheduleResult::Queued,
            "a Vault not yet due must not have been requested by tick()"
        );
    }

    #[test]
    fn reactivating_a_tracked_vault_does_not_reset_an_armed_backoff() {
        let (_coordinator, scheduler) = scheduler();
        let vault = vault_id("00000000-0000-4000-8000-000000000001");
        scheduler.activate(vault, TEST_POLL_INTERVAL);
        scheduler.record_outcome(
            vault,
            &Err(VaultWorkError::new(
                "managed_git_remote_unreachable",
                "x",
                true,
            )),
        );
        let armed_backoff = {
            let entries = scheduler.entries.lock().expect("scheduler entries");
            entries[&vault].schedule.next_attempt
        };

        scheduler.activate(vault, TEST_POLL_INTERVAL);

        let after_reactivate = {
            let entries = scheduler.entries.lock().expect("scheduler entries");
            entries[&vault].schedule.next_attempt
        };
        assert_eq!(armed_backoff, after_reactivate);
    }

    /// Closes issue #97's reopening finding 2: a definition change that only
    /// updates a Vault's configured `poll_interval_secs` must take effect —
    /// `activate()` must update an already-tracked Vault's interval in
    /// place — without disturbing an in-progress backoff, which is a
    /// completely independent concern (see the doc comment on
    /// `ManagedGitScheduler::activate`).
    #[test]
    fn reactivating_with_a_changed_interval_updates_it_without_resetting_an_armed_backoff() {
        let (_coordinator, scheduler) = scheduler();
        let vault = vault_id("00000000-0000-4000-8000-000000000001");
        scheduler.activate(vault, TEST_POLL_INTERVAL);
        scheduler.record_outcome(
            vault,
            &Err(VaultWorkError::new(
                "managed_git_remote_unreachable",
                "x",
                true,
            )),
        );
        let armed_backoff = {
            let entries = scheduler.entries.lock().expect("scheduler entries");
            entries[&vault].schedule.next_attempt
        };

        let new_interval = TEST_POLL_INTERVAL * 2;
        scheduler.activate(vault, new_interval);

        let (after_reactivate, stored_interval) = {
            let entries = scheduler.entries.lock().expect("scheduler entries");
            (
                entries[&vault].schedule.next_attempt,
                entries[&vault].poll_interval,
            )
        };
        assert_eq!(
            armed_backoff, after_reactivate,
            "an interval change must not reset an in-progress backoff"
        );
        assert_eq!(
            stored_interval, new_interval,
            "activate() must update the tracked interval even for an already-tracked Vault"
        );

        // The *new* interval, not the original one, is what actually gets
        // used once the in-progress backoff resolves and a success re-arms
        // the daily-equivalent schedule.
        let before = Instant::now();
        scheduler.record_outcome(vault, &Ok(ManagedGitOutcome::UpToDate));
        let next_attempt = {
            let entries = scheduler.entries.lock().expect("scheduler entries");
            entries[&vault].schedule.next_attempt
        };
        assert!(next_attempt >= before + new_interval - Duration::from_secs(1));
        assert!(next_attempt < before + new_interval + Duration::from_secs(5));
    }

    /// The companion to the test above, for the direction it does not cover:
    /// *shortening* an interval does reach the armed attempt (that is the
    /// whole point of the change), so the backoff is no longer protected by
    /// `activate` leaving every deadline alone — it is protected by an
    /// explicit guard, and this is the case that proves the guard is load
    /// bearing.
    ///
    /// The Vault here has both a remembered success (so a deadline can be
    /// computed from it at all) and a later transient failure's backoff,
    /// which is the only combination that can go wrong: a backoff is
    /// deliberately *not* on the poll interval, so re-deriving the attempt
    /// from the last interval-arming turn would discard the throttle on a
    /// remote that is currently failing — and the shorter the operator makes
    /// the interval, the harder the retry storm.
    #[test]
    fn shortening_the_interval_does_not_re_arm_a_vault_that_is_mid_backoff() {
        let (_coordinator, scheduler) = scheduler();
        let vault = vault_id("00000000-0000-4000-8000-000000000001");
        scheduler.activate(vault, TEST_POLL_INTERVAL);
        // A success first, so the Vault carries a remembered interval-arming
        // turn; then a transient failure, which arms a backoff and leaves
        // that remembered turn in place.
        scheduler.record_outcome(vault, &Ok(ManagedGitOutcome::UpToDate));
        scheduler.record_outcome(
            vault,
            &Err(VaultWorkError::new(
                "managed_git_remote_unreachable",
                "x",
                true,
            )),
        );
        let armed_backoff = {
            let entries = scheduler.entries.lock().expect("scheduler entries");
            entries[&vault].schedule.next_attempt
        };

        // Far shorter than BACKOFF_BASE, so a deadline re-derived from the
        // remembered success would land well before the armed backoff and
        // visibly replace it.
        let shortened = Duration::from_secs(1);
        assert!(
            shortened < BACKOFF_BASE,
            "the shortened interval must be able to undercut the backoff for this test to discriminate"
        );
        scheduler.activate(vault, shortened);

        let (after_reactivate, stored_interval) = {
            let entries = scheduler.entries.lock().expect("scheduler entries");
            (
                entries[&vault].schedule.next_attempt,
                entries[&vault].poll_interval,
            )
        };
        assert_eq!(
            armed_backoff, after_reactivate,
            "shortening the interval must not cut an in-progress backoff short"
        );
        assert_eq!(
            stored_interval, shortened,
            "the shortened interval must still be stored, for the re-arm after the backoff resolves"
        );
    }

    /// The other half of "only forward". A shortened interval reaches the
    /// armed attempt; a *lengthened* one must not, or an operator moving a
    /// Vault from hourly to daily fifty minutes into the hour would push the
    /// check that was ten minutes away out by a further day — the same
    /// surprise this whole change exists to remove, in the opposite
    /// direction. `min` is what makes the re-arm one-directional, and this is
    /// the only test that holds it: replacing it with a plain assignment
    /// passes every other test in the suite.
    #[test]
    fn lengthening_the_interval_leaves_a_nearer_armed_attempt_alone() {
        let (_coordinator, scheduler) = scheduler();
        let vault = vault_id("00000000-0000-4000-8000-000000000001");
        scheduler.activate(vault, TEST_POLL_INTERVAL);
        scheduler.record_outcome(vault, &Ok(ManagedGitOutcome::UpToDate));
        let armed_interval = {
            let entries = scheduler.entries.lock().expect("scheduler entries");
            entries[&vault].schedule.next_attempt
        };

        let lengthened = TEST_POLL_INTERVAL * 24;
        scheduler.activate(vault, lengthened);

        let (after_reactivate, stored_interval) = {
            let entries = scheduler.entries.lock().expect("scheduler entries");
            (
                entries[&vault].schedule.next_attempt,
                entries[&vault].poll_interval,
            )
        };
        assert_eq!(
            armed_interval, after_reactivate,
            "lengthening the interval must not push an already-armed attempt further out"
        );
        assert_eq!(
            stored_interval, lengthened,
            "the lengthened interval must still be stored, for the re-arm after that attempt"
        );
    }

    /// `poll_interval_secs` has a minimum but no maximum, so a registry can
    /// hand the scheduler an interval whose deadline is not representable.
    /// Every `Instant + interval` in this module panics rather than saturating
    /// on that, and `record_outcome` does it *under the `entries` lock*, so
    /// one such Vault would poison the mutex and stop polling for every other
    /// Vault in the process — then do it again after a restart, because the
    /// value is durable.
    ///
    /// `activate` clamping to `MAX_POLL_INTERVAL` is what disarms all of it at
    /// once. This drives the whole lifecycle at `Duration::MAX` — register,
    /// re-activate, succeed, fail transiently, fail permanently — because the
    /// point is not any single addition but that no path can reach an
    /// unclamped one.
    #[test]
    fn an_interval_too_large_to_arm_is_clamped_rather_than_panicking() {
        let (_coordinator, scheduler) = scheduler();
        let vault = vault_id("00000000-0000-4000-8000-000000000001");

        scheduler.activate(vault, Duration::MAX);
        // Re-activation takes the Occupied arm, which re-derives a deadline
        // from the remembered turn — a second, distinct addition.
        scheduler.activate(vault, Duration::MAX);
        // Each of these arms `next_attempt` while holding the lock.
        scheduler.record_outcome(vault, &Ok(ManagedGitOutcome::UpToDate));
        scheduler.activate(vault, Duration::MAX);
        scheduler.record_outcome(
            vault,
            &Err(VaultWorkError::new(
                "managed_git_remote_unreachable",
                "x",
                true,
            )),
        );
        scheduler.record_outcome(
            vault,
            &Err(VaultWorkError::new("managed_git_auth_failed", "x", false)),
        );

        let stored_interval = {
            let entries = scheduler
                .entries
                .lock()
                .expect("the entries lock must not have been poisoned by an overflowing deadline");
            entries[&vault].poll_interval
        };
        assert_eq!(
            stored_interval, MAX_POLL_INTERVAL,
            "an interval past the ceiling must be stored clamped, not as given"
        );
    }

    /// The clamp has to leave the *reported* clock representable too, not just
    /// the internal arithmetic. `polling_clock` converts the armed `Instant`
    /// back to a `SystemTime`, and `vault_management` renders that through
    /// `format_timestamp`, whose chrono conversion panics past year 262143 —
    /// so an unclamped interval would move the panic from the scheduler into
    /// `GET /api/v1/vaults` instead of removing it.
    #[test]
    fn a_clamped_interval_still_reports_a_renderable_next_attempt() {
        let (_coordinator, scheduler) = scheduler();
        let vault = vault_id("00000000-0000-4000-8000-000000000001");
        scheduler.activate(vault, Duration::MAX);
        scheduler.record_outcome(vault, &Ok(ManagedGitOutcome::UpToDate));

        let clock = scheduler.polling_clock(vault).expect("a tracked Vault");
        let rendered = crate::vault_runtime_state::format_timestamp(clock.next_attempt_at);
        assert!(
            chrono::DateTime::parse_from_rfc3339(&rendered).is_ok(),
            "the clamped next attempt must render as a parseable timestamp, got {rendered}"
        );
    }

    /// Closes issue #97's reopening finding 2: before this fix,
    /// `ManagedGitScheduler` had exactly one `poll_interval` shared by every
    /// tracked Vault. Proves two Vaults activated with different intervals
    /// re-arm and become due independently.
    #[test]
    fn two_vaults_with_different_poll_intervals_get_independent_schedules() {
        let (coordinator, scheduler) = scheduler();
        let short_vault = vault_id("00000000-0000-4000-8000-000000000001");
        let long_vault = vault_id("00000000-0000-4000-8000-000000000002");
        let short_interval = Duration::from_secs(3600);
        let long_interval = Duration::from_secs(6 * 3600);
        scheduler.activate(short_vault, short_interval);
        scheduler.activate(long_vault, long_interval);
        scheduler.record_outcome(short_vault, &Ok(ManagedGitOutcome::UpToDate));
        scheduler.record_outcome(long_vault, &Ok(ManagedGitOutcome::UpToDate));

        let (short_next, long_next) = {
            let entries = scheduler.entries.lock().expect("scheduler entries");
            (
                entries[&short_vault].schedule.next_attempt,
                entries[&long_vault].schedule.next_attempt,
            )
        };
        assert!(
            long_next > short_next,
            "a Vault configured with a longer poll interval must re-arm further out than one with a shorter interval"
        );

        // A tick at a time past only the short Vault's re-arm point must
        // request just that Vault, leaving the long-interval Vault alone.
        scheduler.tick(short_next + Duration::from_millis(1));
        assert_eq!(
            coordinator.request(short_vault, VaultWorkKind::Git),
            ScheduleResult::Coalesced,
            "the short-interval Vault must already be due and requested by tick()"
        );
        assert_eq!(
            coordinator.request(long_vault, VaultWorkKind::Git),
            ScheduleResult::Queued,
            "the long-interval Vault must not yet be due"
        );
    }

    #[test]
    fn deactivate_stops_tracking_so_a_later_tick_does_not_request_it() {
        let (coordinator, scheduler) = scheduler();
        let vault = vault_id("00000000-0000-4000-8000-000000000001");
        scheduler.activate(vault, TEST_POLL_INTERVAL);

        scheduler.deactivate(vault);
        scheduler.tick(Instant::now());

        assert_eq!(
            coordinator.request(vault, VaultWorkKind::Git),
            ScheduleResult::Queued,
            "a real request after an untracked tick must be Queued, not Coalesced"
        );
    }

    #[test]
    fn sync_now_and_retry_now_request_immediately_and_track_the_vault() {
        let (coordinator, scheduler) = scheduler();
        let vault = vault_id("00000000-0000-4000-8000-000000000001");

        assert_eq!(
            scheduler.sync_now(vault, TEST_POLL_INTERVAL),
            ScheduleResult::Queued
        );
        assert_eq!(
            coordinator.request(vault, VaultWorkKind::Git),
            ScheduleResult::Coalesced,
            "sync_now already queued this Vault's Git turn"
        );
        // record_outcome now finds the Vault tracked (activated by sync_now).
        scheduler.record_outcome(vault, &Ok(ManagedGitOutcome::UpToDate));
    }

    /// Closes issue #97's reopening finding 1: `tick()` used to call
    /// `coordinator.request()` unconditionally for every due Vault, even one
    /// whose turn was already active. Because `request()` treats a duplicate
    /// of active work as "queue exactly one guaranteed rerun," a tick landing
    /// mid-turn pre-queued a rerun that would fire the instant the active
    /// turn completed — before `record_outcome` (called from inside that same
    /// turn, once its result is known) had a chance to arm backoff. A
    /// retryable failure's backoff was therefore defeated: the Vault retried
    /// immediately instead of waiting.
    ///
    /// Drives the coordinator and scheduler directly, without a real
    /// `VaultWorkWorker::run_next` loop consuming turns automatically
    /// (mirroring `vault_work.rs`'s own `repeated_active_requests_coalesce_to_one_required_rerun`
    /// pattern), so a turn can be held "active" under direct control while
    /// `tick()` runs against it.
    #[tokio::test]
    async fn tick_does_not_pre_queue_a_zero_delay_rerun_for_an_already_active_turn() {
        let (coordinator, mut worker) = VaultWorkCoordinator::new();
        let scheduler = ManagedGitScheduler::without_durable_state(coordinator);
        let vault = vault_id("00000000-0000-4000-8000-000000000001");
        assert_eq!(
            scheduler.sync_now(vault, TEST_POLL_INTERVAL),
            ScheduleResult::Queued
        );

        // Take the turn active under direct control (no automatic worker
        // loop), mirroring `vault_work.rs`'s own
        // `repeated_active_requests_coalesce_to_one_required_rerun` pattern.
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let running = tokio::spawn({
            let started = started.clone();
            let release = release.clone();
            async move {
                let outcome = worker
                    .run_next(move |request| {
                        let started = started.clone();
                        let release = release.clone();
                        async move {
                            started.notify_one();
                            release.notified().await;
                            let _ = request;
                            Ok::<(), VaultWorkError>(())
                        }
                    })
                    .await
                    .expect("active turn");
                (worker, outcome)
            }
        });
        started.notified().await;

        // The turn is now active. A tick landing here, before the fix, would
        // pre-queue a guaranteed immediate rerun regardless of what backoff
        // the turn's own outcome is about to arm.
        scheduler.tick(Instant::now());

        // The turn fails for a retryable reason. Production calls
        // `record_outcome` from inside the `execute` closure — i.e. before
        // `run_next` clears `active` — so this call is placed the same way
        // here, between the tick above and the turn's completion below.
        let transient = Err(VaultWorkError::new(
            "managed_git_remote_unreachable",
            "x",
            true,
        ));
        scheduler.record_outcome(vault, &transient);
        let armed_next_attempt = {
            let entries = scheduler.entries.lock().expect("scheduler entries");
            entries[&vault].schedule.next_attempt
        };
        assert!(
            armed_next_attempt > Instant::now(),
            "a retryable failure must arm a future backoff, not an immediate retry"
        );

        release.notify_one();
        let (mut worker, outcome) = running.await.expect("worker task");
        assert_eq!(outcome.request.vault_id(), vault);

        // If `tick()` had pre-queued a rerun, it would be ready here
        // immediately — proving backoff was defeated. With the fix, no
        // rerun exists yet: the next turn only becomes available once
        // `armed_next_attempt` actually elapses and a later `tick()` finds
        // it due.
        assert!(
            tokio::time::timeout(
                Duration::from_millis(25),
                worker.run_next(|_| async { Ok::<(), VaultWorkError>(()) })
            )
            .await
            .is_err(),
            "tick() must not have queued an immediate rerun for a Vault whose turn was still active"
        );
    }

    #[test]
    fn a_retryable_failure_backs_off_exponentially_and_a_success_resets_it() {
        let (_coordinator, scheduler) = scheduler();
        let vault = vault_id("00000000-0000-4000-8000-000000000001");
        scheduler.activate(vault, TEST_POLL_INTERVAL);
        let transient = Err(VaultWorkError::new(
            "managed_git_remote_unreachable",
            "x",
            true,
        ));

        let before = Instant::now();
        scheduler.record_outcome(vault, &transient);
        let first_backoff = {
            let entries = scheduler.entries.lock().expect("scheduler entries");
            entries[&vault].schedule.next_attempt.duration_since(before)
        };
        scheduler.record_outcome(vault, &transient);
        let second_backoff = {
            let entries = scheduler.entries.lock().expect("scheduler entries");
            entries[&vault].schedule.next_attempt.duration_since(before)
        };
        assert!(
            second_backoff > first_backoff,
            "a repeated transient failure must back off further: {first_backoff:?} -> {second_backoff:?}"
        );

        scheduler.record_outcome(vault, &Ok(ManagedGitOutcome::UpToDate));
        let after_success = {
            let entries = scheduler.entries.lock().expect("scheduler entries");
            entries[&vault].schedule
        };
        assert!(after_success.backoff.is_none());
        assert!(after_success.next_attempt >= before + TEST_POLL_INTERVAL - Duration::from_secs(1));
    }

    /// Issue #132's second acceptance criterion: a transient failure on a
    /// one-minute Vault "backs off and recovers on a scale proportionate to
    /// its schedule, not the old one-hour cap." Reads the raw `backoff`
    /// field (not `next_attempt`, which is also offset by wall-clock elapsed
    /// time between calls) so the plateau value is exact.
    #[test]
    fn repeated_transient_failures_plateau_at_the_new_backoff_max_not_the_old_one_hour_cap() {
        let (_coordinator, scheduler) = scheduler();
        let vault = vault_id("00000000-0000-4000-8000-000000000001");
        scheduler.activate(vault, Duration::from_secs(60));
        let transient = Err(VaultWorkError::new(
            "managed_git_remote_unreachable",
            "x",
            true,
        ));

        let mut last_backoff = None;
        for _ in 0..10 {
            scheduler.record_outcome(vault, &transient);
            last_backoff = {
                let entries = scheduler.entries.lock().expect("scheduler entries");
                entries[&vault].schedule.backoff
            };
        }

        assert_eq!(
            last_backoff,
            Some(BACKOFF_MAX),
            "ten consecutive transient failures must plateau at BACKOFF_MAX, not keep doubling"
        );
        assert!(
            BACKOFF_MAX < Duration::from_secs(60 * 60),
            "the cap itself must be well under the pre-#132 one-hour bound"
        );
    }

    #[test]
    fn a_non_retryable_failure_including_authentication_waits_for_the_normal_schedule_not_backoff()
    {
        let (_coordinator, scheduler) = scheduler();
        let vault = vault_id("00000000-0000-4000-8000-000000000001");
        scheduler.activate(vault, TEST_POLL_INTERVAL);
        let auth_failure = Err(VaultWorkError::new(
            "managed_git_authentication_failed",
            "bad credentials",
            false,
        ));

        let before = Instant::now();
        scheduler.record_outcome(vault, &auth_failure);

        let schedule = {
            let entries = scheduler.entries.lock().expect("scheduler entries");
            entries[&vault].schedule
        };
        assert!(
            schedule.backoff.is_none(),
            "authentication failures must not arm exponential backoff"
        );
        assert!(schedule.next_attempt >= before + TEST_POLL_INTERVAL - Duration::from_secs(1));
    }

    #[test]
    fn record_outcome_is_a_no_op_for_an_untracked_vault() {
        let (_coordinator, scheduler) = scheduler();
        let vault = vault_id("00000000-0000-4000-8000-000000000001");

        // No panic, no tracked entry created.
        scheduler.record_outcome(vault, &Ok(ManagedGitOutcome::UpToDate));

        let entries = scheduler.entries.lock().expect("scheduler entries");
        assert!(!entries.contains_key(&vault));
    }

    /// Closes issue #95's reopening finding: `run_managed_git_turn` used to
    /// acquire its own `ManagedCheckoutLease` and drop it (releasing the
    /// OS-level lock) the instant one turn returned, so a second process
    /// could acquire the same lock in the gap between two scheduled turns —
    /// contradicting the documented process-lifetime ownership lease. This
    /// proves the fix: across two consecutive turns for the same Vault
    /// driven through `ManagedGitScheduler`'s lease-holding methods (the
    /// same ones `dispatch_git_turn_with` uses in production), a
    /// second process's `ManagedCheckoutLease::acquire` for the same
    /// `(state_directory, vault_id)` fails with `OwnershipUnavailable`
    /// *during the gap between those two turns* — the lock is held
    /// continuously, not just while each turn executes.
    ///
    /// Before this fix, the equivalent probe (two direct
    /// `run_managed_git_turn(&config)` calls with an interleaved
    /// contention check) failed: the interleaved acquire attempt succeeded
    /// where it should have been refused.
    #[test]
    fn scheduler_holds_the_checkout_lease_across_turns_until_deactivated() {
        let (_root, config) = fixture(VaultGitMode::TwoWay);
        let (coordinator, _worker) = VaultWorkCoordinator::new();
        let scheduler = ManagedGitScheduler::without_durable_state(coordinator);
        scheduler.activate(config.vault_id, TEST_POLL_INTERVAL);

        // First turn: no lease held yet, so one is acquired fresh and then
        // handed back to the scheduler instead of being dropped.
        let lease = scheduler
            .take_or_acquire_checkout_lease(config.state_directory.clone(), config.vault_id)
            .expect("first turn acquires a fresh lease");
        run_managed_git_turn(&config, &lease).expect("first turn");
        scheduler.keep_checkout_lease(config.vault_id, lease);

        // Between turns — exactly the gap the old, turn-scoped lease used
        // to release the lock in — a second process attempting the same
        // (state_directory, vault_id) must be refused: the scheduler is
        // still holding the OS-level lock from the first turn.
        match ManagedCheckoutLease::acquire(config.state_directory.clone(), config.vault_id) {
            Err(ManagedCheckoutError::OwnershipUnavailable) => {}
            other => panic!(
                "a second process must not acquire the lock between turns, got {:?}",
                other.map(|_| "Ok(lease)")
            ),
        }

        // Second turn: the scheduler reuses the held lease (a fresh acquire
        // would itself fail — the OS lock is still open from the first
        // turn) and proves the checkout is genuinely reused, not
        // re-cloned.
        let repository_root = config
            .state_directory
            .join("vaults")
            .join(config.vault_id.to_string())
            .join("repository");
        std::fs::write(repository_root.join("vault/Local.md"), "local\n").expect("local edit");
        let lease = scheduler
            .take_or_acquire_checkout_lease(config.state_directory.clone(), config.vault_id)
            .expect("second turn reuses the held lease");
        let outcome =
            run_managed_git_turn(&config, &lease).expect("second turn reuses the checkout");
        assert_eq!(outcome, ManagedGitOutcome::Synchronized);
        scheduler.keep_checkout_lease(config.vault_id, lease);

        // Deactivating the Vault (retirement, disable, disconnect) must
        // release the held lock: a later acquisition — a genuine process
        // restart, or this same process reconnecting the Vault — succeeds
        // again. This is the existing "drop/reacquire models restart"
        // guarantee from `managed_checkout.rs`'s own tests, now proven at
        // the point where the fix actually changed *when* the drop
        // happens: at deactivation instead of at end-of-turn.
        scheduler.deactivate(config.vault_id);
        ManagedCheckoutLease::acquire(config.state_directory, config.vault_id)
            .expect("deactivate must release the held lock so re-acquisition succeeds");
    }

    /// Closes the race an independent Standards review found in the
    /// previous version of this fix: `deactivate` and `keep_checkout_lease`
    /// used to touch two *separate* mutexes (`state` and, formerly,
    /// `leases`) with independent lock/unlock cycles, so a
    /// `keep_checkout_lease` that had already read "this Vault is still
    /// tracked" from `state` could still insert its lease into `leases`
    /// after a concurrent `deactivate` removed the Vault from both maps —
    /// leaking the OS-level lock past deactivation.
    /// `VaultWorkCoordinator::drain_vault`'s own doc comment ("a currently
    /// active turn is never force-cancelled ... retains the worker until it
    /// returns at its own safe operation boundary") confirms a `deactivate`
    /// racing an in-flight turn's `keep_checkout_lease` is the designed-for
    /// case, not an edge case.
    ///
    /// Deterministically reproduces the scenario for that contract —
    /// `deactivate` landing while a turn is holding the lease, before it is
    /// handed back — without relying on real thread-timing luck: obtain the
    /// lease, deactivate the Vault, *then* hand the lease back, and assert
    /// it is not retained (no resurrected tracking entry) and its OS-level
    /// lock is genuinely released (a fresh `ManagedCheckoutLease::acquire`
    /// for the same Vault succeeds immediately afterward).
    ///
    /// `entries` now being one `Mutex<BTreeMap<VaultId, VaultScheduleEntry>>`
    /// — the same lock both `deactivate`'s removal and
    /// `keep_checkout_lease`'s check-then-store acquire — makes the two
    /// operations mutually exclusive by construction: there is no longer
    /// any window, real or reordered, in which `keep_checkout_lease` can
    /// observe "still tracked" and then store into an entry `deactivate`
    /// has already removed. (The original defect required a real interleaving
    /// *inside* the old `keep_checkout_lease`'s two separate lock
    /// acquisitions — genuine thread parallelism, not reorderable top-level
    /// calls — so this test encodes the now-guaranteed contract rather than
    /// a call sequence that would necessarily have failed against the old,
    /// two-mutex code.)
    #[test]
    fn keep_checkout_lease_does_not_resurrect_or_leak_a_lease_after_a_concurrent_deactivate() {
        let (_root, config) = fixture(VaultGitMode::PullOnly);
        let (coordinator, _worker) = VaultWorkCoordinator::new();
        let scheduler = ManagedGitScheduler::without_durable_state(coordinator);
        scheduler.activate(config.vault_id, TEST_POLL_INTERVAL);

        // A turn takes the lease to run with — exactly what a real
        // in-flight Git turn does before `spawn_blocking`.
        let lease = scheduler
            .take_or_acquire_checkout_lease(config.state_directory.clone(), config.vault_id)
            .expect("turn acquires a fresh lease");

        // While that turn is still in flight (holding `lease`, not yet
        // handed back), the Vault is deactivated — the coordinator's
        // documented "never force-cancelled" behavior means this is
        // expected, not exceptional.
        scheduler.deactivate(config.vault_id);

        // The turn completes and hands its lease back, unaware the Vault
        // was deactivated while it ran.
        scheduler.keep_checkout_lease(config.vault_id, lease);

        // The lease must not have been resurrected into a tracked entry:
        // no schedule, no held lease, for this Vault.
        {
            let entries = scheduler.entries.lock().expect("scheduler entries");
            assert!(
                !entries.contains_key(&config.vault_id),
                "keep_checkout_lease must not resurrect a deactivated Vault's tracking entry"
            );
        }

        // And the OS-level lock must be genuinely released, not leaked: a
        // fresh acquisition for the same (state_directory, vault_id)
        // succeeds immediately, with nothing still holding the old file
        // descriptor open.
        ManagedCheckoutLease::acquire(config.state_directory, config.vault_id).expect(
            "a lease handed back after a concurrent deactivate must not leak the OS-level lock",
        );
    }

    /// The spawned tick task is what actually drives every scheduled Git
    /// turn in production, and nothing covered it: the tests above all call
    /// `tick()` directly, so a scheduler that never got its timer — or a
    /// timer that never reached `tick()` — would have looked entirely
    /// healthy here while no Vault ever polled. Runs on a paused clock, so
    /// it asserts on elapsed tick intervals rather than wall-clock waiting.
    #[tokio::test(start_paused = true)]
    async fn the_spawned_tick_task_requests_a_due_vaults_git_turn_on_its_own() {
        let (coordinator, _worker) = VaultWorkCoordinator::new();
        let scheduler = Arc::new(ManagedGitScheduler::without_durable_state(
            coordinator.clone(),
        ));
        let vault = vault_id("00000000-0000-4000-8000-000000000001");
        scheduler.activate(vault, TEST_POLL_INTERVAL);

        let handle = spawn_scheduler_tick(scheduler.clone(), DEFAULT_TICK_INTERVAL);
        tokio::time::sleep(DEFAULT_TICK_INTERVAL * 3).await;
        tokio::task::yield_now().await;

        assert_eq!(
            coordinator.request(vault, VaultWorkKind::Git),
            ScheduleResult::Coalesced,
            "the spawned tick must have requested the due Vault"
        );
        handle.abort();
    }
    /// The regression this whole change exists for: a Vault redeployed more
    /// often than its poll interval used to restart its countdown on every
    /// start, so a scheduled turn never came due — every observed Git turn on
    /// a daily-polling deployment was an activation or a manual sync. A
    /// reconstructed Vault must resume the interval its last turn armed,
    /// not begin a fresh one.
    #[test]
    fn a_restart_resumes_a_vaults_poll_interval_instead_of_restarting_it() {
        let directory = tempfile::tempdir().expect("temporary state directory");
        let store = Arc::new(VaultRuntimeStateStore::new(
            directory.path().join("vault-runtime.json"),
        ));
        let vault = vault_id("00000000-0000-4000-8000-000000000001");
        let poll_interval = Duration::from_secs(24 * 60 * 60);
        store
            .record_git_turn(
                vault,
                crate::vault_runtime_state::GitTurnRecord {
                    completed_at: std::time::SystemTime::now() - Duration::from_secs(23 * 60 * 60),
                    outcome: crate::vault_runtime_state::GitTurnOutcome::UpToDate,
                },
            )
            .expect("record the previous process's turn");

        // A fresh process over the same file.
        let (coordinator, _worker) = VaultWorkCoordinator::new();
        let scheduler = ManagedGitScheduler::with_state_store(coordinator.clone(), store);
        scheduler.activate(vault, poll_interval);
        scheduler.tick(Instant::now());

        assert_eq!(
            coordinator.request(vault, VaultWorkKind::Git),
            ScheduleResult::Queued,
            "a Vault an hour short of its interval must not be due just because Hatchdoor restarted"
        );
    }

    /// The other half of durability: recording an outcome has to leave
    /// something behind for the next process to resume from. Observed the way
    /// a restart would see it — through a second scheduler over the same
    /// file — rather than by inspecting the file, so the test survives any
    /// change to how the record is stored.
    #[test]
    fn a_completed_turn_is_remembered_so_the_next_process_keeps_counting() {
        let directory = tempfile::tempdir().expect("temporary state directory");
        let store = Arc::new(VaultRuntimeStateStore::new(
            directory.path().join("vault-runtime.json"),
        ));
        let vault = vault_id("00000000-0000-4000-8000-000000000001");
        let poll_interval = Duration::from_secs(24 * 60 * 60);

        let before_restart =
            ManagedGitScheduler::with_state_store(VaultWorkCoordinator::new().0, store.clone());
        before_restart.activate(vault, poll_interval);
        before_restart.record_outcome(vault, &Ok(ManagedGitOutcome::Synchronized));

        let (coordinator, _worker) = VaultWorkCoordinator::new();
        let after_restart = ManagedGitScheduler::with_state_store(coordinator.clone(), store);
        after_restart.activate(vault, poll_interval);
        after_restart.tick(Instant::now());

        assert_eq!(
            coordinator.request(vault, VaultWorkKind::Git),
            ScheduleResult::Queued,
            "a turn that just completed must leave the next process counting down, not due"
        );
    }

    /// A transient failure's backoff stays process-local on purpose: it
    /// throttles a condition that is usually gone by the next start, and a
    /// restart carries no way to verify it is still true. So a restart must
    /// retry at once rather than serve out a backoff it inherited.
    #[test]
    fn a_transient_failure_is_not_remembered_so_a_restart_retries_at_once() {
        let directory = tempfile::tempdir().expect("temporary state directory");
        let store = Arc::new(VaultRuntimeStateStore::new(
            directory.path().join("vault-runtime.json"),
        ));
        let vault = vault_id("00000000-0000-4000-8000-000000000001");

        let before_restart =
            ManagedGitScheduler::with_state_store(VaultWorkCoordinator::new().0, store.clone());
        before_restart.activate(vault, TEST_POLL_INTERVAL);
        before_restart.record_outcome(
            vault,
            &Err(VaultWorkError::new(
                "managed_git_remote_unreachable",
                "the remote went away",
                true,
            )),
        );

        let (coordinator, _worker) = VaultWorkCoordinator::new();
        let after_restart = ManagedGitScheduler::with_state_store(coordinator.clone(), store);
        after_restart.activate(vault, TEST_POLL_INTERVAL);
        after_restart.tick(Instant::now());

        assert_eq!(
            coordinator.request(vault, VaultWorkKind::Git),
            ScheduleResult::Coalesced,
            "a restart must retry a transiently failed Vault immediately"
        );
    }

    /// A non-retryable failure still arms the interval (it waits for a
    /// configuration change, a manual retry, or the normal schedule), so it
    /// is remembered like a success — and carries its code, which is what
    /// lets a restarted instance say *why* a Vault is not synced before its
    /// next turn runs.
    #[test]
    fn a_permanent_failure_is_remembered_with_its_code() {
        let directory = tempfile::tempdir().expect("temporary state directory");
        let store = Arc::new(VaultRuntimeStateStore::new(
            directory.path().join("vault-runtime.json"),
        ));
        let vault = vault_id("00000000-0000-4000-8000-000000000001");

        let scheduler =
            ManagedGitScheduler::with_state_store(VaultWorkCoordinator::new().0, store.clone());
        scheduler.activate(vault, TEST_POLL_INTERVAL);
        scheduler.record_outcome(
            vault,
            &Err(VaultWorkError::new(
                "managed_git_authentication_failed",
                "the token was rejected",
                false,
            )),
        );

        let remembered = store.last_git_turn(vault).expect("a remembered turn");
        let crate::vault_runtime_state::GitTurnOutcome::Failed { code, .. } = remembered.outcome
        else {
            panic!("a non-retryable failure must be remembered as one");
        };
        assert_eq!(code, "managed_git_authentication_failed");
    }

    /// A Vault already past its interval when Hatchdoor starts — the machine
    /// was off over the weekend, or the file predates a long outage — must
    /// sync immediately rather than wait out another full interval.
    #[test]
    fn a_vault_overdue_at_startup_is_due_immediately() {
        let directory = tempfile::tempdir().expect("temporary state directory");
        let store = Arc::new(VaultRuntimeStateStore::new(
            directory.path().join("vault-runtime.json"),
        ));
        let vault = vault_id("00000000-0000-4000-8000-000000000001");
        store
            .record_git_turn(
                vault,
                crate::vault_runtime_state::GitTurnRecord {
                    completed_at: std::time::SystemTime::now() - Duration::from_secs(25 * 60 * 60),
                    outcome: crate::vault_runtime_state::GitTurnOutcome::UpToDate,
                },
            )
            .expect("record an old turn");

        let (coordinator, _worker) = VaultWorkCoordinator::new();
        let scheduler = ManagedGitScheduler::with_state_store(coordinator.clone(), store);
        scheduler.activate(vault, Duration::from_secs(24 * 60 * 60));
        scheduler.tick(Instant::now());

        assert_eq!(
            coordinator.request(vault, VaultWorkKind::Git),
            ScheduleResult::Coalesced,
            "an overdue Vault must sync at startup"
        );
    }

    /// The stored record anchors the *last turn*, never the next deadline, so
    /// an interval edited while Hatchdoor was down takes effect on the next
    /// start. Storing a computed deadline instead would silently serve out
    /// the interval that was configured when it was written.
    #[test]
    fn an_interval_shortened_while_shut_down_takes_effect_on_the_next_start() {
        let directory = tempfile::tempdir().expect("temporary state directory");
        let store = Arc::new(VaultRuntimeStateStore::new(
            directory.path().join("vault-runtime.json"),
        ));
        let vault = vault_id("00000000-0000-4000-8000-000000000001");
        store
            .record_git_turn(
                vault,
                crate::vault_runtime_state::GitTurnRecord {
                    completed_at: std::time::SystemTime::now() - Duration::from_secs(2 * 60 * 60),
                    outcome: crate::vault_runtime_state::GitTurnOutcome::UpToDate,
                },
            )
            .expect("record a turn two hours ago");

        // The definition now says one hour, so two hours ago is already due.
        let (coordinator, _worker) = VaultWorkCoordinator::new();
        let scheduler = ManagedGitScheduler::with_state_store(coordinator.clone(), store);
        scheduler.activate(vault, Duration::from_secs(60 * 60));
        scheduler.tick(Instant::now());

        assert_eq!(
            coordinator.request(vault, VaultWorkKind::Git),
            ScheduleResult::Coalesced,
            "the interval in force now decides the deadline, not the one that was in force when the turn ran"
        );
    }
}
