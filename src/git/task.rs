use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex, RwLock, mpsc};
use tracing::{error, info, warn};

use super::config::GitConfig;
use super::message::{WriteRecord, build_commit_message};
use super::status::GitSyncStatus;
use super::sync::{GitError, SyncOutcome};

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
/// also acquired by MCP write tools. `runner` performs the actual git work; in
/// production this calls `super::sync::sync`, and tests inject a fake.
pub fn spawn_sync_task<R>(config: GitConfig, vault_lock: Arc<Mutex<()>>, runner: R) -> GitSyncHandle
where
    R: Fn(&GitConfig, &[std::path::PathBuf], &str) -> Result<SyncOutcome, GitError>
        + Send
        + Sync
        + 'static,
{
    let (sender, receiver) = mpsc::unbounded_channel();
    let status = Arc::new(RwLock::new(GitSyncStatus::enabled()));
    let task_status = status.clone();
    let debounce = Duration::from_secs(config.debounce_seconds.max(1));
    let runner = Arc::new(runner);

    tokio::spawn(async move {
        run_loop(config, debounce, vault_lock, receiver, task_status, runner).await;
    });

    GitSyncHandle { sender, status }
}

async fn run_loop<R>(
    config: GitConfig,
    debounce: Duration,
    vault_lock: Arc<Mutex<()>>,
    mut receiver: mpsc::UnboundedReceiver<WriteRecord>,
    status: Arc<RwLock<GitSyncStatus>>,
    runner: Arc<R>,
) where
    R: Fn(&GitConfig, &[std::path::PathBuf], &str) -> Result<SyncOutcome, GitError>
        + Send
        + Sync
        + 'static,
{
    let mut batch: Vec<WriteRecord> = Vec::new();

    loop {
        // Wait for the first record (or channel close).
        let first = match receiver.recv().await {
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

        run_one_sync(
            &config,
            &vault_lock,
            &status,
            &runner,
            std::mem::take(&mut batch),
        )
        .await;
        update_pending(&status, 0).await;
    }
}

async fn run_one_sync<R>(
    config: &GitConfig,
    vault_lock: &Arc<Mutex<()>>,
    status: &Arc<RwLock<GitSyncStatus>>,
    runner: &Arc<R>,
    batch: Vec<WriteRecord>,
) where
    R: Fn(&GitConfig, &[std::path::PathBuf], &str) -> Result<SyncOutcome, GitError>
        + Send
        + Sync
        + 'static,
{
    let message = build_commit_message(&batch);
    let mut paths: Vec<std::path::PathBuf> = batch
        .iter()
        .flat_map(|r| r.affected_paths.clone())
        .collect();
    paths.sort();
    paths.dedup();

    // Hold the vault lock across the blocking git work so no MCP write races it.
    let _guard = vault_lock.lock().await;
    let config_clone = config.clone();
    let runner = runner.clone();
    let result = tokio::task::spawn_blocking(move || runner(&config_clone, &paths, &message))
        .await
        .unwrap_or_else(|join_err| Err(GitError::Other(format!("sync task panicked: {join_err}"))));
    drop(_guard);

    let mut guard = status.write().await;
    guard.last_sync_at = Some(now_rfc3339());
    match result {
        Ok(outcome) => {
            guard.last_ok = true;
            guard.last_error = None;
            match outcome {
                SyncOutcome::NoChanges => info!("git sync: no changes"),
                SyncOutcome::Pushed { committed } => {
                    info!(committed, "git sync: pushed")
                }
            }
        }
        Err(err) => {
            guard.last_ok = false;
            let message = err.to_string();
            match &err {
                GitError::Conflict { .. } => warn!("git sync conflict: {message}"),
                _ => error!("git sync failed: {message}"),
            }
            guard.last_error = Some(message);
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
    use std::sync::atomic::{AtomicUsize, Ordering};

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

        let handle = spawn_sync_task(config, lock, move |_cfg, paths, _msg| {
            calls_for_runner.fetch_add(1, Ordering::SeqCst);
            sizes_for_runner.blocking_lock().push(paths.len());
            Ok(SyncOutcome::Pushed { committed: true })
        });

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
        let handle = spawn_sync_task(config, lock, move |_c, _p, _m| {
            Err(GitError::Remote("boom".into()))
        });
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
