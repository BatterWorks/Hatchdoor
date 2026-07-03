use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex, RwLock, mpsc};
use tracing::{error, info, warn};

use std::path::PathBuf;

use super::config::GitConfig;
use super::message::{WriteRecord, build_commit_message};
use super::status::GitSyncStatus;
use super::sync::{CommitOutcome, GitError, SyncOutcome, has_unpushed, unpushed_count};

/// The four phases of a sync, split so the background task can hold the vault
/// lock only across the local/working-tree phases (`commit`, `integrate`) and
/// release it across the network phases (`fetch`, `push`) — a slow or hanging
/// remote must never block concurrent HTTP/MCP vault writes. In production these
/// are wired to `super::sync::{commit_local, fetch_remote, integrate_fetched,
/// push_branch}`; tests inject fakes.
pub struct SyncOps {
    /// Stage + commit the batch (working tree + index only). Reports whether the
    /// remote phases are needed. Runs UNDER the vault lock.
    pub commit:
        Box<dyn Fn(&GitConfig, &[PathBuf], &str) -> Result<CommitOutcome, GitError> + Send + Sync>,
    /// Fetch the remote branch (network read, no working-tree change). Runs
    /// WITHOUT the vault lock.
    pub fetch: Box<dyn Fn(&GitConfig) -> Result<(), GitError> + Send + Sync>,
    /// Merge the fetched remote into the local branch if it moved ahead (may
    /// checkout the working tree). Runs UNDER the vault lock.
    pub integrate: Box<dyn Fn(&GitConfig) -> Result<(), GitError> + Send + Sync>,
    /// Push the local branch (network write). Runs WITHOUT the vault lock.
    pub push: Box<dyn Fn(&GitConfig) -> Result<(), GitError> + Send + Sync>,
}

/// Backoff bounds for re-attempting a sync that failed for a transient reason
/// (remote/network/auth). Without this, a brief outage strands committed vault
/// edits unpushed until the next write or a full process restart.
const RETRY_BASE: Duration = Duration::from_secs(5);
const RETRY_MAX: Duration = Duration::from_secs(300);

/// Handle stored in AppState so write tools can enqueue records and readers can
/// observe status. `None` everywhere when git sync is disabled.
#[derive(Clone)]
pub struct GitSyncHandle {
    sender: mpsc::UnboundedSender<WriteRecord>,
    status: Arc<RwLock<GitSyncStatus>>,
}

impl GitSyncHandle {
    /// Enqueue a write for the next debounced sync. Never blocks; drops silently
    /// if the background task has stopped (it will be retried by later writes).
    pub fn record(&self, record: WriteRecord) {
        let _ = self.sender.send(record);
    }

    pub fn status(&self) -> Arc<RwLock<GitSyncStatus>> {
        self.status.clone()
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
    let status = Arc::new(RwLock::new(GitSyncStatus::enabled()));
    let task_status = status.clone();
    let debounce = Duration::from_secs(config.debounce_seconds.max(1));
    let ops = Arc::new(ops);

    // Decide up front whether to flush commits stranded by an earlier outage.
    // This is a cheap local check; if it can't read git state (e.g. no repo, as
    // in unit tests) we simply don't flush.
    let startup_flush = matches!(has_unpushed(&config), Ok(true));

    tokio::spawn(async move {
        run_loop(
            config,
            debounce,
            vault_lock,
            receiver,
            task_status,
            ops,
            startup_flush,
        )
        .await;
    });

    GitSyncHandle { sender, status }
}

async fn run_loop(
    config: GitConfig,
    debounce: Duration,
    vault_lock: Arc<Mutex<()>>,
    mut receiver: mpsc::UnboundedReceiver<WriteRecord>,
    status: Arc<RwLock<GitSyncStatus>>,
    ops: Arc<SyncOps>,
    startup_flush: bool,
) {
    let mut batch: Vec<WriteRecord> = Vec::new();
    // When a sync fails transiently, `retry_after` holds the backoff before the
    // next unprompted re-attempt (empty batch). `None` means nothing to retry.
    let mut retry_after: Option<Duration> = None;

    // Immediately flush any commits stranded by an earlier outage, rather than
    // waiting for the first write to trigger a debounced sync.
    if startup_flush && run_one_sync(&config, &vault_lock, &status, &ops, Vec::new()).await {
        retry_after = Some(RETRY_BASE);
    }

    loop {
        // Wait for the first record (or channel close). If a previous sync failed
        // transiently, also race a backoff timer that re-attempts the push with
        // no new write, so a brief remote outage self-heals.
        let first = match next_record_or_retry(
            &mut receiver,
            &config,
            &vault_lock,
            &status,
            &ops,
            &mut retry_after,
        )
        .await
        {
            Some(record) => record,
            None => break,
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
}

/// Await the next queued write. If `retry_after` is set (a previous sync failed
/// transiently), also race a backoff timer: when it fires first, re-attempt the
/// sync with an empty batch and either clear the backoff (success) or grow it
/// toward `RETRY_MAX` (still failing), then keep waiting. Returns `None` only
/// when the channel closes.
async fn next_record_or_retry(
    receiver: &mut mpsc::UnboundedReceiver<WriteRecord>,
    config: &GitConfig,
    vault_lock: &Arc<Mutex<()>>,
    status: &Arc<RwLock<GitSyncStatus>>,
    ops: &Arc<SyncOps>,
    retry_after: &mut Option<Duration>,
) -> Option<WriteRecord> {
    loop {
        let delay = match *retry_after {
            Some(delay) => delay,
            None => return receiver.recv().await,
        };

        let timer = tokio::time::sleep(delay);
        tokio::pin!(timer);
        tokio::select! {
            maybe = receiver.recv() => return maybe,
            _ = &mut timer => {
                let failed = run_one_sync(config, vault_lock, status, ops, Vec::new()).await;
                *retry_after = failed.then(|| (delay * 2).min(RETRY_MAX));
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
    let mut paths: Vec<PathBuf> = batch
        .iter()
        .flat_map(|r| r.affected_paths.clone())
        .collect();
    paths.sort();
    paths.dedup();

    let result = run_sync_phases(config, vault_lock, ops, paths, message).await;

    // Best-effort: read how many local commits remain unpushed afterward. This
    // is a local read (no working-tree change), so it needs no lock.
    let config_clone = config.clone();
    let unpushed = tokio::task::spawn_blocking(move || unpushed_count(&config_clone).ok())
        .await
        .ok()
        .flatten();

    let mut guard = status.write().await;
    guard.last_sync_at = Some(now_rfc3339());
    if let Some(unpushed) = unpushed {
        guard.unpushed = unpushed;
    }
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
            remote: "origin".into(),
            branch: "main".into(),
            username: "u".into(),
            token: "t".into(),
            debounce_seconds,
            author_name: "n".into(),
            author_email: "e".into(),
        }
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
}
