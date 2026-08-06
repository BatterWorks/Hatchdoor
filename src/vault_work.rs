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
}

impl VaultWorkError {
    pub fn new(code: impl Into<String>, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable,
        }
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
}

/// Whether a request added a required turn or joined existing pending work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScheduleResult {
    Queued,
    Coalesced,
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
            state: Mutex::new(QueueState::default()),
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
        let vault = state.vaults.entry(vault_id).or_default();
        if vault.pending_kinds.contains(&kind) {
            return ScheduleResult::Coalesced;
        }

        vault.pending.push_back(kind);
        vault.pending_kinds.insert(kind);
        let enqueue_vault = !vault.queued;
        if enqueue_vault {
            vault.queued = true;
            state.fifo.push_back(vault_id);
        }
        drop(state);
        self.shared.ready.notify_one();
        ScheduleResult::Queued
    }
}

impl VaultWorkWorker {
    /// Wait for and execute one globally serialized operation turn.
    ///
    /// Returned failures complete the turn exactly like successes, allowing
    /// the next Vault to proceed. Panics are intentionally outside this
    /// returned-failure contract and remain task failures for the lifecycle
    /// owner to handle.
    pub async fn run_next<F, Fut>(&mut self, execute: F) -> VaultWorkOutcome
    where
        F: FnOnce(VaultWorkRequest) -> Fut,
        Fut: Future<Output = Result<(), VaultWorkError>>,
    {
        let request = self.next_request().await;
        let result = execute(request).await;
        self.shared
            .state
            .lock()
            .expect("Vault work queue poisoned")
            .complete(request);
        VaultWorkOutcome { request, result }
    }

    async fn next_request(&self) -> VaultWorkRequest {
        loop {
            let notified = self.shared.ready.notified();
            if let Some(request) = self
                .shared
                .state
                .lock()
                .expect("Vault work queue poisoned")
                .take_next()
            {
                return request;
            }
            notified.await;
        }
    }
}

impl QueueState {
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
                .await;
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
                    .await;
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
            .await;
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
            .await;
        assert_eq!(failed.request.vault_id(), failing);
        assert_eq!(failed.request.kind(), VaultWorkKind::Repair);
        assert_eq!(
            failed.result.expect_err("repair fails").code(),
            "repair_failed"
        );

        let succeeded = worker
            .run_next(|_| async { Ok::<(), VaultWorkError>(()) })
            .await;
        assert_eq!(succeeded.request.vault_id(), healthy);
        assert_eq!(succeeded.request.kind(), VaultWorkKind::Index);
        succeeded.result.expect("healthy Vault proceeds");
    }
}
