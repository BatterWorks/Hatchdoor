use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use notify::{
    Config, Event, RecommendedWatcher, RecursiveMode, Watcher,
    event::{MetadataKind, ModifyKind},
};
use tokio::sync::{broadcast, mpsc, watch};
use tracing::{debug, info, warn};

use crate::vault::ExcludeMatcher;
use crate::vault_registry::VaultId;

pub const WATCH_DEBOUNCE: Duration = Duration::from_millis(500);
/// A quiet debounce keeps a save burst together, but it must not let a busy
/// editor defer cache freshness forever.
pub const WATCH_MAX_DEBOUNCE: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct VaultWatcherHandle {
    inner: Arc<VaultWatcherControl>,
}

struct VaultWatcherControl {
    cancelled: AtomicBool,
    cancel: watch::Sender<bool>,
    task: tokio::task::AbortHandle,
}

impl Drop for VaultWatcherControl {
    fn drop(&mut self) {
        let _ = self.cancel.send(true);
        self.task.abort();
    }
}

impl VaultWatcherHandle {
    /// Stop this Vault's watcher without affecting any other Vault runtime.
    pub fn cancel(&self) {
        if !self.inner.cancelled.swap(true, Ordering::SeqCst) {
            let _ = self.inner.cancel.send(true);
            self.inner.task.abort();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::SeqCst)
    }
}

/// Start one independently cancellable watcher that reports only the changed
/// Vault's identity. Queueing and coalescing these intents belongs to #89.
pub fn spawn_vault_change_watcher(
    vault_id: VaultId,
    vault_path: PathBuf,
    cache_db_path: PathBuf,
    exclude: ExcludeMatcher,
    changes: broadcast::Sender<VaultId>,
) -> Result<VaultWatcherHandle, String> {
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let mut watcher = RecommendedWatcher::new(
        move |result| {
            if event_tx.send(result).is_err() {
                debug!(%vault_id, "Vault watcher receiver closed");
            }
        },
        Config::default(),
    )
    .map_err(|error| format!("failed to create watcher: {error}"))?;
    watcher
        .watch(&vault_path, RecursiveMode::Recursive)
        .map_err(|error| format!("failed to watch {}: {error}", vault_path.display()))?;
    let (cancel, cancel_rx) = watch::channel(false);
    let task = tokio::spawn(run_vault_change_watcher(
        watcher,
        vault_id,
        vault_path,
        cache_db_path,
        exclude,
        changes,
        event_rx,
        cancel_rx,
    ));
    Ok(VaultWatcherHandle {
        inner: Arc::new(VaultWatcherControl {
            cancelled: AtomicBool::new(false),
            cancel,
            task: task.abort_handle(),
        }),
    })
}

#[allow(clippy::too_many_arguments)]
async fn run_vault_change_watcher(
    _watcher: RecommendedWatcher,
    vault_id: VaultId,
    vault_path: PathBuf,
    cache_db_path: PathBuf,
    exclude: ExcludeMatcher,
    changes: broadcast::Sender<VaultId>,
    mut event_rx: mpsc::UnboundedReceiver<notify::Result<Event>>,
    mut cancel: watch::Receiver<bool>,
) {
    info!(%vault_id, vault_path = %vault_path.display(), "Vault watcher started");
    loop {
        tokio::select! {
            changed = cancel.changed() => {
                if changed.is_err() || *cancel.borrow() {
                    break;
                }
            }
            result = event_rx.recv() => {
                let Some(result) = result else {
                    break;
                };
                match result {
                    Ok(event) if should_refresh_for_event(
                        &event,
                        &cache_db_path,
                        &vault_path,
                        &exclude,
                    ) => {
                        debounce_events(
                            &mut event_rx,
                            &cache_db_path,
                            &vault_path,
                            &exclude,
                        )
                        .await;
                        let _ = changes.send(vault_id);
                    }
                    Ok(_) => {}
                    Err(error) => warn!(%vault_id, "Vault watcher event error: {error}"),
                }
            }
        }
    }
    info!(%vault_id, "Vault watcher stopped");
}

/// Wait for the burst that just started to go quiet before the caller reports
/// the change. Each qualifying event restarts the `WATCH_DEBOUNCE` window, so a
/// single save is reported once; `WATCH_MAX_DEBOUNCE` caps how long that
/// restarting may defer the report. A burst that keeps writing is therefore
/// reported no later than the ceiling after its window opened, and the next
/// event after that opens the next window.
async fn debounce_events(
    event_rx: &mut mpsc::UnboundedReceiver<notify::Result<Event>>,
    cache_db_path: &Path,
    vault_path: &Path,
    exclude: &ExcludeMatcher,
) {
    let timer = tokio::time::sleep(WATCH_DEBOUNCE);
    tokio::pin!(timer);
    let ceiling = tokio::time::sleep(WATCH_MAX_DEBOUNCE);
    tokio::pin!(ceiling);

    loop {
        tokio::select! {
            _ = &mut timer => break,
            _ = &mut ceiling => break,
            Some(result) = event_rx.recv() => {
                match result {
                    Ok(event) if should_refresh_for_event(&event, cache_db_path, vault_path, exclude) => {
                        timer.as_mut().reset(tokio::time::Instant::now() + WATCH_DEBOUNCE);
                    }
                    Ok(_) => {}
                    Err(error) => warn!("Vault watcher event error: {error}"),
                }
            }
        }
    }
}

pub fn should_refresh_for_event(
    event: &Event,
    cache_db_path: &Path,
    vault_path: &Path,
    exclude: &ExcludeMatcher,
) -> bool {
    if !refreshable_event_kind(event) {
        return false;
    }

    event.paths.iter().any(|path| {
        !is_cache_path(path, cache_db_path)
            && !is_git_path(path)
            && !is_noise_path(path, vault_path, exclude)
    })
}

/// True when the changed path is deployment noise (matches a built-in or
/// `HATCHDOOR_EXCLUDE` pattern) and so must not trigger a reindex. The
/// `.hatchdoor-layer` marker is never noise — `ExcludeMatcher::is_excluded`
/// exempts it — so a marker change still refreshes. A path outside the vault
/// (not prefix-comparable) is not treated as noise; the cache/git guards handle
/// those cases separately.
fn is_noise_path(path: &Path, vault_path: &Path, exclude: &ExcludeMatcher) -> bool {
    let absolute_path = absolute_clean_path(path);
    let absolute_vault = absolute_clean_path(vault_path);
    let Ok(relative) = absolute_path.strip_prefix(&absolute_vault) else {
        return false;
    };
    exclude.is_excluded(relative, absolute_path.is_dir())
}

/// True when the path lives inside a `.git` directory. Git's own bookkeeping
/// (and the commits/fetches/merges performed by git sync) must not trigger a
/// vault reindex, or every sync would cause a reindex storm.
fn is_git_path(path: &Path) -> bool {
    path.components()
        .any(|component| matches!(component, Component::Normal(name) if name == ".git"))
}

fn refreshable_event_kind(event: &Event) -> bool {
    match event.kind {
        notify::EventKind::Access(_) => false,
        notify::EventKind::Modify(ModifyKind::Metadata(MetadataKind::AccessTime)) => false,
        notify::EventKind::Create(_)
        | notify::EventKind::Modify(_)
        | notify::EventKind::Remove(_) => true,
        notify::EventKind::Any | notify::EventKind::Other => false,
    }
}

fn is_cache_path(path: &Path, cache_db_path: &Path) -> bool {
    let path = absolute_clean_path(path);
    let cache = absolute_clean_path(cache_db_path);
    if path == cache {
        return true;
    }

    let Some(path_parent) = path.parent() else {
        return false;
    };
    let Some(cache_parent) = cache.parent() else {
        return false;
    };
    if path_parent != cache_parent {
        return false;
    }

    let Some(cache_file_name) = cache.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let Some(path_file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };

    ["journal", "wal", "shm"]
        .iter()
        .any(|suffix| path_file_name == format!("{cache_file_name}-{suffix}"))
}

fn absolute_clean_path(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };

    let mut clean = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                clean.pop();
            }
            _ => clean.push(component.as_os_str()),
        }
    }
    clean
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;
    use notify::{
        EventKind,
        event::{AccessKind, AccessMode, ModifyKind},
    };
    use tempfile::tempdir;

    fn default_exclude() -> ExcludeMatcher {
        ExcludeMatcher::default()
    }

    #[test]
    fn should_refresh_for_event_ignores_cache_database() {
        let dir = tempdir().expect("temp dir");
        let cache = dir.path().join("cache.sqlite3");
        let event = Event::new(EventKind::Modify(ModifyKind::Data(
            notify::event::DataChange::Content,
        )))
        .add_path(cache.clone());

        assert!(!should_refresh_for_event(
            &event,
            &cache,
            dir.path(),
            &default_exclude()
        ));
    }

    #[test]
    fn should_refresh_for_event_ignores_sqlite_cache_sidecars() {
        let dir = tempdir().expect("temp dir");
        let cache = dir.path().join("cache.sqlite3");

        for suffix in ["journal", "wal", "shm"] {
            let event = Event::new(EventKind::Modify(ModifyKind::Data(
                notify::event::DataChange::Content,
            )))
            .add_path(dir.path().join(format!("cache.sqlite3-{suffix}")));

            assert!(
                !should_refresh_for_event(&event, &cache, dir.path(), &default_exclude()),
                "{suffix} should be ignored"
            );
        }
    }

    #[test]
    fn should_refresh_for_event_ignores_relative_cache_path() {
        let relative_cache = PathBuf::from("./data/cache/cache.sqlite3");
        let absolute_sidecar = std::env::current_dir()
            .expect("current dir")
            .join("data/cache/cache.sqlite3-wal");
        let event = Event::new(EventKind::Modify(ModifyKind::Data(
            notify::event::DataChange::Content,
        )))
        .add_path(absolute_sidecar);

        assert!(!should_refresh_for_event(
            &event,
            &relative_cache,
            Path::new("./data/cache"),
            &default_exclude()
        ));
    }

    #[test]
    fn should_refresh_for_event_ignores_git_directory() {
        let dir = tempdir().expect("temp dir");
        let cache = dir.path().join("cache.sqlite3");

        for relative in [".git/index", ".git/refs/heads/main", ".git/objects/ab/cdef"] {
            let event = Event::new(EventKind::Modify(ModifyKind::Data(
                notify::event::DataChange::Content,
            )))
            .add_path(dir.path().join(relative));

            assert!(
                !should_refresh_for_event(&event, &cache, dir.path(), &default_exclude()),
                "{relative} should be ignored"
            );
        }
    }

    #[test]
    fn should_refresh_for_event_accepts_vault_file_changes() {
        let dir = tempdir().expect("temp dir");
        let cache = dir.path().join("cache.sqlite3");
        let event = Event::new(EventKind::Modify(ModifyKind::Data(
            notify::event::DataChange::Content,
        )))
        .add_path(dir.path().join("Home.md"));

        assert!(should_refresh_for_event(
            &event,
            &cache,
            dir.path(),
            &default_exclude()
        ));
    }

    #[test]
    fn should_refresh_for_event_ignores_non_mutating_access_events() {
        let dir = tempdir().expect("temp dir");
        let cache = dir.path().join("cache.sqlite3");

        for kind in [
            EventKind::Access(AccessKind::Read),
            EventKind::Access(AccessKind::Open(AccessMode::Read)),
            EventKind::Access(AccessKind::Close(AccessMode::Read)),
            EventKind::Modify(ModifyKind::Metadata(MetadataKind::AccessTime)),
        ] {
            let event = Event::new(kind).add_path(dir.path().join("Home.md"));

            assert!(
                !should_refresh_for_event(&event, &cache, dir.path(), &default_exclude()),
                "{kind:?} should be ignored"
            );
        }
    }

    #[test]
    fn should_refresh_for_event_accepts_write_metadata_changes() {
        let dir = tempdir().expect("temp dir");
        let cache = dir.path().join("cache.sqlite3");
        let event = Event::new(EventKind::Modify(ModifyKind::Metadata(
            MetadataKind::WriteTime,
        )))
        .add_path(dir.path().join("Home.md"));

        assert!(should_refresh_for_event(
            &event,
            &cache,
            dir.path(),
            &default_exclude()
        ));
    }

    #[test]
    fn should_refresh_for_event_ignores_noise_paths() {
        let dir = tempdir().expect("temp dir");
        let cache = dir.path().join("cache.sqlite3");

        for relative in [
            ".obsidian/workspace.json",
            ".trash/Deleted.md",
            "notes/scratch.tmp",
            "notes/A.sync-conflict-2026.md",
        ] {
            let event = Event::new(EventKind::Modify(ModifyKind::Data(
                notify::event::DataChange::Content,
            )))
            .add_path(dir.path().join(relative));

            assert!(
                !should_refresh_for_event(&event, &cache, dir.path(), &default_exclude()),
                "{relative} is noise and must not trigger a reindex"
            );
        }
    }

    #[test]
    fn should_refresh_for_event_accepts_layer_marker_changes() {
        // The `.hatchdoor-layer` marker is a dotfile but is never noise: a
        // create/modify/delete must trigger a full reindex so the marker set is
        // re-classified.
        let dir = tempdir().expect("temp dir");
        let cache = dir.path().join("cache.sqlite3");

        for kind in [
            EventKind::Create(notify::event::CreateKind::File),
            EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Content)),
            EventKind::Remove(notify::event::RemoveKind::File),
        ] {
            let event = Event::new(kind).add_path(dir.path().join("sources/.hatchdoor-layer"));

            assert!(
                should_refresh_for_event(&event, &cache, dir.path(), &default_exclude()),
                "{kind:?} on a layer marker must trigger a reindex"
            );
        }
    }

    #[test]
    fn should_refresh_for_event_respects_user_exclude_patterns() {
        let dir = tempdir().expect("temp dir");
        let cache = dir.path().join("cache.sqlite3");
        let exclude = ExcludeMatcher::new(&["build/".to_string()]).expect("matcher");

        let noise = Event::new(EventKind::Modify(ModifyKind::Data(
            notify::event::DataChange::Content,
        )))
        .add_path(dir.path().join("build/Generated.md"));
        assert!(
            !should_refresh_for_event(&noise, &cache, dir.path(), &exclude),
            "a path under a HATCHDOOR_EXCLUDE pattern must not trigger a reindex"
        );

        let content = Event::new(EventKind::Modify(ModifyKind::Data(
            notify::event::DataChange::Content,
        )))
        .add_path(dir.path().join("wiki/Keep.md"));
        assert!(
            should_refresh_for_event(&content, &cache, dir.path(), &exclude),
            "a real content change must still trigger a reindex"
        );
    }

    /// One watcher over an empty Vault directory, holding the temporary
    /// directory alive for as long as the test keeps the watcher.
    struct TestWatcher {
        _dir: tempfile::TempDir,
        vault_path: PathBuf,
        vault_id: VaultId,
        handle: VaultWatcherHandle,
        changes: broadcast::Receiver<VaultId>,
    }

    fn spawn_test_watcher() -> TestWatcher {
        let dir = tempdir().expect("temp dir");
        let vault_path = dir.path().join("vault");
        std::fs::create_dir_all(&vault_path).expect("Vault directory");
        let cache = dir.path().join("cache.sqlite3");
        let vault_id = VaultId::from_str("12345678-1234-4567-89ab-1234567890ab").expect("Vault ID");
        let (changes, receiver) = broadcast::channel(64);
        let handle = spawn_vault_change_watcher(
            vault_id,
            vault_path.clone(),
            cache,
            default_exclude(),
            changes,
        )
        .expect("start per-Vault watcher");

        TestWatcher {
            _dir: dir,
            vault_path,
            vault_id,
            handle,
            changes: receiver,
        }
    }

    #[tokio::test]
    async fn per_vault_watcher_reports_identity_and_can_be_cancelled() {
        let mut watcher = spawn_test_watcher();

        std::fs::write(watcher.vault_path.join("Changed.md"), "# Changed\n")
            .expect("write changed note");
        let changed = tokio::time::timeout(Duration::from_secs(3), watcher.changes.recv())
            .await
            .expect("watcher change timeout")
            .expect("watcher change");
        assert_eq!(changed, watcher.vault_id);

        watcher.handle.cancel();
        assert!(watcher.handle.is_cancelled());
    }

    /// A sustained write burst must not defer the change intent until the burst
    /// stops. `WATCH_MAX_DEBOUNCE` caps the quiet `WATCH_DEBOUNCE` window that
    /// every event restarts, so a writer saving faster than that window is
    /// exactly the case the ceiling exists for. The burst outlasts the deadline
    /// deliberately: a change reported before it ends can only have come from
    /// the ceiling, never from the burst going quiet.
    #[tokio::test]
    async fn a_sustained_write_burst_still_reports_a_change_within_the_debounce_ceiling() {
        let mut watcher = spawn_test_watcher();

        let burst_path = watcher.vault_path.clone();
        let burst = tokio::spawn(async move {
            let interval = WATCH_DEBOUNCE / 2;
            let stop_writing = tokio::time::Instant::now() + WATCH_MAX_DEBOUNCE * 3;
            let mut written = 0;
            while tokio::time::Instant::now() < stop_writing {
                std::fs::write(
                    burst_path.join(format!("Note-{written}.md")),
                    format!("# Note {written}\n"),
                )
                .expect("write burst note");
                written += 1;
                tokio::time::sleep(interval).await;
            }
        });

        let deadline = WATCH_MAX_DEBOUNCE + WATCH_DEBOUNCE + Duration::from_secs(3);
        let changed = tokio::time::timeout(deadline, watcher.changes.recv()).await;
        burst.abort();
        watcher.handle.cancel();

        let changed = changed
            .expect("a sustained burst must report a change within the debounce ceiling")
            .expect("watcher change");
        assert_eq!(changed, watcher.vault_id);
    }
}
