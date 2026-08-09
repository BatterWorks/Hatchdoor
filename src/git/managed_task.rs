//! Per-Vault managed-Git scheduling: daily polling, manual Sync/Retry now,
//! and bounded exponential backoff on transient failure — plus the concrete
//! acquire-or-reuse-then-synchronize turn `VaultWorkKind::Git` executes.
//!
//! This boundary decides *when* to request a Git turn for a Vault and *what*
//! one turn does. It does not know about `VaultControlBlock`, the Vault
//! registry, or the coordinator's worker loop: runtime composition (#97's
//! `src/vault_runtime.rs` seam) resolves a Vault's current configuration,
//! runs the turn, and publishes the result.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::vault_registry::{HttpsCredentials, VaultGitMode, VaultId};
use crate::vault_work::{ScheduleResult, VaultWorkCoordinator, VaultWorkError, VaultWorkKind};

use super::managed_checkout::{
    ManagedCheckoutError, ManagedCheckoutLease, ManagedCheckoutRequest, ManagedHttpsCredentials,
    acquire_or_reuse,
};
use super::managed_sync::{
    ManagedSyncConfig, ManagedSyncError, ManagedSyncMode, ManagedSyncOutcome,
    synchronize_managed_checkout,
};

/// How long a healthy or non-retryably-failed (including authentication)
/// managed Vault waits before its next scheduled Git turn. Not yet
/// user-configurable: Phase 1 has no UI-managed Vault configuration, so every
/// managed-Git Vault currently shares this one default.
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Backoff bounds for re-attempting a turn that failed for a retryable
/// (transient) reason, mirroring the legacy single-Vault task's bounds
/// (`git/task.rs::RETRY_BASE`/`RETRY_MAX`) at a coarser scale appropriate for
/// a shared, fair, instance-wide worker.
const BACKOFF_BASE: Duration = Duration::from_secs(30);
const BACKOFF_MAX: Duration = Duration::from_secs(60 * 60);

/// How often [`spawn_scheduler_tick`] checks for due Vaults in production.
/// Well below `BACKOFF_BASE`, so a transient failure's backoff resolves with
/// reasonable promptness rather than only on the next daily tick.
pub const DEFAULT_TICK_INTERVAL: Duration = Duration::from_secs(15);

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
pub fn run_existing_git_remote_turn(
    repository_path: PathBuf,
    vault_path: PathBuf,
    branch: Option<String>,
    mode: VaultGitMode,
    credentials: Option<HttpsCredentials>,
    author_name: String,
    author_email: String,
) -> Result<ManagedGitOutcome, VaultWorkError> {
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
/// `pub(crate)`: also used by `vault_runtime::dispatch_managed_git_turn_with`
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
fn classify_sync_error(error: ManagedSyncError) -> VaultWorkError {
    use ManagedSyncError::*;
    let (code, retryable) = match error {
        Validation => ("managed_git_validation_failed", false),
        DirtyWorkingCopy { .. } => ("managed_git_dirty_working_copy", false),
        LocalCommits { .. } => ("managed_git_pull_only_local_commits", false),
        Conflict { .. } => ("managed_git_conflict", false),
        // The bounded fetch-integrate-push replay inside
        // `synchronize_managed_checkout` already absorbed a couple of
        // immediate retries; reaching here means the race is still live,
        // which is exactly the kind of transient condition backoff exists
        // for.
        PushRace => ("managed_git_push_race_exhausted", true),
        Authentication => ("managed_git_authentication_failed", false),
        Remote => ("managed_git_remote_unreachable", true),
    };
    VaultWorkError::new(code, error.to_string(), retryable)
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
    lease: Option<ManagedCheckoutLease>,
}

/// Decides *when* to request `VaultWorkKind::Git` for each active managed-Git
/// Vault: a daily tick, an immediate manual Sync/Retry-now, or a bounded
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
    poll_interval: Duration,
    entries: Mutex<BTreeMap<VaultId, VaultScheduleEntry>>,
}

impl ManagedGitScheduler {
    pub fn new(coordinator: VaultWorkCoordinator) -> Self {
        Self::with_poll_interval(coordinator, DEFAULT_POLL_INTERVAL)
    }

    pub fn with_poll_interval(coordinator: VaultWorkCoordinator, poll_interval: Duration) -> Self {
        Self {
            coordinator,
            poll_interval,
            entries: Mutex::new(BTreeMap::new()),
        }
    }

    /// Register a Vault so it participates in scheduled polling, due
    /// immediately. Idempotent: re-activating an already-tracked Vault
    /// leaves its current schedule (including an in-progress backoff and any
    /// held checkout lease) untouched, so a `reconcile()` that retains an
    /// unchanged Vault does not reset it.
    pub fn activate(&self, vault_id: VaultId) {
        let mut entries = self.entries.lock().expect("managed Git scheduler poisoned");
        entries.entry(vault_id).or_insert(VaultScheduleEntry {
            schedule: ScheduleState {
                next_attempt: Instant::now(),
                backoff: None,
            },
            lease: None,
        });
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

    /// Request an immediate turn for exactly one Vault, bypassing its
    /// schedule. Registers the Vault if it was not already tracked, so a
    /// manual sync can never be silently dropped. Coalesces with any
    /// already-pending Git turn for that Vault.
    pub fn sync_now(&self, vault_id: VaultId) -> ScheduleResult {
        self.activate(vault_id);
        self.coordinator.request(vault_id, VaultWorkKind::Git)
    }

    /// Same admitted operation as [`Self::sync_now`]; kept as a distinctly
    /// named entry point because "retry a failed Vault" and "sync a healthy
    /// one" are different user intents even though both resolve to the same
    /// coordinator request.
    pub fn retry_now(&self, vault_id: VaultId) -> ScheduleResult {
        self.sync_now(vault_id)
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
        let mut entries = self.entries.lock().expect("managed Git scheduler poisoned");
        let Some(entry) = entries.get_mut(&vault_id) else {
            return;
        };
        let schedule = &mut entry.schedule;
        match result {
            Ok(_) => {
                schedule.backoff = None;
                schedule.next_attempt = Instant::now() + self.poll_interval;
            }
            Err(error) if error.retryable() => {
                let next_backoff = schedule
                    .backoff
                    .map_or(BACKOFF_BASE, |previous| (previous * 2).min(BACKOFF_MAX));
                schedule.backoff = Some(next_backoff);
                schedule.next_attempt = Instant::now() + next_backoff;
            }
            Err(_) => {
                schedule.backoff = None;
                schedule.next_attempt = Instant::now() + self.poll_interval;
            }
        }
    }

    /// Request a Git turn for every tracked Vault whose schedule is due as of
    /// `now`. Naturally coalesces with the coordinator: a Vault whose
    /// previous turn is still active or already queued is a no-op
    /// (`ScheduleResult::Coalesced`).
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
            self.coordinator.request(vault_id, VaultWorkKind::Git);
        }
    }
}

/// Spawn the periodic tick that keeps every tracked Vault's daily schedule
/// (and any armed backoff) moving forward. `tick_interval` controls how often
/// the schedule is *checked*, not how often a Vault actually syncs — keep it
/// well below `poll_interval` and `BACKOFF_BASE`. Aborting the returned
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

    use git2::{Repository, Signature};
    use tempfile::TempDir;

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

    fn scheduler() -> (VaultWorkCoordinator, ManagedGitScheduler) {
        let (coordinator, _worker) = VaultWorkCoordinator::new();
        let scheduler =
            ManagedGitScheduler::with_poll_interval(coordinator.clone(), Duration::from_secs(3600));
        (coordinator, scheduler)
    }

    fn vault_id(value: &str) -> VaultId {
        value.parse().expect("valid test Vault ID")
    }

    #[test]
    fn activate_requests_an_immediate_turn_on_the_next_tick() {
        let (coordinator, scheduler) = scheduler();
        let vault = vault_id("00000000-0000-4000-8000-000000000001");

        scheduler.activate(vault);
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
        scheduler.activate(vault);
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
        scheduler.activate(vault);
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

        scheduler.activate(vault);

        let after_reactivate = {
            let entries = scheduler.entries.lock().expect("scheduler entries");
            entries[&vault].schedule.next_attempt
        };
        assert_eq!(armed_backoff, after_reactivate);
    }

    #[test]
    fn deactivate_stops_tracking_so_a_later_tick_does_not_request_it() {
        let (coordinator, scheduler) = scheduler();
        let vault = vault_id("00000000-0000-4000-8000-000000000001");
        scheduler.activate(vault);

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

        assert_eq!(scheduler.sync_now(vault), ScheduleResult::Queued);
        assert_eq!(
            coordinator.request(vault, VaultWorkKind::Git),
            ScheduleResult::Coalesced,
            "sync_now already queued this Vault's Git turn"
        );
        // record_outcome now finds the Vault tracked (activated by sync_now).
        scheduler.record_outcome(vault, &Ok(ManagedGitOutcome::UpToDate));
    }

    #[test]
    fn a_retryable_failure_backs_off_exponentially_and_a_success_resets_it() {
        let (_coordinator, scheduler) = scheduler();
        let vault = vault_id("00000000-0000-4000-8000-000000000001");
        scheduler.activate(vault);
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
        assert!(
            after_success.next_attempt
                >= before + DEFAULT_POLL_INTERVAL.min(Duration::from_secs(3600))
                    - Duration::from_secs(1)
        );
    }

    #[test]
    fn a_non_retryable_failure_including_authentication_waits_for_the_normal_schedule_not_backoff()
    {
        let (_coordinator, scheduler) = scheduler();
        let vault = vault_id("00000000-0000-4000-8000-000000000001");
        scheduler.activate(vault);
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
        assert!(
            schedule.next_attempt >= before + Duration::from_secs(3600) - Duration::from_secs(1)
        );
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
    /// same ones `dispatch_managed_git_turn_with` uses in production), a
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
        let scheduler = ManagedGitScheduler::new(coordinator);
        scheduler.activate(config.vault_id);

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
        let scheduler = ManagedGitScheduler::new(coordinator);
        scheduler.activate(config.vault_id);

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
}
