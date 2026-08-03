use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use tokio::sync::{Mutex, Notify, RwLock, mpsc};
use tracing::{error, info, warn};

use std::path::PathBuf;

use super::config::{GitConfig, GitMode};
use super::message::{WriteRecord, build_commit_message};
use super::status::GitSyncStatus;
use super::sync::{
    CommitOutcome, GitError, SyncOutcome, has_uncommitted_changes, has_unpushed, unpushed_count,
};

/// The four phases of a sync, split so the background task can hold the vault
/// lock only across the local/working-tree phases (`commit`, `integrate`) and
/// release it across the network phases (`fetch`, `push`) — a slow or hanging
/// remote must never block concurrent HTTP/MCP vault writes. In production these
/// are wired to `super::sync::{commit_local, fetch_remote, integrate_fetched,
/// push_branch}`; tests inject fakes.
pub struct SyncOps {
    /// Stage + commit the batch (working tree + index only). Reports whether the
    /// remote phases are needed. Runs UNDER the vault lock.
    pub commit: CommitOp,
    /// Fetch the remote branch (network read, no working-tree change). Runs
    /// WITHOUT the vault lock.
    pub fetch: GitPhaseOp,
    /// Merge the fetched remote into the local branch if it moved ahead (may
    /// checkout the working tree). Runs UNDER the vault lock.
    pub integrate: GitPhaseOp,
    /// Push the local branch (network write). Runs WITHOUT the vault lock.
    pub push: GitPhaseOp,
}

pub type CommitOp =
    Box<dyn Fn(&GitConfig, &[PathBuf], &str) -> Result<CommitOutcome, GitError> + Send + Sync>;
pub type GitPhaseOp = Box<dyn Fn(&GitConfig) -> Result<(), GitError> + Send + Sync>;

/// Backoff bounds for re-attempting a sync that failed for a transient reason
/// (remote/network/auth). Without this, a brief outage strands committed vault
/// edits unpushed until the next write or a full process restart.
const RETRY_BASE: Duration = Duration::from_secs(5);
const RETRY_MAX: Duration = Duration::from_secs(300);

/// Handle stored in AppState so write tools can enqueue records and readers can
/// observe status. `None` everywhere when git sync is disabled.
#[derive(Clone)]
pub struct GitSyncHandle {
    sender: Arc<StdMutex<Option<mpsc::UnboundedSender<WriteRecord>>>>,
    status: Arc<RwLock<GitSyncStatus>>,
    task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    /// Requests a graceful stop. The record channel is deliberately never
    /// closed to signal shutdown (that would be irreversible the moment a
    /// timed-out `stop` gives up): the task instead polls this flag, and a
    /// timed-out `stop` clears it again so the still-running task keeps
    /// accepting and committing records exactly as if it had never been asked
    /// to stop (S3: a refused stop must not leave a handle that silently
    /// drops every future write).
    stop_requested: Arc<AtomicBool>,
    /// Wakes the task promptly when `stop_requested` changes, instead of
    /// waiting for the next record or retry timer.
    shutdown: Arc<Notify>,
}

impl GitSyncHandle {
    /// Enqueue a write for the next debounced sync. Never blocks; drops silently
    /// if the background task has stopped (it will be retried by later writes).
    pub fn record(&self, record: WriteRecord) {
        if let Some(sender) = self
            .sender
            .lock()
            .expect("git task sender poisoned")
            .as_ref()
        {
            let _ = sender.send(record);
        }
    }

    pub fn status(&self) -> Arc<RwLock<GitSyncStatus>> {
        self.status.clone()
    }

    /// Ask the worker to finish its current/queued batch and exit, then wait
    /// up to `timeout` for it to do so.
    ///
    /// On success, the record channel is closed and this handle is spent: a
    /// caller replaces it (or drops it) rather than reusing it.
    ///
    /// On timeout, the stop request is *withdrawn* rather than left pending:
    /// the still-running task keeps servicing `record()` calls exactly as
    /// before, so a caller that gives up on this attempt (e.g. to answer an
    /// HTTP request with `409`) does not strand future vault writes on a
    /// handle nobody is draining. A later call to `stop` tries again. This
    /// preserves the #56 invariant (two sync tasks never touch the repository
    /// together): the caller only installs a replacement task once `stop`
    /// truly returns `Ok`.
    pub async fn stop(&self, timeout: Duration) -> Result<(), String> {
        self.status.write().await.state = "stopping".to_string();
        self.stop_requested.store(true, Ordering::SeqCst);
        self.shutdown.notify_one();
        let mut task = self.task.lock().await;
        let Some(join) = task.as_mut() else {
            return Ok(());
        };
        match tokio::time::timeout(timeout, join).await {
            Ok(Ok(())) => {
                task.take();
                self.sender.lock().expect("git task sender poisoned").take();
                Ok(())
            }
            Ok(Err(error)) => Err(format!("Versioning task failed while stopping: {error}")),
            Err(_) => {
                self.stop_requested.store(false, Ordering::SeqCst);
                Err("Versioning is still draining its current sync. Retry shortly.".to_string())
            }
        }
    }
}

/// Spawn the background sync task. `vault_lock` is the shared vault-mutation lock,
/// also acquired by MCP write tools. `ops` performs the actual git work in four
/// phases; in production these call `super::sync::*`, and tests inject fakes.
pub fn spawn_sync_task(
    config: GitConfig,
    vault_lock: Arc<Mutex<()>>,
    ops: SyncOps,
) -> GitSyncHandle {
    let (sender, receiver) = mpsc::unbounded_channel();
    let sender = Arc::new(StdMutex::new(Some(sender)));
    let status = Arc::new(RwLock::new(GitSyncStatus::starting(config.mode.as_str())));
    let task_status = status.clone();
    let debounce = Duration::from_secs(config.debounce_seconds.max(1));
    let ops = Arc::new(ops);
    let stop_requested = Arc::new(AtomicBool::new(false));
    let shutdown = Arc::new(Notify::new());
    let task_stop_requested = stop_requested.clone();
    let task_shutdown = shutdown.clone();

    // Decide up front whether to flush drift accumulated while versioning was
    // off: uncommitted working-tree edits in either mode, or commits stranded
    // by an earlier outage in remote mode. This is a cheap local check; if it
    // can't read git state (e.g. no repo yet, as in unit tests) we simply
    // don't flush — the first real write covers it instead.
    let startup_flush = matches!(has_uncommitted_changes(&config), Ok(true))
        || (config.mode == GitMode::Remote && matches!(has_unpushed(&config), Ok(true)));

    let task = tokio::spawn(async move {
        run_loop(
            config,
            debounce,
            vault_lock,
            receiver,
            task_status,
            ops,
            startup_flush,
            task_stop_requested,
            task_shutdown,
        )
        .await;
    });

    GitSyncHandle {
        sender,
        status,
        task: Arc::new(Mutex::new(Some(task))),
        stop_requested,
        shutdown,
    }
}

/// What made `next_event` return.
enum NextEvent {
    Record(WriteRecord),
    /// The channel closed (handle dropped) or a stop was requested and is
    /// still in effect.
    Shutdown,
}

#[allow(clippy::too_many_arguments)]
async fn run_loop(
    config: GitConfig,
    debounce: Duration,
    vault_lock: Arc<Mutex<()>>,
    mut receiver: mpsc::UnboundedReceiver<WriteRecord>,
    status: Arc<RwLock<GitSyncStatus>>,
    ops: Arc<SyncOps>,
    startup_flush: bool,
    stop_requested: Arc<AtomicBool>,
    shutdown: Arc<Notify>,
) {
    status.write().await.state = "running".to_string();
    let mut batch: Vec<WriteRecord> = Vec::new();
    // When a sync fails transiently, `retry_after` holds the backoff before the
    // next unprompted re-attempt (empty batch). `None` means nothing to retry.
    let mut retry_after: Option<Duration> = None;

    // Immediately flush any drift accumulated while versioning was off, rather
    // than waiting for the first write to trigger a debounced sync.
    if startup_flush
        && run_one_sync_with_message(
            &config,
            &vault_lock,
            &status,
            &ops,
            Vec::new(),
            STARTUP_DRIFT_MESSAGE.to_string(),
        )
        .await
    {
        retry_after = Some(RETRY_BASE);
    }

    loop {
        // Wait for the first record, a stop request, or channel close. If a
        // previous sync failed transiently, also race a backoff timer that
        // re-attempts the push with no new write, so a brief remote outage
        // self-heals.
        let first = match next_event(
            &mut receiver,
            &config,
            &vault_lock,
            &status,
            &ops,
            &mut retry_after,
            &stop_requested,
            &shutdown,
        )
        .await
        {
            NextEvent::Record(record) => record,
            NextEvent::Shutdown => break,
        };
        batch.push(first);
        update_pending(&status, batch.len()).await;

        // Debounce: keep extending the quiet window while records keep arriving.
        loop {
            let timer = tokio::time::sleep(debounce);
            tokio::pin!(timer);
            tokio::select! {
                _ = &mut timer => break,
                maybe = receiver.recv() => match maybe {
                    Some(record) => {
                        batch.push(record);
                        update_pending(&status, batch.len()).await;
                    }
                    None => break,
                }
            }
        }

        let failed = run_one_sync(
            &config,
            &vault_lock,
            &status,
            &ops,
            std::mem::take(&mut batch),
        )
        .await;
        // A fresh write already reset the debounce; start any new backoff from
        // the base rather than compounding a prior one.
        retry_after = failed.then_some(RETRY_BASE);
        update_pending(&status, 0).await;
    }

    // A stop was requested while we were mid-batch (or right as we returned to
    // the top of the loop) and any records that arrived in that window are
    // still sitting in the channel: drain them synchronously and commit once
    // more before the task truly exits, rather than silently dropping them.
    let mut trailing = Vec::new();
    while let Ok(record) = receiver.try_recv() {
        trailing.push(record);
    }
    if !trailing.is_empty() {
        run_one_sync(&config, &vault_lock, &status, &ops, trailing).await;
    }
    // Matches the frontend's `GitStatus.state` union (never surfaced beyond
    // this point in practice: the caller removes this handle from `AppState`
    // as soon as `stop` returns `Ok`, replacing or clearing it).
    status.write().await.state = "disabled".to_string();
}

/// Await the next queued write, a withdrawable stop request, or (when a
/// previous sync failed transiently) a retry backoff timer. Returns
/// `NextEvent::Shutdown` when the channel closes (the handle was dropped) or
/// when a stop is currently requested; a stop that is later withdrawn (a
/// timed-out `GitSyncHandle::stop`) simply lets this keep waiting normally.
#[allow(clippy::too_many_arguments)]
async fn next_event(
    receiver: &mut mpsc::UnboundedReceiver<WriteRecord>,
    config: &GitConfig,
    vault_lock: &Arc<Mutex<()>>,
    status: &Arc<RwLock<GitSyncStatus>>,
    ops: &Arc<SyncOps>,
    retry_after: &mut Option<Duration>,
    stop_requested: &Arc<AtomicBool>,
    shutdown: &Arc<Notify>,
) -> NextEvent {
    loop {
        if stop_requested.load(Ordering::SeqCst) {
            return NextEvent::Shutdown;
        }

        let notified = shutdown.notified();
        tokio::pin!(notified);

        match *retry_after {
            None => {
                tokio::select! {
                    maybe = receiver.recv() => {
                        return match maybe {
                            Some(record) => NextEvent::Record(record),
                            None => NextEvent::Shutdown,
                        };
                    }
                    _ = &mut notified => continue,
                }
            }
            Some(delay) => {
                let timer = tokio::time::sleep(delay);
                tokio::pin!(timer);
                tokio::select! {
                    maybe = receiver.recv() => {
                        return match maybe {
                            Some(record) => NextEvent::Record(record),
                            None => NextEvent::Shutdown,
                        };
                    }
                    _ = &mut notified => continue,
                    _ = &mut timer => {
                        let failed = run_one_sync(config, vault_lock, status, ops, Vec::new()).await;
                        *retry_after = failed.then(|| (delay * 2).min(RETRY_MAX));
                    }
                }
            }
        }
    }
}

/// Run one sync as four phases, holding `vault_lock` ONLY across the local
/// commit and the working-tree-mutating integrate, and releasing it across the
/// network fetch/push. A single blocking phase is one `spawn_blocking` hop.
async fn run_sync_phases(
    config: &GitConfig,
    vault_lock: &Arc<Mutex<()>>,
    ops: &Arc<SyncOps>,
    paths: Vec<PathBuf>,
    message: String,
) -> Result<SyncOutcome, GitError> {
    // Phase 1 — local commit, UNDER the vault lock (touches working tree + index).
    let commit = {
        let _guard = vault_lock.lock().await;
        let ops = ops.clone();
        let cfg = config.clone();
        run_blocking(move || (ops.commit)(&cfg, &paths, &message)).await?
    };
    if config.mode == GitMode::Local {
        return Ok(SyncOutcome::Committed {
            committed: commit.committed,
        });
    }
    if !commit.needs_remote {
        return Ok(SyncOutcome::NoChanges);
    }

    // Phase 2 — fetch, WITHOUT the lock. A hanging remote here must not block
    // vault writers, so the lock is released across this network read.
    {
        let ops = ops.clone();
        let cfg = config.clone();
        run_blocking(move || (ops.fetch)(&cfg)).await?;
    }

    // Phase 3 — integrate the fetched remote, UNDER the lock (may checkout).
    {
        let _guard = vault_lock.lock().await;
        let ops = ops.clone();
        let cfg = config.clone();
        run_blocking(move || (ops.integrate)(&cfg)).await?;
    }

    // Phase 4 — push, WITHOUT the lock (network write).
    {
        let ops = ops.clone();
        let cfg = config.clone();
        run_blocking(move || (ops.push)(&cfg)).await?;
    }

    Ok(SyncOutcome::Pushed {
        committed: commit.committed,
    })
}

/// Run a blocking git closure on the blocking pool, mapping a join error (panic)
/// to a `GitError` rather than unwinding the task.
async fn run_blocking<T, F>(f: F) -> Result<T, GitError>
where
    F: FnOnce() -> Result<T, GitError> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .unwrap_or_else(|join_err| Err(GitError::Other(format!("sync task panicked: {join_err}"))))
}

/// Runs one sync and records status. Returns `true` when the attempt failed for
/// a transient reason (remote/network/other) and should be re-attempted on a
/// backoff; `false` on success or on a non-transient failure (conflict / dirty
/// tree / validation) that a bare re-attempt cannot fix.
async fn run_one_sync(
    config: &GitConfig,
    vault_lock: &Arc<Mutex<()>>,
    status: &Arc<RwLock<GitSyncStatus>>,
    ops: &Arc<SyncOps>,
    batch: Vec<WriteRecord>,
) -> bool {
    let message = build_commit_message(&batch);
    run_one_sync_with_message(config, vault_lock, status, ops, batch, message).await
}

/// Startup-drift commit message: distinguishable in history from an ordinary
/// batch (issue #56), since it was never actually a debounced batch of
/// specific writes — it is whatever the working tree already held when
/// versioning was turned on.
const STARTUP_DRIFT_MESSAGE: &str =
    "hatchdoor: recorded existing changes from before versioning was turned on";

async fn run_one_sync_with_message(
    config: &GitConfig,
    vault_lock: &Arc<Mutex<()>>,
    status: &Arc<RwLock<GitSyncStatus>>,
    ops: &Arc<SyncOps>,
    batch: Vec<WriteRecord>,
    message: String,
) -> bool {
    let mut paths: Vec<PathBuf> = batch
        .iter()
        .flat_map(|r| r.affected_paths.clone())
        .collect();
    paths.sort();
    paths.dedup();

    let result = run_sync_phases(config, vault_lock, ops, paths, message).await;

    // Best-effort: read how many local commits remain unpushed afterward. This
    // is a local read (no working-tree change), so it needs no lock.
    let unpushed = if config.mode == GitMode::Remote {
        let config_clone = config.clone();
        tokio::task::spawn_blocking(move || unpushed_count(&config_clone).ok())
            .await
            .ok()
            .flatten()
    } else {
        None
    };

    let mut guard = status.write().await;
    guard.last_sync_at = Some(now_rfc3339());
    guard.unpushed = unpushed;
    match result {
        Ok(outcome) => {
            guard.last_ok = true;
            guard.last_error = None;
            guard.last_error_kind = None;
            match outcome {
                SyncOutcome::NoChanges => info!("git sync: no changes"),
                SyncOutcome::Pushed { committed } => {
                    info!(committed, "git sync: pushed")
                }
                SyncOutcome::Committed { committed } => {
                    info!(committed, "git versioning: committed locally")
                }
            }
            false
        }
        Err(err) => {
            guard.last_ok = false;
            let message = err.to_string();
            // Remote/other failures are typically transient (network, auth blip,
            // non-fast-forward) and worth an automatic retry. A conflict, dirty
            // tree, or validation error needs the remote or a human to change
            // first, so hammering it would just spin.
            let transient = matches!(err, GitError::Remote(_) | GitError::Other(_));
            match &err {
                GitError::Conflict { .. } => warn!("git sync conflict: {message}"),
                GitError::DirtyWorkingTree { .. } => warn!("git sync skipped: {message}"),
                _ => error!("git sync failed: {message}"),
            }
            guard.last_error = Some(message);
            guard.last_error_kind = Some(err.kind().to_string());
            transient
        }
    }
}

async fn update_pending(status: &Arc<RwLock<GitSyncStatus>>, pending: usize) {
    status.write().await.pending = pending;
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    fn unused_config(debounce_seconds: u64) -> GitConfig {
        GitConfig {
            vault_path: std::path::PathBuf::from("/unused"),
            mode: crate::git::config::GitMode::Remote,
            remote: "origin".into(),
            branch: "main".into(),
            username: "u".into(),
            token: "t".into(),
            debounce_seconds,
            author_name: "n".into(),
            author_email: "e".into(),
        }
    }

    /// A real local git repo with one committed note and one uncommitted
    /// working-tree edit — the "drift accumulated while versioning was off"
    /// scenario from issue #56.
    fn repo_with_drift(mode: GitMode) -> (tempfile::TempDir, GitConfig) {
        let temp = tempfile::tempdir().expect("tempdir");
        let vault = temp.path().join("vault");
        std::fs::create_dir_all(&vault).expect("create vault");
        let repo = git2::Repository::init(&vault).expect("init repo");
        std::fs::write(vault.join("Home.md"), "# Home\n").expect("write note");
        {
            let mut index = repo.index().expect("index");
            index
                .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
                .expect("stage");
            index.write().expect("write index");
            let tree_oid = index.write_tree().expect("write tree");
            let tree = repo.find_tree(tree_oid).expect("tree");
            let sig = git2::Signature::now("n", "e").expect("sig");
            repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
                .expect("initial commit");
        }
        // Uncommitted drift: an edit after the last commit, with no batch
        // ever recorded for it.
        std::fs::write(vault.join("Home.md"), "# Home\n\ndrift\n").expect("write drift");

        let config = GitConfig {
            vault_path: vault,
            mode,
            remote: "origin".into(),
            branch: "main".into(),
            username: "u".into(),
            token: "t".into(),
            debounce_seconds: 1,
            author_name: "n".into(),
            author_email: "e".into(),
        };
        (temp, config)
    }

    /// S7 regression: turning versioning on for a vault that already has
    /// uncommitted edits must commit that drift immediately, under its own
    /// commit message — in both local and remote mode. Before the fix,
    /// `startup_flush` only ever triggered in remote mode with unpushed
    /// *commits*, never for uncommitted working-tree drift, and never in
    /// local mode at all.
    async fn wait_for_head_message(vault_path: &std::path::Path, expected: &str) -> bool {
        for _ in 0..400 {
            let matched = (|| -> Option<bool> {
                let repo = git2::Repository::open(vault_path).ok()?;
                let head = repo.head().ok()?;
                let commit = head.peel_to_commit().ok()?;
                Some(commit.message().ok() == Some(expected))
            })()
            .unwrap_or(false);
            if matched {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        false
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn spawning_flushes_accumulated_drift_in_local_mode() {
        let (_temp, config) = repo_with_drift(GitMode::Local);
        let vault_path = config.vault_path.clone();
        let handle = spawn_sync_task(
            config,
            Arc::new(Mutex::new(())),
            SyncOps {
                commit: Box::new(crate::git::commit_local),
                fetch: Box::new(|_| Ok(())),
                integrate: Box::new(|_| Ok(())),
                push: Box::new(|_| Ok(())),
            },
        );
        let status = handle.status();

        assert!(
            wait_for_head_message(&vault_path, STARTUP_DRIFT_MESSAGE).await,
            "startup flush never committed the pre-existing drift"
        );
        assert!(status.read().await.last_ok);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn spawning_flushes_accumulated_drift_in_remote_mode_without_a_remote_configured() {
        // Remote mode additionally needs the remote phases, which have no
        // remote to talk to here — assert the *local* commit still happens
        // (the important S7 behavior) even though the sync as a whole then
        // fails on the missing remote.
        let (_temp, config) = repo_with_drift(GitMode::Remote);
        let vault_path = config.vault_path.clone();
        let _handle = spawn_sync_task(
            config,
            Arc::new(Mutex::new(())),
            SyncOps {
                commit: Box::new(crate::git::commit_local),
                fetch: Box::new(|_| Err(GitError::Remote("no remote in test".into()))),
                integrate: Box::new(|_| Ok(())),
                push: Box::new(|_| Err(GitError::Remote("no remote in test".into()))),
            },
        );

        assert!(
            wait_for_head_message(&vault_path, STARTUP_DRIFT_MESSAGE).await,
            "startup flush never committed the pre-existing drift"
        );
    }

    /// The vault-write lock must be FREE while a network phase (fetch/push) is in
    /// flight, so a slow or hanging remote cannot block concurrent HTTP/MCP vault
    /// writes. We make `fetch` block (simulating a hung remote) and assert the
    /// lock is acquirable while it blocks.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn network_phase_does_not_hold_vault_lock() {
        let config = unused_config(1);
        let lock = Arc::new(Mutex::new(()));

        let fetch_entered = Arc::new(AtomicBool::new(false));
        let release_fetch = Arc::new(AtomicBool::new(false));
        let entered_for_fetch = fetch_entered.clone();
        let release_for_fetch = release_fetch.clone();

        let ops = SyncOps {
            commit: Box::new(|_c, _p, _m| {
                Ok(CommitOutcome {
                    committed: true,
                    needs_remote: true,
                })
            }),
            fetch: Box::new(move |_c| {
                // Simulate a hung remote: signal entry, then block until released.
                entered_for_fetch.store(true, Ordering::SeqCst);
                while !release_for_fetch.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Ok(())
            }),
            integrate: Box::new(|_| Ok(())),
            push: Box::new(|_| Ok(())),
        };

        let handle = spawn_sync_task(config, lock.clone(), ops);
        handle.record(WriteRecord {
            op: "update".into(),
            target: "n".into(),
            affected_paths: vec![PathBuf::from("/v/n.md")],
            summary: None,
        });

        // Wait (real time, past the 1s debounce) until fetch is blocking.
        let mut entered = false;
        for _ in 0..400 {
            if fetch_entered.load(Ordering::SeqCst) {
                entered = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(entered, "fetch phase never ran");

        // The network phase is blocked; the vault lock must NOT be held by the
        // sync task (it is only held across the local commit/integrate phases).
        assert!(
            lock.try_lock().is_ok(),
            "vault write lock is held during the network fetch phase"
        );

        release_fetch.store(true, Ordering::SeqCst);
    }

    #[tokio::test(start_paused = true)]
    async fn coalesces_records_into_single_sync() {
        let calls = Arc::new(AtomicUsize::new(0));
        let batch_sizes = Arc::new(Mutex::new(Vec::<usize>::new()));
        let calls_for_runner = calls.clone();
        let sizes_for_runner = batch_sizes.clone();

        let config = GitConfig {
            vault_path: std::path::PathBuf::from("/unused"),
            mode: crate::git::config::GitMode::Remote,
            remote: "origin".into(),
            branch: "main".into(),
            username: "u".into(),
            token: "t".into(),
            debounce_seconds: 5,
            author_name: "n".into(),
            author_email: "e".into(),
        };
        let lock = Arc::new(Mutex::new(()));

        let handle = spawn_sync_task(
            config,
            lock,
            SyncOps {
                commit: Box::new(move |_cfg, paths, _msg| {
                    calls_for_runner.fetch_add(1, Ordering::SeqCst);
                    sizes_for_runner.blocking_lock().push(paths.len());
                    Ok(CommitOutcome {
                        committed: true,
                        needs_remote: false,
                    })
                }),
                fetch: Box::new(|_| Ok(())),
                integrate: Box::new(|_| Ok(())),
                push: Box::new(|_| Ok(())),
            },
        );

        for i in 0..3 {
            handle.record(WriteRecord {
                op: "update".into(),
                target: format!("n{i}"),
                affected_paths: vec![std::path::PathBuf::from(format!("/v/n{i}.md"))],
                summary: None,
            });
        }

        // Let the task drain the channel and arm its debounce timer before we
        // jump the clock; otherwise `advance` fires no timer.
        tokio::task::yield_now().await;

        // Advance past the debounce window and let the task run.
        tokio::time::advance(Duration::from_secs(6)).await;
        tokio::time::sleep(Duration::from_millis(1)).await;

        assert_eq!(calls.load(Ordering::SeqCst), 1, "one coalesced sync");
        assert_eq!(batch_sizes.lock().await.as_slice(), &[3]);
    }

    #[tokio::test(start_paused = true)]
    async fn failed_sync_is_retried_without_a_new_write() {
        // A transient remote failure must self-heal: after a sync whose push
        // fails, the task re-attempts on a backoff timer even though no further
        // write arrives, instead of stranding the commit unpushed until restart.
        let push_calls = Arc::new(AtomicUsize::new(0));
        let pc = push_calls.clone();
        let config = unused_config(1);
        let lock = Arc::new(Mutex::new(()));
        let handle = spawn_sync_task(
            config,
            lock,
            SyncOps {
                commit: Box::new(|_c, _p, _m| {
                    Ok(CommitOutcome {
                        committed: true,
                        needs_remote: true,
                    })
                }),
                fetch: Box::new(|_| Ok(())),
                integrate: Box::new(|_| Ok(())),
                push: Box::new(move |_| {
                    pc.fetch_add(1, Ordering::SeqCst);
                    Err(GitError::Remote("remote down".into()))
                }),
            },
        );

        handle.record(WriteRecord {
            op: "update".into(),
            target: "n".into(),
            affected_paths: vec![PathBuf::from("/v/n.md")],
            summary: None,
        });
        tokio::task::yield_now().await;
        // Past the 1s debounce: first sync runs, push #1 fails.
        tokio::time::advance(Duration::from_secs(2)).await;
        tokio::time::sleep(Duration::from_millis(1)).await;
        // No new write. Past the retry backoff: push must be attempted again.
        tokio::time::advance(Duration::from_secs(30)).await;
        tokio::time::sleep(Duration::from_millis(1)).await;

        assert!(
            push_calls.load(Ordering::SeqCst) >= 2,
            "a failed sync should be retried without a new write (got {} push attempts)",
            push_calls.load(Ordering::SeqCst)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn records_error_in_status() {
        let config = GitConfig {
            vault_path: std::path::PathBuf::from("/unused"),
            mode: crate::git::config::GitMode::Remote,
            remote: "origin".into(),
            branch: "main".into(),
            username: "u".into(),
            token: "t".into(),
            debounce_seconds: 1,
            author_name: "n".into(),
            author_email: "e".into(),
        };
        let lock = Arc::new(Mutex::new(()));
        let handle = spawn_sync_task(
            config,
            lock,
            SyncOps {
                commit: Box::new(|_c, _p, _m| Err(GitError::Remote("boom".into()))),
                fetch: Box::new(|_| Ok(())),
                integrate: Box::new(|_| Ok(())),
                push: Box::new(|_| Ok(())),
            },
        );
        let status = handle.status();

        handle.record(WriteRecord {
            op: "update".into(),
            target: "n".into(),
            affected_paths: vec![std::path::PathBuf::from("/v/n.md")],
            summary: None,
        });
        // Let the task drain the channel and arm its debounce timer first.
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(2)).await;
        tokio::time::sleep(Duration::from_millis(1)).await;

        let guard = status.read().await;
        assert!(!guard.last_ok);
        assert_eq!(guard.last_error.as_deref(), Some("git remote error: boom"));
    }

    #[tokio::test]
    async fn stopping_drains_the_queued_batch_before_the_task_exits() {
        let commits = Arc::new(AtomicUsize::new(0));
        let committed = commits.clone();
        let handle = spawn_sync_task(
            unused_config(60),
            Arc::new(Mutex::new(())),
            SyncOps {
                commit: Box::new(move |_config, _paths, _message| {
                    committed.fetch_add(1, Ordering::SeqCst);
                    Ok(CommitOutcome {
                        committed: true,
                        needs_remote: false,
                    })
                }),
                fetch: Box::new(|_| Ok(())),
                integrate: Box::new(|_| Ok(())),
                push: Box::new(|_| Ok(())),
            },
        );
        handle.record(WriteRecord {
            op: "update".into(),
            target: "note".into(),
            affected_paths: vec![PathBuf::from("/vault/note.md")],
            summary: None,
        });
        handle
            .stop(Duration::from_secs(2))
            .await
            .expect("drained stop");
        assert_eq!(commits.load(Ordering::SeqCst), 1);
    }

    /// S3 regression: `stop` timing out while a sync is genuinely still
    /// running must not permanently strand the handle. Once the in-flight
    /// sync finishes, the handle must keep accepting `record()` calls and a
    /// later write must still get committed, instead of every future write
    /// being silently dropped by a handle nobody is draining.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_timed_out_stop_leaves_the_handle_working_for_a_later_write() {
        let commits = Arc::new(AtomicUsize::new(0));
        let commit_entered = Arc::new(AtomicBool::new(false));
        let release_commit = Arc::new(AtomicBool::new(false));

        let committed = commits.clone();
        let entered = commit_entered.clone();
        let release = release_commit.clone();
        let handle = spawn_sync_task(
            unused_config(1),
            Arc::new(Mutex::new(())),
            SyncOps {
                commit: Box::new(move |_config, _paths, _message| {
                    let call = committed.fetch_add(1, Ordering::SeqCst);
                    if call == 0 {
                        // Simulate a slow first commit: block until released,
                        // so a `stop` racing it genuinely cannot drain in time.
                        entered.store(true, Ordering::SeqCst);
                        while !release.load(Ordering::SeqCst) {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                    }
                    Ok(CommitOutcome {
                        committed: true,
                        needs_remote: false,
                    })
                }),
                fetch: Box::new(|_| Ok(())),
                integrate: Box::new(|_| Ok(())),
                push: Box::new(|_| Ok(())),
            },
        );

        handle.record(WriteRecord {
            op: "update".into(),
            target: "first".into(),
            affected_paths: vec![PathBuf::from("/vault/first.md")],
            summary: None,
        });

        // Wait (real time, past the 1s debounce) until the slow commit is
        // blocking.
        let mut entered = false;
        for _ in 0..400 {
            if commit_entered.load(Ordering::SeqCst) {
                entered = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(entered, "first commit never started");

        // A short timeout cannot possibly drain a commit that is deliberately
        // still blocking.
        let result = handle.stop(Duration::from_millis(50)).await;
        assert!(result.is_err(), "stop should time out while sync is stuck");

        // Let the first (slow) commit finish.
        release_commit.store(true, Ordering::SeqCst);
        for _ in 0..400 {
            if commits.load(Ordering::SeqCst) >= 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        // The handle must still be usable: a later write is queued and
        // eventually committed rather than silently dropped.
        handle.record(WriteRecord {
            op: "update".into(),
            target: "second".into(),
            affected_paths: vec![PathBuf::from("/vault/second.md")],
            summary: None,
        });

        let mut second_committed = false;
        for _ in 0..400 {
            if commits.load(Ordering::SeqCst) >= 2 {
                second_committed = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            second_committed,
            "a write recorded after a timed-out stop must still be committed (got {} commits)",
            commits.load(Ordering::SeqCst)
        );
    }
}
