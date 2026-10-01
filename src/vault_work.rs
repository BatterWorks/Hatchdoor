//! Fair, instance-wide admission for expensive per-Vault background work.
//!
//! Work is admitted in two lanes (ADR-31). Index and Repair turns share one
//! instance-wide slot. Git, Commit and Recovery turns never wait for an Index
//! turn: up to four Vaults run them at once, each Vault one turn at a time.
//! Both lanes keep request order and per-Vault coalescing. The same-Vault
//! guard between an Index turn and a Git turn is the Vault's mutation lock,
//! not this queue.
//!
//! The coordinator owns only disposable in-memory ordering and coalescing.
//! Runtime lifecycle, restart reconstruction, and graceful shutdown belong to
//! the collection lifecycle boundary.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::future::Future;
use std::sync::{Arc, Mutex};

use tokio::sync::Notify;

use crate::vault_registry::VaultId;

/// One expensive operation admitted through the coordinator. Its kind
/// decides which lane admits it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum VaultWorkKind {
    /// Git lifecycle work such as acquisition or synchronization.
    Git,
    /// A purely local Git commit of whatever has changed in the Vault's own
    /// subtree. Never opens a network connection, which is why it is a kind
    /// of its own rather than a flavour of [`Self::Git`]: committing is free
    /// and can run on every change, while talking to a remote costs a round
    /// trip and stays on the Vault's configured schedule. Coalescing the two
    /// together would let a due sync swallow a pending commit, or the other
    /// way round.
    Commit,
    /// Publishing a Two-way Vault's side of a sync conflict to its recovery
    /// branch, on an operator's request (ADR-30). A kind of its own so it
    /// never coalesces with a scheduled sync: a sync due at the same moment
    /// must not swallow the request, nor the request a sync.
    Recovery,
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

    /// A request for a test to describe an outcome with, without taking it
    /// from a coordinator.
    #[cfg(test)]
    pub(crate) fn for_tests(vault_id: VaultId, kind: VaultWorkKind) -> Self {
        Self::new(vault_id, kind)
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

/// The unique admission side of the coordinator.
///
/// This type is deliberately not cloneable: one loop takes turns from it.
/// Each admitted [`VaultWorkTurn`] holds its lane slot until it is dropped,
/// so the caller may run several at once and the lanes still enforce how
/// many may overlap (ADR-31).
pub struct VaultWorkWorker {
    shared: Arc<SharedQueue>,
}

/// One admitted turn. It holds its lane slot and its Vault's place in that
/// lane until it is dropped, so a caller that publishes the outcome before
/// dropping it keeps the next turn in that lane waiting for the publication,
/// as the single serial loop used to.
pub struct VaultWorkTurn {
    shared: Arc<SharedQueue>,
    request: VaultWorkRequest,
}

/// How many Vaults may run Git work at once (ADR-31 decision 3). A fixed
/// constant, not a setting (ADR-14).
const GIT_LANE_WIDTH: usize = 4;

/// Where a kind of work is admitted. Indexing is CPU-bound and the embedder
/// already serializes inference, so it keeps one instance-wide slot; Git
/// work is network- and disk-bound, so a few Vaults may run it side by side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lane {
    Index = 0,
    Git = 1,
}

impl Lane {
    fn of(kind: VaultWorkKind) -> Self {
        match kind {
            VaultWorkKind::Index | VaultWorkKind::Repair => Self::Index,
            VaultWorkKind::Git | VaultWorkKind::Commit | VaultWorkKind::Recovery => Self::Git,
        }
    }

    /// How many turns this lane admits at once, across every Vault.
    fn width(self) -> usize {
        match self {
            Self::Index => 1,
            Self::Git => GIT_LANE_WIDTH,
        }
    }
}

struct SharedQueue {
    state: Mutex<QueueState>,
    ready: Notify,
}

#[derive(Default)]
struct QueueState {
    accepting_work: bool,
    drained_vaults: BTreeSet<VaultId>,
    /// One position per Vault per lane, in request order across both lanes,
    /// so a caller taking one turn at a time sees plain FIFO.
    fifo: VecDeque<(VaultId, Lane)>,
    /// Turns running in each lane, indexed by [`Lane`].
    running: [usize; 2],
    vaults: BTreeMap<VaultId, VaultQueueState>,
}

/// One Vault's work in each lane, indexed by [`Lane`].
#[derive(Default)]
struct VaultQueueState {
    lanes: [LaneQueue; 2],
}

/// One Vault's work in one lane: at most one turn active, the rest pending
/// in request order.
#[derive(Default)]
struct LaneQueue {
    active: Option<VaultWorkKind>,
    pending: VecDeque<VaultWorkKind>,
    pending_kinds: BTreeSet<VaultWorkKind>,
    queued: bool,
}

impl LaneQueue {
    fn is_idle(&self) -> bool {
        self.active.is_none() && !self.queued && self.pending.is_empty()
    }

    fn discard_pending(&mut self) {
        self.pending.clear();
        self.pending_kinds.clear();
        self.queued = false;
    }
}

impl VaultQueueState {
    fn lane(&self, lane: Lane) -> &LaneQueue {
        &self.lanes[lane as usize]
    }

    fn lane_mut(&mut self, lane: Lane) -> &mut LaneQueue {
        &mut self.lanes[lane as usize]
    }

    fn is_idle(&self) -> bool {
        self.lanes.iter().all(LaneQueue::is_idle)
    }

    fn has_active(&self) -> bool {
        self.lanes.iter().any(|lane| lane.active.is_some())
    }

    fn discard_pending(&mut self) {
        self.lanes.iter_mut().for_each(LaneQueue::discard_pending);
    }
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
    /// duplicates coalesce into that rerun. Different operation kinds in the
    /// same lane retain their request order inside that Vault's single FIFO
    /// position for the lane.
    pub fn request(&self, vault_id: VaultId, kind: VaultWorkKind) -> ScheduleResult {
        let mut state = self.shared.state.lock().expect("Vault work queue poisoned");
        if !state.accepting_work || state.drained_vaults.contains(&vault_id) {
            return ScheduleResult::Rejected;
        }
        if state
            .vaults
            .get(&vault_id)
            .is_some_and(|vault| vault.lane(Lane::of(kind)).pending_kinds.contains(&kind))
        {
            return ScheduleResult::Coalesced;
        }
        state.enqueue(vault_id, kind);
        drop(state);
        self.shared.ready.notify_waiters();
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
        self.shared.ready.notify_waiters();
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

    /// Stop accepting work for one Vault and discard its queued turns in
    /// both lanes.
    ///
    /// An active turn is never force-cancelled. It keeps its slot until it
    /// returns at its own safe operation boundary.
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

    /// Wait only for the already-active turns, in either lane, of one
    /// drained Vault.
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
    /// Wait for the next turn a lane has room for. Returns `None` once the
    /// coordinator has shut down and nothing queued remains.
    ///
    /// The returned turn holds its slot, so the caller may start it on its
    /// own task and come straight back for the next one: no more than one
    /// Index or Repair turn, and no more than [`GIT_LANE_WIDTH`] Vaults' Git
    /// work, are ever admitted at once, and one Vault never has two turns of
    /// the same lane running.
    pub async fn next_turn(&mut self) -> Option<VaultWorkTurn> {
        loop {
            let notified = self.shared.ready.notified();
            {
                let mut state = self.shared.state.lock().expect("Vault work queue poisoned");
                if let Some(request) = state.take_next() {
                    return Some(VaultWorkTurn {
                        shared: self.shared.clone(),
                        request,
                    });
                }
                if !state.accepting_work {
                    return None;
                }
            }
            notified.await;
        }
    }

    /// Wait for and run one turn to completion, admitting nothing else
    /// meanwhile, for tests that drive turns one at a time. The turn's slot
    /// is released before its outcome is returned.
    #[cfg(test)]
    pub async fn run_next<F, Fut>(&mut self, execute: F) -> Option<VaultWorkOutcome>
    where
        F: FnOnce(VaultWorkRequest) -> Fut,
        Fut: Future<Output = Result<(), VaultWorkError>>,
    {
        let turn = self.next_turn().await?;
        Some(turn.run(execute).await)
    }
}

impl VaultWorkTurn {
    pub fn request(&self) -> VaultWorkRequest {
        self.request
    }

    /// Execute this turn. The slot stays held until the turn is dropped.
    ///
    /// Returned failures are the turn's outcome exactly like successes. So is
    /// a panic: it is caught here and returned as a [`TURN_PANICKED`] failure.
    /// Letting the panic unwind would end the task running the turn without
    /// reporting anything about it (#326).
    pub async fn run<F, Fut>(&self, execute: F) -> VaultWorkOutcome
    where
        F: FnOnce(VaultWorkRequest) -> Fut,
        Fut: Future<Output = Result<(), VaultWorkError>>,
    {
        let request = self.request;
        let result = CatchUnwind(Box::pin(execute(request)))
            .await
            .unwrap_or_else(|panic| {
                Err(VaultWorkError::new(
                    TURN_PANICKED,
                    format!(
                        "The {:?} turn stopped unexpectedly: {}",
                        request.kind(),
                        panic_message(panic.as_ref())
                    ),
                    false,
                ))
            });
        VaultWorkOutcome { request, result }
    }
}

impl Drop for VaultWorkTurn {
    /// Completing on drop is the only way a turn completes, so a turn
    /// abandoned midway, e.g. by a runtime shutting down, cannot keep its
    /// lane slot or its Vault's safe boundary forever.
    fn drop(&mut self) {
        self.shared
            .state
            .lock()
            .expect("Vault work queue poisoned")
            .complete(self.request);
        self.shared.ready.notify_waiters();
    }
}

/// The error code a turn that panicked completes with.
pub const TURN_PANICKED: &str = "vault_work_turn_panicked";

/// Polls a turn's future, turning a panic raised while polling it into an
/// ordinary `Err`. The future is boxed so it is `Unpin`, which keeps this
/// free of `unsafe` pin projection and of a dependency for one combinator.
struct CatchUnwind<Fut>(std::pin::Pin<Box<Fut>>);

impl<Fut: Future> Future for CatchUnwind<Fut> {
    type Output = std::thread::Result<Fut::Output>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let turn = self.0.as_mut();
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| turn.poll(cx))) {
            Ok(poll) => poll.map(Ok),
            Err(panic) => std::task::Poll::Ready(Err(panic)),
        }
    }
}

/// The text a panic was raised with, when it carried one.
pub(crate) fn panic_message(panic: &(dyn std::any::Any + Send)) -> &str {
    panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("no panic message")
}

impl QueueState {
    /// Whether `kind` is currently active or already pending for `vault_id`.
    fn vault_has_work(&self, vault_id: VaultId, kind: VaultWorkKind) -> bool {
        self.vaults.get(&vault_id).is_some_and(|vault| {
            let lane = vault.lane(Lane::of(kind));
            lane.active == Some(kind) || lane.pending_kinds.contains(&kind)
        })
    }

    /// Append `kind` to `vault_id`'s pending work in its lane, taking a FIFO
    /// position for the Vault in that lane if it does not already hold one.
    /// The caller has already decided this turn is required.
    fn enqueue(&mut self, vault_id: VaultId, kind: VaultWorkKind) {
        let lane = Lane::of(kind);
        let queue = self.vaults.entry(vault_id).or_default().lane_mut(lane);
        queue.pending.push_back(kind);
        queue.pending_kinds.insert(kind);
        if !queue.queued {
            queue.queued = true;
            self.fifo.push_back((vault_id, lane));
        }
    }

    /// Admit the earliest position whose lane has a free slot and whose
    /// Vault is not already running a turn in that lane.
    fn take_next(&mut self) -> Option<VaultWorkRequest> {
        let position = self.fifo.iter().position(|(vault_id, lane)| {
            self.running[*lane as usize] < lane.width()
                && self
                    .vaults
                    .get(vault_id)
                    .is_some_and(|vault| vault.lane(*lane).active.is_none())
        })?;
        let (vault_id, lane) = self
            .fifo
            .remove(position)
            .expect("found FIFO position exists");
        let (kind, requeue) = {
            let queue = self
                .vaults
                .get_mut(&vault_id)
                .expect("queued Vault work state missing")
                .lane_mut(lane);
            let kind = queue
                .pending
                .pop_front()
                .expect("queued Vault has no pending work");
            queue.pending_kinds.remove(&kind);
            queue.active = Some(kind);
            queue.queued = !queue.pending.is_empty();
            (kind, queue.queued)
        };
        self.running[lane as usize] += 1;
        if requeue {
            self.fifo.push_back((vault_id, lane));
        }
        Some(VaultWorkRequest::new(vault_id, kind))
    }

    fn complete(&mut self, request: VaultWorkRequest) {
        let lane = Lane::of(request.kind);
        let remove = {
            let vault = self
                .vaults
                .get_mut(&request.vault_id)
                .expect("active Vault work state missing");
            let queue = vault.lane_mut(lane);
            debug_assert_eq!(queue.active, Some(request.kind));
            queue.active = None;
            vault.is_idle()
        };
        self.running[lane as usize] -= 1;
        if remove {
            self.vaults.remove(&request.vault_id);
        }
    }

    fn discard_pending(&mut self, vault_id: VaultId) {
        self.fifo
            .retain(|(queued_vault, _)| *queued_vault != vault_id);
        let remove = match self.vaults.get_mut(&vault_id) {
            Some(vault) => {
                vault.discard_pending();
                !vault.has_active()
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
            vault.discard_pending();
        }
        self.vaults.retain(|_, vault| vault.has_active());
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
        ScheduleResult, VaultWorkCoordinator, VaultWorkError, VaultWorkKind, VaultWorkOutcome,
        VaultWorkRequest, VaultWorkWorker,
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

    /// A panic in a turn's async shell used to unwind straight through the
    /// one shared worker: the dispatch task died, no other Vault's work ever
    /// ran again, and the panicking Vault stayed marked active, so a disable
    /// or edit waiting for its safe boundary hung (#326).
    #[tokio::test]
    async fn a_panicking_turn_completes_and_the_worker_keeps_serving_other_vaults() {
        let panicking = vault_id("00000000-0000-4000-8000-000000000001");
        let healthy = vault_id("00000000-0000-4000-8000-000000000002");
        let (coordinator, mut worker) = VaultWorkCoordinator::new();
        coordinator.request(panicking, VaultWorkKind::Index);
        coordinator.request(healthy, VaultWorkKind::Index);

        let outcome = worker
            .run_next(|request| async move {
                if request.vault_id() == panicking {
                    panic!("injected turn panic");
                }
                Ok::<(), VaultWorkError>(())
            })
            .await
            .expect("the panicking turn still completes");
        assert_eq!(
            outcome.request,
            VaultWorkRequest::new(panicking, VaultWorkKind::Index)
        );
        let error = outcome
            .result
            .expect_err("a panic is reported as a failure");
        assert_eq!(error.code(), super::TURN_PANICKED);
        assert!(!error.retryable());
        assert!(
            error.message().contains("injected turn panic"),
            "the panic's own text reaches the log line: {}",
            error.message()
        );

        coordinator.drain_vault(panicking);
        timeout(
            Duration::from_secs(1),
            coordinator.wait_for_vault_safe_boundary(panicking),
        )
        .await
        .expect("a disable or edit of the panicking Vault does not hang");

        let next = timeout(
            Duration::from_secs(1),
            worker.run_next(|_| async { Ok::<(), VaultWorkError>(()) }),
        )
        .await
        .expect("the worker is still serving")
        .expect("the other Vault's turn");
        assert_eq!(
            next.request,
            VaultWorkRequest::new(healthy, VaultWorkKind::Index)
        );
        next.result.expect("the other Vault's turn runs normally");
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

    /// Start the next admitted turn on its own task, parked until `release`
    /// fires, and return once it is running.
    async fn park_next_turn(
        worker: &mut VaultWorkWorker,
        release: Arc<Notify>,
    ) -> (VaultWorkRequest, tokio::task::JoinHandle<VaultWorkOutcome>) {
        let turn = timeout(Duration::from_secs(1), worker.next_turn())
            .await
            .expect("a turn is admitted")
            .expect("the worker is still serving");
        let request = turn.request();
        let started = Arc::new(Notify::new());
        let handle = tokio::spawn({
            let started = started.clone();
            async move {
                turn.run(move |_| async move {
                    started.notify_one();
                    release.notified().await;
                    Ok::<(), VaultWorkError>(())
                })
                .await
            }
        });
        started.notified().await;
        (request, handle)
    }

    async fn no_turn_admitted(worker: &mut VaultWorkWorker) -> bool {
        timeout(Duration::from_millis(25), worker.next_turn())
            .await
            .is_err()
    }

    /// ADR-31 decision 2: a commit or sync of one Vault never waits out
    /// another Vault's Index turn, which on slow hardware runs for hours.
    #[tokio::test]
    async fn git_work_for_another_vault_runs_while_an_index_turn_is_active() {
        let indexing = vault_id("00000000-0000-4000-8000-000000000001");
        let saving = vault_id("00000000-0000-4000-8000-000000000002");
        let (coordinator, mut worker) = VaultWorkCoordinator::new();
        coordinator.request(indexing, VaultWorkKind::Index);
        coordinator.request(saving, VaultWorkKind::Commit);
        coordinator.request(saving, VaultWorkKind::Git);

        let release_index = Arc::new(Notify::new());
        let (request, index_turn) = park_next_turn(&mut worker, release_index.clone()).await;
        assert_eq!(
            request,
            VaultWorkRequest::new(indexing, VaultWorkKind::Index)
        );

        for expected in [VaultWorkKind::Commit, VaultWorkKind::Git] {
            let outcome = timeout(
                Duration::from_secs(1),
                worker.run_next(|_| async { Ok::<(), VaultWorkError>(()) }),
            )
            .await
            .expect("Git work does not wait for the Index turn")
            .expect("queued turn");
            assert_eq!(outcome.request, VaultWorkRequest::new(saving, expected));
        }
        assert!(!index_turn.is_finished(), "the Index turn is still parked");

        release_index.notify_one();
        index_turn
            .await
            .expect("Index turn task")
            .result
            .expect("Index turn succeeds");
    }

    /// The coordinator admits a Vault's own Git work beside its own Index
    /// turn too. The mutation lock, not the queue, is what keeps a sync out
    /// of an Index turn's read phase (ADR-31 decision 4).
    #[tokio::test]
    async fn a_vaults_own_git_work_is_admitted_beside_its_index_turn() {
        let vault = vault_id("00000000-0000-4000-8000-000000000001");
        let (coordinator, mut worker) = VaultWorkCoordinator::new();
        coordinator.request(vault, VaultWorkKind::Index);
        coordinator.request(vault, VaultWorkKind::Commit);

        let release_index = Arc::new(Notify::new());
        let (_, index_turn) = park_next_turn(&mut worker, release_index.clone()).await;
        let commit = timeout(
            Duration::from_secs(1),
            worker.run_next(|_| async { Ok::<(), VaultWorkError>(()) }),
        )
        .await
        .expect("the commit does not wait for its own Vault's Index turn")
        .expect("queued turn");
        assert_eq!(
            commit.request,
            VaultWorkRequest::new(vault, VaultWorkKind::Commit)
        );

        release_index.notify_one();
        index_turn.await.expect("Index turn task");
    }

    /// ADR-31 decision 3: at most four Vaults run Git work at once, and a
    /// fifth waits for a slot rather than for anything else.
    #[tokio::test]
    async fn at_most_four_vaults_run_git_work_at_once() {
        let vaults = (1..=5)
            .map(|n| vault_id(&format!("00000000-0000-4000-8000-00000000000{n}")))
            .collect::<Vec<_>>();
        let (coordinator, mut worker) = VaultWorkCoordinator::new();
        for vault in &vaults {
            coordinator.request(*vault, VaultWorkKind::Git);
        }

        let mut running = Vec::new();
        for vault in &vaults[..4] {
            let release = Arc::new(Notify::new());
            let (request, handle) = park_next_turn(&mut worker, release.clone()).await;
            assert_eq!(request.vault_id(), *vault, "Git slots fill in FIFO order");
            running.push((release, handle));
        }
        assert!(
            no_turn_admitted(&mut worker).await,
            "a fifth Vault's Git work waits while four are running"
        );

        // An Index turn is not held up by the full Git slots.
        coordinator.request(vaults[0], VaultWorkKind::Index);
        let index = timeout(
            Duration::from_secs(1),
            worker.run_next(|_| async { Ok::<(), VaultWorkError>(()) }),
        )
        .await
        .expect("indexing has its own lane")
        .expect("queued turn");
        assert_eq!(
            index.request,
            VaultWorkRequest::new(vaults[0], VaultWorkKind::Index)
        );

        let (release, handle) = running.remove(1);
        release.notify_one();
        handle.await.expect("Git turn task");
        let fifth = timeout(
            Duration::from_secs(1),
            worker.run_next(|_| async { Ok::<(), VaultWorkError>(()) }),
        )
        .await
        .expect("the fifth Vault starts once a slot frees")
        .expect("queued turn");
        assert_eq!(
            fifth.request,
            VaultWorkRequest::new(vaults[4], VaultWorkKind::Git)
        );

        for (release, handle) in running {
            release.notify_one();
            handle.await.expect("Git turn task");
        }
    }

    /// One Vault's Git work still runs one turn at a time, in FIFO order,
    /// with free Git slots to spare, and keeps today's coalescing.
    #[tokio::test]
    async fn one_vaults_git_work_never_overlaps_and_still_coalesces() {
        let vault = vault_id("00000000-0000-4000-8000-000000000001");
        let (coordinator, mut worker) = VaultWorkCoordinator::new();
        assert_eq!(
            coordinator.request(vault, VaultWorkKind::Git),
            ScheduleResult::Queued
        );
        assert_eq!(
            coordinator.request(vault, VaultWorkKind::Commit),
            ScheduleResult::Queued,
            "a pending sync does not swallow a commit"
        );
        assert_eq!(
            coordinator.request(vault, VaultWorkKind::Git),
            ScheduleResult::Coalesced,
            "a pending duplicate collapses"
        );

        let release = Arc::new(Notify::new());
        let (request, sync) = park_next_turn(&mut worker, release.clone()).await;
        assert_eq!(request, VaultWorkRequest::new(vault, VaultWorkKind::Git));
        assert_eq!(
            coordinator.request(vault, VaultWorkKind::Git),
            ScheduleResult::Queued,
            "an active sync keeps one rerun"
        );
        assert_eq!(
            coordinator.request(vault, VaultWorkKind::Git),
            ScheduleResult::Coalesced
        );
        assert_eq!(
            coordinator.request_if_idle(vault, VaultWorkKind::Commit),
            ScheduleResult::Coalesced,
            "request_if_idle adds nothing to work already pending"
        );
        assert!(
            no_turn_admitted(&mut worker).await,
            "the Vault's commit waits for its own active sync, not for a free slot"
        );

        release.notify_one();
        sync.await.expect("sync task");
        let mut observed = Vec::new();
        for _ in 0..2 {
            observed.push(
                worker
                    .run_next(|_| async { Ok::<(), VaultWorkError>(()) })
                    .await
                    .expect("queued turn")
                    .request
                    .kind(),
            );
        }
        assert_eq!(observed, vec![VaultWorkKind::Commit, VaultWorkKind::Git]);
        assert!(no_turn_admitted(&mut worker).await, "the burst converged");
    }

    /// ADR-31 decision 1: indexing keeps one instance-wide lane.
    #[tokio::test]
    async fn index_turns_for_different_vaults_never_overlap_and_stay_fifo() {
        let first = vault_id("00000000-0000-4000-8000-000000000001");
        let second = vault_id("00000000-0000-4000-8000-000000000002");
        let third = vault_id("00000000-0000-4000-8000-000000000003");
        let (coordinator, mut worker) = VaultWorkCoordinator::new();
        coordinator.request(first, VaultWorkKind::Index);
        coordinator.request(second, VaultWorkKind::Repair);
        coordinator.request(third, VaultWorkKind::Index);

        let release = Arc::new(Notify::new());
        let (request, active) = park_next_turn(&mut worker, release.clone()).await;
        assert_eq!(request, VaultWorkRequest::new(first, VaultWorkKind::Index));
        assert!(
            no_turn_admitted(&mut worker).await,
            "no second Index or Repair turn starts beside an active one"
        );
        release.notify_one();
        active.await.expect("Index turn task");

        let mut observed = Vec::new();
        for _ in 0..2 {
            observed.push(
                worker
                    .run_next(|_| async { Ok::<(), VaultWorkError>(()) })
                    .await
                    .expect("queued turn")
                    .request,
            );
        }
        assert_eq!(
            observed,
            vec![
                VaultWorkRequest::new(second, VaultWorkKind::Repair),
                VaultWorkRequest::new(third, VaultWorkKind::Index),
            ]
        );
    }

    /// ADR-31 decision 5: a drained Vault's safe boundary covers whatever it
    /// has running in either lane, and nothing it had queued runs later.
    #[tokio::test]
    async fn draining_a_vault_waits_for_its_index_and_git_turns_and_discards_its_queue() {
        let vault = vault_id("00000000-0000-4000-8000-000000000001");
        let other = vault_id("00000000-0000-4000-8000-000000000002");
        let (coordinator, mut worker) = VaultWorkCoordinator::new();
        coordinator.request(vault, VaultWorkKind::Index);
        coordinator.request(vault, VaultWorkKind::Git);

        let release_index = Arc::new(Notify::new());
        let release_git = Arc::new(Notify::new());
        let (_, index_turn) = park_next_turn(&mut worker, release_index.clone()).await;
        let (_, git_turn) = park_next_turn(&mut worker, release_git.clone()).await;
        coordinator.request(vault, VaultWorkKind::Index);
        coordinator.request(vault, VaultWorkKind::Commit);
        coordinator.request(other, VaultWorkKind::Git);

        coordinator.drain_vault(vault);
        let boundary = tokio::spawn({
            let coordinator = coordinator.clone();
            async move { coordinator.wait_for_vault_safe_boundary(vault).await }
        });
        release_index.notify_one();
        index_turn.await.expect("Index turn task");
        tokio::task::yield_now().await;
        assert!(
            !boundary.is_finished(),
            "the boundary still waits for the Vault's running Git turn"
        );
        release_git.notify_one();
        git_turn.await.expect("Git turn task");
        timeout(Duration::from_secs(1), boundary)
            .await
            .expect("the boundary is reached once both turns finish")
            .expect("boundary task");

        let next = worker
            .run_next(|_| async { Ok::<(), VaultWorkError>(()) })
            .await
            .expect("the other Vault's turn");
        assert_eq!(
            next.request,
            VaultWorkRequest::new(other, VaultWorkKind::Git)
        );
        assert!(
            no_turn_admitted(&mut worker).await,
            "nothing the drained Vault had queued runs afterwards"
        );
    }

    #[tokio::test]
    async fn shutdown_waits_for_running_turns_in_both_lanes() {
        let indexing = vault_id("00000000-0000-4000-8000-000000000001");
        let syncing = vault_id("00000000-0000-4000-8000-000000000002");
        let (coordinator, mut worker) = VaultWorkCoordinator::new();
        coordinator.request(indexing, VaultWorkKind::Index);
        coordinator.request(syncing, VaultWorkKind::Git);

        let release_index = Arc::new(Notify::new());
        let release_git = Arc::new(Notify::new());
        let (_, index_turn) = park_next_turn(&mut worker, release_index.clone()).await;
        let (_, git_turn) = park_next_turn(&mut worker, release_git.clone()).await;
        coordinator.request(indexing, VaultWorkKind::Commit);

        coordinator.shutdown();
        assert!(
            worker.next_turn().await.is_none(),
            "a stopped worker admits nothing more"
        );
        let boundary = tokio::spawn({
            let coordinator = coordinator.clone();
            async move { coordinator.wait_for_shutdown_boundary().await }
        });
        release_git.notify_one();
        git_turn.await.expect("Git turn task");
        tokio::task::yield_now().await;
        assert!(!boundary.is_finished(), "the Index turn is still running");
        release_index.notify_one();
        index_turn.await.expect("Index turn task");
        timeout(Duration::from_secs(1), boundary)
            .await
            .expect("shutdown is quiescent once both lanes finish")
            .expect("boundary task");
    }

    /// #326 in both lanes: a panic completes its turn, and the lane it ran
    /// in keeps serving.
    #[tokio::test]
    async fn a_panic_in_either_lane_completes_the_turn_and_the_lane_keeps_serving() {
        let vault = vault_id("00000000-0000-4000-8000-000000000001");
        let (coordinator, mut worker) = VaultWorkCoordinator::new();
        for kind in [VaultWorkKind::Git, VaultWorkKind::Index] {
            coordinator.request(vault, kind);
            let outcome = worker
                .run_next(|request| async move {
                    if request.kind() == kind {
                        panic!("injected {kind:?} panic");
                    }
                    Ok::<(), VaultWorkError>(())
                })
                .await
                .expect("the panicking turn completes");
            assert_eq!(outcome.request, VaultWorkRequest::new(vault, kind));
            assert_eq!(
                outcome.result.expect_err("reported as a failure").code(),
                super::TURN_PANICKED
            );
        }
        for kind in [VaultWorkKind::Git, VaultWorkKind::Index] {
            coordinator.request(vault, kind);
            worker
                .run_next(|_| async { Ok::<(), VaultWorkError>(()) })
                .await
                .expect("a later turn of the same kind still runs")
                .result
                .expect("and succeeds");
        }
    }

    /// The dispatch loop publishes a turn's outcome before it drops the
    /// turn, so the next turn in that lane cannot start, and set its Vault's
    /// status, before the previous outcome is published.
    #[tokio::test]
    async fn a_lane_slot_stays_held_until_the_turn_is_dropped() {
        let first = vault_id("00000000-0000-4000-8000-000000000001");
        let second = vault_id("00000000-0000-4000-8000-000000000002");
        let (coordinator, mut worker) = VaultWorkCoordinator::new();
        coordinator.request(first, VaultWorkKind::Index);
        coordinator.request(second, VaultWorkKind::Index);

        let turn = worker.next_turn().await.expect("admitted turn");
        turn.run(|_| async { Ok::<(), VaultWorkError>(()) })
            .await
            .result
            .expect("turn succeeds");
        assert!(
            no_turn_admitted(&mut worker).await,
            "a finished but unpublished turn still holds the Index lane"
        );
        drop(turn);
        let next = timeout(Duration::from_secs(1), worker.next_turn())
            .await
            .expect("the lane frees once the turn is dropped")
            .expect("queued turn");
        assert_eq!(
            next.request(),
            VaultWorkRequest::new(second, VaultWorkKind::Index)
        );
    }

    /// A turn dropped before it finishes, e.g. with the runtime shutting
    /// down, still hands back its slot and its Vault's safe boundary.
    #[tokio::test]
    async fn a_dropped_turn_releases_its_slot() {
        let vault = vault_id("00000000-0000-4000-8000-000000000001");
        let (coordinator, mut worker) = VaultWorkCoordinator::new();
        coordinator.request(vault, VaultWorkKind::Index);
        let turn = worker.next_turn().await.expect("admitted turn");
        drop(turn);
        timeout(
            Duration::from_secs(1),
            coordinator.wait_for_vault_safe_boundary(vault),
        )
        .await
        .expect("the dropped turn's Vault is idle again");
        coordinator.request(vault, VaultWorkKind::Index);
        timeout(
            Duration::from_secs(1),
            worker.run_next(|_| async { Ok::<(), VaultWorkError>(()) }),
        )
        .await
        .expect("the Index lane is free again")
        .expect("queued turn");
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
