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
use std::path::PathBuf;
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
pub fn run_managed_git_turn(
    config: &ManagedGitTurnConfig,
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

    let lease = ManagedCheckoutLease::acquire(config.state_directory.clone(), config.vault_id)
        .map_err(classify_checkout_error)?;
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
    let checkout = acquire_or_reuse(&lease, &request).map_err(classify_checkout_error)?;

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

/// Classify a checkout failure. `retryable` decides whether the scheduler
/// backs off and retries automatically or waits for the normal schedule, a
/// manual retry, a configuration change, or a restart.
fn classify_checkout_error(error: ManagedCheckoutError) -> VaultWorkError {
    use ManagedCheckoutError::*;
    let (code, retryable) = match error {
        StateDirectoryUnavailable => ("managed_git_state_directory_unavailable", false),
        // Contention on the process-lifetime lock. In normal operation the
        // coordinator already keeps at most one turn per Vault in flight, so
        // this indicates another process (or a concurrent Hatchdoor
        // instance) holds it; worth a bounded automatic retry.
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

/// Decides *when* to request `VaultWorkKind::Git` for each active managed-Git
/// Vault: a daily tick, an immediate manual Sync/Retry-now, or a bounded
/// exponential backoff after a transient failure.
///
/// One instance drives every managed-Git Vault in the process — mirroring
/// the coordinator's own single-worker design, this adds no per-Vault
/// execution lane (ADR-13).
pub struct ManagedGitScheduler {
    coordinator: VaultWorkCoordinator,
    poll_interval: Duration,
    state: Mutex<BTreeMap<VaultId, ScheduleState>>,
}

impl ManagedGitScheduler {
    pub fn new(coordinator: VaultWorkCoordinator) -> Self {
        Self::with_poll_interval(coordinator, DEFAULT_POLL_INTERVAL)
    }

    pub fn with_poll_interval(coordinator: VaultWorkCoordinator, poll_interval: Duration) -> Self {
        Self {
            coordinator,
            poll_interval,
            state: Mutex::new(BTreeMap::new()),
        }
    }

    /// Register a Vault so it participates in scheduled polling, due
    /// immediately. Idempotent: re-activating an already-tracked Vault
    /// leaves its current schedule (including an in-progress backoff)
    /// untouched, so a `reconcile()` that retains an unchanged Vault does
    /// not reset it.
    pub fn activate(&self, vault_id: VaultId) {
        let mut state = self.state.lock().expect("managed Git scheduler poisoned");
        state.entry(vault_id).or_insert(ScheduleState {
            next_attempt: Instant::now(),
            backoff: None,
        });
    }

    /// Stop tracking a Vault (disabled, disconnected, or retired). Any
    /// coordinator-side pending work is discarded separately by
    /// `VaultWorkCoordinator::drain_vault`.
    pub fn deactivate(&self, vault_id: VaultId) {
        self.state
            .lock()
            .expect("managed Git scheduler poisoned")
            .remove(&vault_id);
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
        let mut state = self.state.lock().expect("managed Git scheduler poisoned");
        let Some(entry) = state.get_mut(&vault_id) else {
            return;
        };
        match result {
            Ok(_) => {
                entry.backoff = None;
                entry.next_attempt = Instant::now() + self.poll_interval;
            }
            Err(error) if error.retryable() => {
                let next_backoff = entry
                    .backoff
                    .map_or(BACKOFF_BASE, |previous| (previous * 2).min(BACKOFF_MAX));
                entry.backoff = Some(next_backoff);
                entry.next_attempt = Instant::now() + next_backoff;
            }
            Err(_) => {
                entry.backoff = None;
                entry.next_attempt = Instant::now() + self.poll_interval;
            }
        }
    }

    /// Request a Git turn for every tracked Vault whose schedule is due as of
    /// `now`. Naturally coalesces with the coordinator: a Vault whose
    /// previous turn is still active or already queued is a no-op
    /// (`ScheduleResult::Coalesced`).
    pub fn tick(&self, now: Instant) {
        let due = {
            let state = self.state.lock().expect("managed Git scheduler poisoned");
            state
                .iter()
                .filter(|(_, schedule)| schedule.next_attempt <= now)
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

        let outcome = run_managed_git_turn(&config).expect("first turn acquires and syncs");

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
        run_managed_git_turn(&config).expect("first turn");

        let repository_root = config
            .state_directory
            .join("vaults")
            .join(config.vault_id.to_string())
            .join("repository");
        std::fs::write(repository_root.join("vault/Local.md"), "local\n").expect("local edit");

        let outcome = run_managed_git_turn(&config).expect("second turn reuses the checkout");

        assert_eq!(outcome, ManagedGitOutcome::Synchronized);
    }

    #[test]
    fn run_managed_git_turn_rejects_local_history_mode_without_touching_the_filesystem() {
        let (_root, config) = fixture(VaultGitMode::LocalHistory);

        let error = run_managed_git_turn(&config).expect_err("Local history has no remote");

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
            let state = scheduler.state.lock().expect("scheduler state");
            state[&vault].next_attempt
        };

        scheduler.activate(vault);

        let after_reactivate = {
            let state = scheduler.state.lock().expect("scheduler state");
            state[&vault].next_attempt
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
            let state = scheduler.state.lock().expect("scheduler state");
            state[&vault].next_attempt.duration_since(before)
        };
        scheduler.record_outcome(vault, &transient);
        let second_backoff = {
            let state = scheduler.state.lock().expect("scheduler state");
            state[&vault].next_attempt.duration_since(before)
        };
        assert!(
            second_backoff > first_backoff,
            "a repeated transient failure must back off further: {first_backoff:?} -> {second_backoff:?}"
        );

        scheduler.record_outcome(vault, &Ok(ManagedGitOutcome::UpToDate));
        let after_success = {
            let state = scheduler.state.lock().expect("scheduler state");
            state[&vault]
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

        let entry = {
            let state = scheduler.state.lock().expect("scheduler state");
            state[&vault]
        };
        assert!(
            entry.backoff.is_none(),
            "authentication failures must not arm exponential backoff"
        );
        assert!(entry.next_attempt >= before + Duration::from_secs(3600) - Duration::from_secs(1));
    }

    #[test]
    fn record_outcome_is_a_no_op_for_an_untracked_vault() {
        let (_coordinator, scheduler) = scheduler();
        let vault = vault_id("00000000-0000-4000-8000-000000000001");

        // No panic, no tracked entry created.
        scheduler.record_outcome(vault, &Ok(ManagedGitOutcome::UpToDate));

        let state = scheduler.state.lock().expect("scheduler state");
        assert!(!state.contains_key(&vault));
    }
}
