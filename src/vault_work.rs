//! Fair, instance-wide admission for expensive per-Vault background work.
//!
//! The coordinator owns only disposable in-memory ordering and coalescing.
//! Runtime lifecycle, restart reconstruction, and graceful shutdown belong to
//! the collection lifecycle boundary.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::future::Future;
use std::sync::{Arc, Mutex};

use tokio::sync::Notify;

use crate::vault_registry::VaultId;

/// One expensive operation admitted through the instance-wide worker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum VaultWorkKind {
    /// Git lifecycle work such as acquisition or synchronization.
    Git,
    /// Index construction, including embedding work.
    Index,
    /// Explicit repair work.
    Repair,
}

/// One Vault-qualified operation turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VaultWorkRequest {
    vault_id: VaultId,
    kind: VaultWorkKind,
}

impl VaultWorkRequest {
    fn new(vault_id: VaultId, kind: VaultWorkKind) -> Self {
        Self { vault_id, kind }
    }

    pub fn vault_id(self) -> VaultId {
        self.vault_id
    }

    pub fn kind(self) -> VaultWorkKind {
        self.kind
    }
}

/// A sanitized failure returned by one background operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VaultWorkError {
    code: String,
    message: String,
    retryable: bool,
    detail: Option<VaultWorkErrorDetail>,
}

impl VaultWorkError {
    pub fn new(code: impl Into<String>, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable,
            detail: None,
        }
    }

    /// Attach structured detail alongside `code`/`message`/`retryable` — see
    /// [`VaultWorkErrorDetail`]. Currently only `classify_sync_error`
    /// (`git::managed_task`) populates this, for the handful of managed-Git
    /// sync failure codes a caller genuinely cannot act on from `message`
    /// alone.
    pub fn with_detail(mut self, detail: VaultWorkErrorDetail) -> Self {
        self.detail = Some(detail);
        self
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn retryable(&self) -> bool {
        self.retryable
    }

    pub fn detail(&self) -> Option<&VaultWorkErrorDetail> {
        self.detail.as_ref()
    }
}

/// Structured data behind select [`VaultWorkError`] codes, carried
/// unbounded — `vault_runtime::VaultRuntimeError` is what bounds and
/// publishes it externally (over HTTP and MCP).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VaultWorkErrorDetail {
    /// Affected repository-relative paths, e.g. for
    /// `managed_git_dirty_working_copy` or `managed_git_conflict`.
    AffectedPaths(Vec<String>),
    /// The count behind `managed_git_pull_only_local_commits`.
    LocalCommitsAhead(usize),
}

/// Whether a request added a required turn or joined existing pending work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScheduleResult {
    Queued,
    Coalesced,
    Rejected,
}

/// The Vault-qualified result of exactly one worker turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VaultWorkOutcome {
    pub request: VaultWorkRequest,
    pub result: Result<(), VaultWorkError>,
}

/// Cloneable request side of the instance-wide work coordinator.
#[derive(Clone)]
pub struct VaultWorkCoordinator {
    shared: Arc<SharedQueue>,
}

/// The unique worker side of the coordinator.
///
/// This type is deliberately not cloneable. Calling `run_next` with `&mut
/// self` keeps expensive operations globally serial without introducing a
/// trait or a second execution lane. #90 owns the loop's lifecycle policy.
pub struct VaultWorkWorker {
    shared: Arc<SharedQueue>,
}

struct SharedQueue {
    state: Mutex<QueueState>,
    ready: Notify,
}

#[derive(Default)]
struct QueueState {
    accepting_work: bool,
    drained_vaults: BTreeSet<VaultId>,
    fifo: VecDeque<VaultId>,
    vaults: BTreeMap<VaultId, VaultQueueState>,
}

#[derive(Default)]
struct VaultQueueState {
    active: Option<VaultWorkKind>,
    pending: VecDeque<VaultWorkKind>,
    pending_kinds: BTreeSet<VaultWorkKind>,
    queued: bool,
}

impl VaultWorkCoordinator {
    pub fn new() -> (Self, VaultWorkWorker) {
        let shared = Arc::new(SharedQueue {
            state: Mutex::new(QueueState {
                accepting_work: true,
                ..QueueState::default()
            }),
            ready: Notify::new(),
        });
        (
            Self {
                shared: shared.clone(),
            },
            VaultWorkWorker { shared },
        )
    }

    /// Request one operation for a Vault without adding a duplicate turn.
    ///
    /// A duplicate of active work adds exactly one required rerun. Further
    /// duplicates coalesce into that rerun. Different operation kinds retain
    /// their request order inside the same Vault's single FIFO position.
    pub fn request(&self, vault_id: VaultId, kind: VaultWorkKind) -> ScheduleResult {
        let mut state = self.shared.state.lock().expect("Vault work queue poisoned");
        if !state.accepting_work || state.drained_vaults.contains(&vault_id) {
            return ScheduleResult::Rejected;
        }
        if state
            .vaults
            .get(&vault_id)
            .is_some_and(|vault| vault.pending_kinds.contains(&kind))
        {
            return ScheduleResult::Coalesced;
        }
        state.enqueue(vault_id, kind);
        drop(state);
        self.shared.ready.notify_one();
        ScheduleResult::Queued
    }

    /// Request one operation for a Vault only if that operation is not
    /// already active or pending for it, reporting `Coalesced` when it is.
    ///
    /// Unlike [`Self::request`], this never adds the one guaranteed rerun an
    /// already-active turn would otherwise get. It exists for an automatic,
    /// unattended producer — `git::ManagedGitScheduler::tick`'s due-check —
    /// whose whole job is to not pile turns onto a Vault that is already
    /// working: a rerun queued while a turn is active fires the instant that
    /// turn's execution closure returns, before the turn's outcome has armed
    /// backoff, which defeats backoff entirely on a long turn.
    ///
    /// The check and the enqueue happen under the one lock that owns the
    /// answer, so there is no window between them and no second, separately
    /// tracked notion of "is this Vault busy" to drift out of agreement with
    /// this one (issue #127). A user-driven request — a manual sync or retry
    /// — must still use [`Self::request`] and its guaranteed rerun: someone
    /// explicitly asking for a resync is not a case this skip should swallow.
    pub fn request_if_idle(&self, vault_id: VaultId, kind: VaultWorkKind) -> ScheduleResult {
        let mut state = self.shared.state.lock().expect("Vault work queue poisoned");
        if !state.accepting_work || state.drained_vaults.contains(&vault_id) {
            return ScheduleResult::Rejected;
        }
        if state.vault_has_work(vault_id, kind) {
            return ScheduleResult::Coalesced;
        }
        state.enqueue(vault_id, kind);
        drop(state);
        self.shared.ready.notify_one();
        ScheduleResult::Queued
    }

    /// Whether `kind` is currently active or already pending for `vault_id`.
    ///
    /// A test-only observation of the queue's own per-Vault state. Production
    /// callers that need to act on this answer must not read it and then act:
    /// use [`Self::request_if_idle`], which decides and enqueues under one
    /// lock.
    #[cfg(test)]
    pub fn has_work(&self, vault_id: VaultId, kind: VaultWorkKind) -> bool {
        let state = self.shared.state.lock().expect("Vault work queue poisoned");
        state.vault_has_work(vault_id, kind)
    }

    /// Reopen a Vault after runtime activation or restart reconstruction.
    pub fn activate_vault(&self, vault_id: VaultId) {
        let mut state = self.shared.state.lock().expect("Vault work queue poisoned");
        if state.accepting_work {
            state.drained_vaults.remove(&vault_id);
        }
    }

    /// Stop accepting work for one Vault and discard its queued turns.
    ///
    /// A currently active turn is never force-cancelled. It retains the worker
    /// until it returns at its own safe operation boundary.
    pub fn drain_vault(&self, vault_id: VaultId) {
        let mut state = self.shared.state.lock().expect("Vault work queue poisoned");
        state.drained_vaults.insert(vault_id);
        state.discard_pending(vault_id);
        drop(state);
        self.shared.ready.notify_waiters();
    }

    /// Stop scheduling globally and discard all queued turns.
    ///
    /// Active work is allowed to complete. The queue itself is disposable and
    /// is rebuilt from durable Vault state on the next startup.
    pub fn shutdown(&self) {
        let mut state = self.shared.state.lock().expect("Vault work queue poisoned");
        state.accepting_work = false;
        state.discard_all_pending();
        drop(state);
        self.shared.ready.notify_waiters();
    }

    /// Wait only for an already-active turn of one drained Vault.
    pub async fn wait_for_vault_safe_boundary(&self, vault_id: VaultId) {
        loop {
            let notified = self.shared.ready.notified();
            if self
                .shared
                .state
                .lock()
                .expect("Vault work queue poisoned")
                .vault_is_idle(vault_id)
            {
                return;
            }
            notified.await;
        }
    }

    /// Wait only for active work after shutdown has discarded the queue.
    pub async fn wait_for_shutdown_boundary(&self) {
        loop {
            let notified = self.shared.ready.notified();
            if self
                .shared
                .state
                .lock()
                .expect("Vault work queue poisoned")
                .shutdown_is_quiescent()
            {
                return;
            }
            notified.await;
        }
    }
}

impl VaultWorkWorker {
    /// Wait for and execute one globally serialized operation turn.
    ///
    /// Returned failures complete the turn exactly like successes, allowing
    /// the next Vault to proceed. Panics are intentionally outside this
    /// returned-failure contract and remain task failures for the lifecycle
    /// owner to handle.
    pub async fn run_next<F, Fut>(&mut self, execute: F) -> Option<VaultWorkOutcome>
    where
        F: FnOnce(VaultWorkRequest) -> Fut,
        Fut: Future<Output = Result<(), VaultWorkError>>,
    {
        let request = self.next_request().await?;
        let result = execute(request).await;
        self.shared
            .state
            .lock()
            .expect("Vault work queue poisoned")
            .complete(request);
        self.shared.ready.notify_waiters();
        Some(VaultWorkOutcome { request, result })
    }

    async fn next_request(&self) -> Option<VaultWorkRequest> {
        loop {
            let notified = self.shared.ready.notified();
            {
                let mut state = self.shared.state.lock().expect("Vault work queue poisoned");
                if let Some(request) = state.take_next() {
                    return Some(request);
                }
                if !state.accepting_work {
                    return None;
                }
            }
            notified.await;
        }
    }
}

impl QueueState {
    /// Whether `kind` is currently active or already pending for `vault_id`.
    fn vault_has_work(&self, vault_id: VaultId, kind: VaultWorkKind) -> bool {
        self.vaults
            .get(&vault_id)
            .is_some_and(|vault| vault.active == Some(kind) || vault.pending_kinds.contains(&kind))
    }

    /// Append `kind` to `vault_id`'s pending work, taking a FIFO position for
    /// the Vault if it does not already hold one. The caller has already
    /// decided this turn is required.
    fn enqueue(&mut self, vault_id: VaultId, kind: VaultWorkKind) {
        let vault = self.vaults.entry(vault_id).or_default();
        vault.pending.push_back(kind);
        vault.pending_kinds.insert(kind);
        if !vault.queued {
            vault.queued = true;
            self.fifo.push_back(vault_id);
        }
    }

    fn take_next(&mut self) -> Option<VaultWorkRequest> {
        let vault_id = self.fifo.pop_front()?;
        let (kind, requeue) = {
            let vault = self
                .vaults
                .get_mut(&vault_id)
                .expect("queued Vault work state missing");
            vault.queued = false;
            let kind = vault
                .pending
                .pop_front()
                .expect("queued Vault has no pending work");
            vault.pending_kinds.remove(&kind);
            debug_assert!(vault.active.is_none());
            vault.active = Some(kind);
            (kind, !vault.pending.is_empty())
        };
        if requeue {
            self.vaults
                .get_mut(&vault_id)
                .expect("active Vault work state missing")
                .queued = true;
            self.fifo.push_back(vault_id);
        }
        Some(VaultWorkRequest::new(vault_id, kind))
    }

    fn complete(&mut self, request: VaultWorkRequest) {
        let remove = {
            let vault = self
                .vaults
                .get_mut(&request.vault_id)
                .expect("active Vault work state missing");
            debug_assert_eq!(vault.active, Some(request.kind));
            vault.active = None;
            !vault.queued && vault.pending.is_empty()
        };
        if remove {
            self.vaults.remove(&request.vault_id);
        }
    }

    fn discard_pending(&mut self, vault_id: VaultId) {
        self.fifo.retain(|queued_vault| *queued_vault != vault_id);
        let remove = match self.vaults.get_mut(&vault_id) {
            Some(vault) => {
                vault.pending.clear();
                vault.pending_kinds.clear();
                vault.queued = false;
                vault.active.is_none()
            }
            None => false,
        };
        if remove {
            self.vaults.remove(&vault_id);
        }
    }

    fn discard_all_pending(&mut self) {
        self.fifo.clear();
        for vault in self.vaults.values_mut() {
            vault.pending.clear();
            vault.pending_kinds.clear();
            vault.queued = false;
        }
        self.vaults.retain(|_, vault| vault.active.is_some());
    }

    fn vault_is_idle(&self, vault_id: VaultId) -> bool {
        !self.vaults.contains_key(&vault_id)
    }

    fn shutdown_is_quiescent(&self) -> bool {
        !self.accepting_work && self.vaults.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use tokio::sync::Notify;
    use tokio::time::timeout;

    use super::{
        ScheduleResult, VaultWorkCoordinator, VaultWorkError, VaultWorkKind, VaultWorkRequest,
    };
    use crate::vault_registry::VaultId;

    fn vault_id(value: &str) -> VaultId {
        value.parse().expect("valid test Vault ID")
    }

    #[tokio::test]
    async fn has_work_reflects_pending_and_active_state_for_the_requested_kind_only() {
        let vault = vault_id("00000000-0000-4000-8000-000000000001");
        let other = vault_id("00000000-0000-4000-8000-000000000002");
        let (coordinator, mut worker) = VaultWorkCoordinator::new();

        assert!(!coordinator.has_work(vault, VaultWorkKind::Git));

        coordinator.request(vault, VaultWorkKind::Git);
        assert!(coordinator.has_work(vault, VaultWorkKind::Git));
        assert!(
            !coordinator.has_work(vault, VaultWorkKind::Index),
            "a different kind for the same Vault must not be reported as having work"
        );
        assert!(
            !coordinator.has_work(other, VaultWorkKind::Git),
            "a different Vault must not be reported as having work"
        );

        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let running = tokio::spawn({
            let started = started.clone();
            let release = release.clone();
            async move {
                worker
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
                worker
            }
        });
        started.notified().await;
        assert!(
            coordinator.has_work(vault, VaultWorkKind::Git),
            "an active (not just pending) turn must also report has_work"
        );
        release.notify_one();
        let mut worker = running.await.expect("worker task");
        assert!(!coordinator.has_work(vault, VaultWorkKind::Git));

        // Drain the worker so it does not outlive the test.
        let _ = tokio::time::timeout(
            Duration::from_millis(1),
            worker.run_next(|_| async { Ok::<(), VaultWorkError>(()) }),
        )
        .await;
    }

    #[tokio::test]
    async fn request_if_idle_adds_no_rerun_while_the_same_kind_is_active() {
        let vault = vault_id("00000000-0000-4000-8000-000000000001");
        let (coordinator, mut worker) = VaultWorkCoordinator::new();
        assert_eq!(
            coordinator.request_if_idle(vault, VaultWorkKind::Git),
            ScheduleResult::Queued,
            "an idle Vault's first automatic request is admitted"
        );
        assert_eq!(
            coordinator.request_if_idle(vault, VaultWorkKind::Git),
            ScheduleResult::Coalesced,
            "a pending turn is not duplicated"
        );

        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let running = tokio::spawn({
            let started = started.clone();
            let release = release.clone();
            async move {
                worker
                    .run_next(move |_| {
                        let started = started.clone();
                        let release = release.clone();
                        async move {
                            started.notify_one();
                            release.notified().await;
                            Ok::<(), VaultWorkError>(())
                        }
                    })
                    .await
                    .expect("active turn");
                worker
            }
        });

        started.notified().await;
        assert_eq!(
            coordinator.request_if_idle(vault, VaultWorkKind::Git),
            ScheduleResult::Coalesced,
            "an automatic request must not pre-queue a rerun that would fire \
             before the active turn's outcome can arm backoff"
        );
        assert_eq!(
            coordinator.request_if_idle(vault, VaultWorkKind::Index),
            ScheduleResult::Queued,
            "a different kind for the same Vault is unaffected"
        );
        release.notify_one();

        let mut worker = running.await.expect("worker task");
        let next = worker
            .run_next(|_| async { Ok::<(), VaultWorkError>(()) })
            .await
            .expect("the Index turn queued during the active Git turn");
        assert_eq!(
            next.request,
            VaultWorkRequest::new(vault, VaultWorkKind::Index)
        );
        assert!(
            timeout(
                Duration::from_millis(25),
                worker.run_next(|_| async { Ok::<(), VaultWorkError>(()) })
            )
            .await
            .is_err(),
            "no Git rerun was queued by the skipped automatic requests"
        );
    }

    #[tokio::test]
    async fn request_if_idle_is_rejected_for_a_drained_or_shut_down_vault() {
        let vault = vault_id("00000000-0000-4000-8000-000000000001");
        let (coordinator, _worker) = VaultWorkCoordinator::new();
        coordinator.drain_vault(vault);
        assert_eq!(
            coordinator.request_if_idle(vault, VaultWorkKind::Git),
            ScheduleResult::Rejected
        );

        let other = vault_id("00000000-0000-4000-8000-000000000002");
        coordinator.shutdown();
        assert_eq!(
            coordinator.request_if_idle(other, VaultWorkKind::Git),
            ScheduleResult::Rejected
        );
    }

    #[tokio::test]
    async fn fifo_turns_are_fair_and_deterministic_across_vaults() {
        let first = vault_id("00000000-0000-4000-8000-000000000001");
        let second = vault_id("00000000-0000-4000-8000-000000000002");
        let (coordinator, mut worker) = VaultWorkCoordinator::new();

        assert_eq!(
            coordinator.request(first, VaultWorkKind::Index),
            ScheduleResult::Queued
        );
        assert_eq!(
            coordinator.request(first, VaultWorkKind::Index),
            ScheduleResult::Coalesced,
            "a queued operation must not add a second Vault position"
        );
        assert_eq!(
            coordinator.request(second, VaultWorkKind::Index),
            ScheduleResult::Queued
        );
        assert_eq!(
            coordinator.request(first, VaultWorkKind::Git),
            ScheduleResult::Queued
        );

        let mut observed = Vec::new();
        for _ in 0..3 {
            let outcome = worker
                .run_next(|_| async { Ok::<(), VaultWorkError>(()) })
                .await
                .expect("queued turn");
            outcome.result.expect("work succeeds");
            observed.push(outcome.request);
        }

        assert_eq!(
            observed,
            vec![
                VaultWorkRequest::new(first, VaultWorkKind::Index),
                VaultWorkRequest::new(second, VaultWorkKind::Index),
                VaultWorkRequest::new(first, VaultWorkKind::Git),
            ]
        );
    }

    #[tokio::test]
    async fn repeated_active_requests_coalesce_to_one_required_rerun() {
        let vault = vault_id("00000000-0000-4000-8000-000000000001");
        let (coordinator, mut worker) = VaultWorkCoordinator::new();
        assert_eq!(
            coordinator.request(vault, VaultWorkKind::Index),
            ScheduleResult::Queued
        );

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
        assert_eq!(
            coordinator.request(vault, VaultWorkKind::Index),
            ScheduleResult::Queued,
            "the first request during active work retains one rerun"
        );
        assert_eq!(
            coordinator.request(vault, VaultWorkKind::Index),
            ScheduleResult::Coalesced
        );
        assert_eq!(
            coordinator.request(vault, VaultWorkKind::Index),
            ScheduleResult::Coalesced
        );
        release.notify_one();

        let (mut worker, first_outcome) = running.await.expect("worker task");
        assert_eq!(
            first_outcome.request,
            VaultWorkRequest::new(vault, VaultWorkKind::Index)
        );
        first_outcome.result.expect("first run succeeds");
        let rerun = worker
            .run_next(|_| async { Ok::<(), VaultWorkError>(()) })
            .await
            .expect("rerun");
        assert_eq!(
            rerun.request,
            VaultWorkRequest::new(vault, VaultWorkKind::Index)
        );
        rerun.result.expect("rerun succeeds");
        assert!(
            timeout(
                Duration::from_millis(25),
                worker.run_next(|_| async { Ok::<(), VaultWorkError>(()) })
            )
            .await
            .is_err(),
            "the burst must converge after exactly one rerun"
        );
    }

    #[tokio::test]
    async fn returned_failure_is_vault_qualified_and_releases_the_worker() {
        let failing = vault_id("00000000-0000-4000-8000-000000000001");
        let healthy = vault_id("00000000-0000-4000-8000-000000000002");
        let (coordinator, mut worker) = VaultWorkCoordinator::new();
        coordinator.request(failing, VaultWorkKind::Repair);
        coordinator.request(healthy, VaultWorkKind::Index);

        let failed = worker
            .run_next(|_| async {
                Err::<(), VaultWorkError>(VaultWorkError::new(
                    "repair_failed",
                    "repair could not complete",
                    true,
                ))
            })
            .await
            .expect("failed turn");
        assert_eq!(failed.request.vault_id(), failing);
        assert_eq!(failed.request.kind(), VaultWorkKind::Repair);
        assert_eq!(
            failed.result.expect_err("repair fails").code(),
            "repair_failed"
        );

        let succeeded = worker
            .run_next(|_| async { Ok::<(), VaultWorkError>(()) })
            .await
            .expect("healthy turn");
        assert_eq!(succeeded.request.vault_id(), healthy);
        assert_eq!(succeeded.request.kind(), VaultWorkKind::Index);
        succeeded.result.expect("healthy Vault proceeds");
    }

    #[tokio::test]
    async fn draining_one_vault_rejects_new_work_and_preserves_another_vaults_turn() {
        let draining = vault_id("00000000-0000-4000-8000-000000000001");
        let healthy = vault_id("00000000-0000-4000-8000-000000000002");
        let (coordinator, mut worker) = VaultWorkCoordinator::new();
        coordinator.request(draining, VaultWorkKind::Index);
        coordinator.request(healthy, VaultWorkKind::Index);

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
                            assert_eq!(request.vault_id(), draining);
                            started.notify_one();
                            release.notified().await;
                            Ok::<(), VaultWorkError>(())
                        }
                    })
                    .await;
                (worker, outcome)
            }
        });

        started.notified().await;
        coordinator.request(draining, VaultWorkKind::Git);
        coordinator.drain_vault(draining);
        assert_eq!(
            coordinator.request(draining, VaultWorkKind::Repair),
            ScheduleResult::Rejected
        );

        release.notify_one();
        let (mut worker, first) = running.await.expect("worker task");
        first
            .expect("active work reaches its safe boundary")
            .result
            .expect("active work succeeds");

        let healthy_turn = worker
            .run_next(|_| async { Ok::<(), VaultWorkError>(()) })
            .await
            .expect("healthy Vault remains queued");
        assert_eq!(healthy_turn.request.vault_id(), healthy);
        assert!(
            timeout(
                Duration::from_millis(25),
                worker.run_next(|_| async { Ok::<(), VaultWorkError>(()) })
            )
            .await
            .is_err(),
            "draining discards the target Vault's queued rerun"
        );
    }

    #[tokio::test]
    async fn shutdown_discards_pending_turns_without_waiting_for_the_queue_to_drain() {
        let first = vault_id("00000000-0000-4000-8000-000000000001");
        let second = vault_id("00000000-0000-4000-8000-000000000002");
        let (coordinator, mut worker) = VaultWorkCoordinator::new();
        coordinator.request(first, VaultWorkKind::Index);
        coordinator.request(second, VaultWorkKind::Git);

        coordinator.shutdown();

        assert_eq!(
            worker
                .run_next(|_| async { Ok::<(), VaultWorkError>(()) })
                .await,
            None,
            "shutdown drops queued work because durable state reconstructs it at restart"
        );
        assert_eq!(
            coordinator.request(first, VaultWorkKind::Repair),
            ScheduleResult::Rejected
        );
    }
}
