use std::hash::{DefaultHasher, Hash, Hasher};
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
/// How often a Vault's folder is compared against what was last seen there.
/// Kernel events stay the fast path; this catches up on a filesystem that
/// delivers none, such as a Windows folder shared into a container by Docker
/// Desktop. Fixed, with no setting (ADR-43).
pub const RESCAN_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub struct VaultWatcherHandle {
    inner: Arc<VaultWatcherControl>,
}

struct VaultWatcherControl {
    cancelled: AtomicBool,
    cancel: watch::Sender<bool>,
    /// The kernel-event task and the folder re-scan task.
    tasks: [tokio::task::AbortHandle; 2],
}

impl VaultWatcherControl {
    fn stop(&self) {
        let _ = self.cancel.send(true);
        for task in &self.tasks {
            task.abort();
        }
    }
}

impl Drop for VaultWatcherControl {
    fn drop(&mut self) {
        self.stop();
    }
}

impl VaultWatcherHandle {
    /// Stop this Vault's watcher without affecting any other Vault runtime.
    pub fn cancel(&self) {
        if !self.inner.cancelled.swap(true, Ordering::SeqCst) {
            self.inner.stop();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::SeqCst)
    }
}

/// Start one independently cancellable watcher that reports only the changed
/// Vault's identity. Queueing and coalescing these intents belongs to #89.
///
/// The handle also owns a folder re-scan every `RESCAN_INTERVAL`, which
/// reports through the same channel when the kernel delivered no event.
pub fn spawn_vault_change_watcher(
    vault_id: VaultId,
    vault_path: PathBuf,
    cache_db_path: PathBuf,
    exclude: ExcludeMatcher,
    changes: broadcast::Sender<VaultId>,
) -> Result<VaultWatcherHandle, String> {
    spawn_watcher_with_rescan_interval(
        vault_id,
        vault_path,
        cache_db_path,
        exclude,
        changes,
        RESCAN_INTERVAL,
    )
}

fn spawn_watcher_with_rescan_interval(
    vault_id: VaultId,
    vault_path: PathBuf,
    cache_db_path: PathBuf,
    exclude: ExcludeMatcher,
    changes: broadcast::Sender<VaultId>,
    rescan_interval: Duration,
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
    let exclude = Arc::new(exclude);
    let rescan = tokio::spawn(run_vault_folder_rescan(
        vault_id,
        vault_path.clone(),
        cache_db_path.clone(),
        Arc::clone(&exclude),
        changes.clone(),
        rescan_interval,
        cancel_rx.clone(),
    ));
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
            tasks: [task.abort_handle(), rescan.abort_handle()],
        }),
    })
}

#[allow(clippy::too_many_arguments)]
async fn run_vault_change_watcher(
    _watcher: RecommendedWatcher,
    vault_id: VaultId,
    vault_path: PathBuf,
    cache_db_path: PathBuf,
    exclude: Arc<ExcludeMatcher>,
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

/// Compare the Vault's folder against what the previous round saw, every
/// `interval`, and report a difference exactly as a kernel event is reported.
/// The first round only sets the baseline: activation already indexes. A
/// round that cannot read the folder keeps the previous baseline and reports
/// nothing, so the next readable round still notices what changed meanwhile.
async fn run_vault_folder_rescan(
    vault_id: VaultId,
    vault_path: PathBuf,
    cache_db_path: PathBuf,
    exclude: Arc<ExcludeMatcher>,
    changes: broadcast::Sender<VaultId>,
    interval: Duration,
    mut cancel: watch::Receiver<bool>,
) {
    let mut rounds = tokio::time::interval(interval);
    // A scan slower than the interval must not be followed by a burst of
    // catch-up scans.
    rounds.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last_seen: Option<FolderFingerprint> = None;
    loop {
        tokio::select! {
            changed = cancel.changed() => {
                if changed.is_err() || *cancel.borrow() {
                    break;
                }
                continue;
            }
            _ = rounds.tick() => {}
        }
        // Walking a large Vault is blocking filesystem work, so it stays off
        // the async workers that serve requests.
        let scan = {
            let vault_path = vault_path.clone();
            let cache_db_path = cache_db_path.clone();
            let exclude = Arc::clone(&exclude);
            tokio::task::spawn_blocking(move || {
                fingerprint_vault_folder(&vault_path, &cache_db_path, &exclude)
            })
        };
        let seen = match scan.await {
            Ok(Ok(seen)) => seen,
            Ok(Err(error)) => {
                warn!(
                    %vault_id,
                    vault_path = %vault_path.display(),
                    "Vault folder re-scan could not read the folder, trying again next round: {error}"
                );
                continue;
            }
            Err(error) => {
                warn!(%vault_id, "Vault folder re-scan failed, trying again next round: {error}");
                continue;
            }
        };
        if last_seen.replace(seen).is_some_and(|before| before != seen) {
            debug!(%vault_id, "Vault folder re-scan found a change");
            let _ = changes.send(vault_id);
        }
    }
}

/// What one re-scan round saw: how many entries count as Vault content, and
/// an order-independent digest of each one's path, kind, size and
/// modification time. Two rounds that differ in any of those differ here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FolderFingerprint {
    entries: u64,
    digest: u64,
}

/// Read names, sizes and modification times under the Vault, never file
/// contents. Anything that cannot be read fails the whole round, so a share
/// that errors now and then is retried instead of being reported as files
/// going away and coming back. The one exception is an entry deleted while
/// the walk was on its way to it, which is an ordinary change.
fn fingerprint_vault_folder(
    vault_path: &Path,
    cache_db_path: &Path,
    exclude: &ExcludeMatcher,
) -> Result<FolderFingerprint, walkdir::Error> {
    let mut fingerprint = FolderFingerprint {
        entries: 0,
        digest: 0,
    };
    let walk = walkdir::WalkDir::new(vault_path)
        .follow_links(false)
        .into_iter()
        // Nothing under `.git` can count, so there is no reason to list it.
        // Excluded folders are still listed: a layer marker counts wherever
        // it sits, exactly as it does for an event and for the index.
        .filter_entry(|entry| entry.depth() == 0 || !is_git_path(entry.path()));
    for entry in walk {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) if vanished_mid_walk(&error) => continue,
            Err(error) => return Err(error),
        };
        if entry.depth() == 0 {
            continue;
        }
        let is_dir = entry.file_type().is_dir();
        // An event asks the filesystem, which follows a link to a folder, so
        // the exclude patterns must see a link the same way here.
        let excluded_as_dir =
            |path: &Path| is_dir || (entry.file_type().is_symlink() && path.is_dir());
        if !is_vault_change_path(
            entry.path(),
            excluded_as_dir,
            cache_db_path,
            vault_path,
            exclude,
        ) {
            continue;
        }
        let mut hasher = DefaultHasher::new();
        entry.path().hash(&mut hasher);
        is_dir.hash(&mut hasher);
        // A folder's own modification time moves whenever anything inside it
        // does, noise included, so a folder counts by its presence alone.
        if !is_dir {
            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(error) if vanished_mid_walk(&error) => continue,
                Err(error) => return Err(error),
            };
            metadata.len().hash(&mut hasher);
            metadata.modified().ok().hash(&mut hasher);
        }
        fingerprint.entries += 1;
        fingerprint.digest = fingerprint.digest.wrapping_add(hasher.finish());
    }
    Ok(fingerprint)
}

/// True for an entry below the Vault root that was listed and then gone. A
/// missing root is never this: it is a folder that cannot be read.
fn vanished_mid_walk(error: &walkdir::Error) -> bool {
    error.depth() > 0
        && error
            .io_error()
            .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound)
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
    // The kernel dropped events it could not queue, so no path list can say
    // what changed. An Index turn is already a full authoritative rescan,
    // which is exactly the recovery this flag asks for (#324). It arrives as
    // `EventKind::Other` with no paths, which the filters below would drop.
    if event.need_rescan() {
        warn!(
            vault_path = %vault_path.display(),
            "Vault watcher lost filesystem events (queue overflow); requesting a full reindex"
        );
        return true;
    }
    if !refreshable_event_kind(event) {
        return false;
    }

    event
        .paths
        .iter()
        .any(|path| is_vault_change_path(path, Path::is_dir, cache_db_path, vault_path, exclude))
}

/// The one rule for whether a change at `path` matters to the Vault, shared
/// by kernel events and the folder re-scan: the cache database and its
/// sidecars, `.git` and noise paths never do. `is_dir` is asked only when the
/// exclude patterns need it, with the absolute path.
fn is_vault_change_path(
    path: &Path,
    is_dir: impl FnOnce(&Path) -> bool,
    cache_db_path: &Path,
    vault_path: &Path,
    exclude: &ExcludeMatcher,
) -> bool {
    !is_cache_path(path, cache_db_path)
        && !is_git_path(path)
        && !is_noise_path(path, is_dir, vault_path, exclude)
}

/// True when the changed path is deployment noise (matches a built-in or
/// `HATCHDOOR_EXCLUDE` pattern) and so must not trigger a reindex. The
/// `.hatchdoor-layer` marker is never noise — `ExcludeMatcher::is_excluded`
/// exempts it — so a marker change still refreshes. A path outside the vault
/// (not prefix-comparable) is not treated as noise; the cache/git guards handle
/// those cases separately.
fn is_noise_path(
    path: &Path,
    is_dir: impl FnOnce(&Path) -> bool,
    vault_path: &Path,
    exclude: &ExcludeMatcher,
) -> bool {
    let absolute_path = absolute_clean_path(path);
    let absolute_vault = absolute_clean_path(vault_path);
    let Ok(relative) = absolute_path.strip_prefix(&absolute_vault) else {
        return false;
    };
    exclude.is_excluded(relative, is_dir(&absolute_path))
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

    /// An inotify queue overflow reaches us as one `Other` event flagged
    /// `Rescan`, with no paths (notify's `Q_OVERFLOW` handling). It means
    /// events were lost, so it must ask for the full rescan an Index turn is,
    /// not be discarded as noise (#324).
    #[test]
    fn should_refresh_for_event_accepts_a_rescan_flag() {
        let dir = tempdir().expect("temp dir");
        let cache = dir.path().join("cache.sqlite3");

        for kind in [EventKind::Other, EventKind::Any] {
            let overflow = Event::new(kind).set_flag(notify::event::Flag::Rescan);
            assert!(
                should_refresh_for_event(&overflow, &cache, dir.path(), &default_exclude()),
                "{kind:?} flagged Rescan must trigger a reindex"
            );
        }

        assert!(
            !should_refresh_for_event(
                &Event::new(EventKind::Other),
                &cache,
                dir.path(),
                &default_exclude()
            ),
            "an unflagged Other event is still noise"
        );
    }

    /// Short enough that a test sees several rounds without waiting a real
    /// minute.
    const TEST_RESCAN_INTERVAL: Duration = Duration::from_millis(40);

    /// The folder re-scan alone, with no kernel watcher beside it, so every
    /// report a test sees can only have come from a re-scan round.
    struct TestRescan {
        _dir: tempfile::TempDir,
        vault_path: PathBuf,
        cache_db_path: PathBuf,
        vault_id: VaultId,
        changes: broadcast::Receiver<VaultId>,
        _cancel: watch::Sender<bool>,
        task: tokio::task::JoinHandle<()>,
    }

    impl Drop for TestRescan {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    impl TestRescan {
        /// `prepare` fills the Vault before the first round sets the baseline.
        async fn start(exclude: ExcludeMatcher, prepare: impl FnOnce(&Path)) -> Self {
            let dir = tempdir().expect("temp dir");
            let vault_path = dir.path().join("vault");
            std::fs::create_dir_all(&vault_path).expect("Vault directory");
            prepare(&vault_path);
            let cache_db_path = vault_path.join("cache.sqlite3");
            let vault_id =
                VaultId::from_str("12345678-1234-4567-89ab-1234567890ab").expect("Vault ID");
            let (changes, receiver) = broadcast::channel(64);
            let (cancel, cancel_rx) = watch::channel(false);
            let task = tokio::spawn(run_vault_folder_rescan(
                vault_id,
                vault_path.clone(),
                cache_db_path.clone(),
                Arc::new(exclude),
                changes,
                TEST_RESCAN_INTERVAL,
                cancel_rx,
            ));
            let mut rescan = TestRescan {
                _dir: dir,
                vault_path,
                cache_db_path,
                vault_id,
                changes: receiver,
                _cancel: cancel,
                task,
            };
            rescan
                .expect_no_report("the first round only sets the baseline")
                .await;
            rescan
        }

        /// Several rounds pass and none of them reports.
        async fn expect_no_report(&mut self, why: &str) {
            tokio::time::sleep(TEST_RESCAN_INTERVAL * 5).await;
            assert!(
                matches!(
                    self.changes.try_recv(),
                    Err(broadcast::error::TryRecvError::Empty)
                ),
                "{why}"
            );
        }

        /// One report arrives, and the rounds after it stay quiet.
        async fn expect_one_report(&mut self, why: &str) {
            let changed = tokio::time::timeout(Duration::from_secs(3), self.changes.recv())
                .await
                .unwrap_or_else(|_| panic!("no report: {why}"))
                .expect("re-scan change");
            assert_eq!(changed, self.vault_id);
            self.expect_no_report(&format!("reported more than once: {why}"))
                .await;
        }
    }

    #[tokio::test]
    async fn rescan_reports_a_file_added_changed_and_removed_once_each() {
        let mut rescan = TestRescan::start(default_exclude(), |_| {}).await;
        let note = rescan.vault_path.join("Kettle.md");

        std::fs::write(&note, "# Kettle\n").expect("add note");
        rescan.expect_one_report("a note was added").await;

        std::fs::write(&note, "# Kettle\n\nDescale it monthly.\n").expect("change note");
        rescan.expect_one_report("a note was changed").await;

        std::fs::remove_file(&note).expect("remove note");
        rescan.expect_one_report("a note was removed").await;
    }

    #[tokio::test]
    async fn rescan_reports_nothing_for_an_unchanged_vault() {
        let mut rescan = TestRescan::start(default_exclude(), |vault| {
            std::fs::create_dir_all(vault.join("notes")).expect("notes folder");
            std::fs::write(vault.join("notes/Home.md"), "# Home\n").expect("write note");
        })
        .await;

        for _ in 0..4 {
            rescan
                .expect_no_report("an unchanged Vault must stay quiet")
                .await;
        }
    }

    #[tokio::test]
    async fn rescan_ignores_cache_git_noise_and_excluded_paths_but_not_a_layer_marker() {
        let exclude = ExcludeMatcher::new(&["build/".to_string()]).expect("matcher");
        let mut rescan = TestRescan::start(exclude, |vault| {
            for folder in [".git/refs", ".obsidian", "notes", "build", "sources"] {
                std::fs::create_dir_all(vault.join(folder)).expect("folder");
            }
        })
        .await;
        let vault = rescan.vault_path.clone();
        let cache = rescan.cache_db_path.clone();

        std::fs::write(&cache, "cache").expect("cache database");
        for suffix in ["journal", "wal", "shm"] {
            std::fs::write(vault.join(format!("cache.sqlite3-{suffix}")), "sidecar")
                .expect("cache sidecar");
        }
        for relative in [
            ".git/index",
            ".git/refs/main",
            ".obsidian/workspace.json",
            "notes/scratch.tmp",
            "notes/A.sync-conflict-2026.md",
            "build/Generated.md",
        ] {
            std::fs::write(vault.join(relative), "noise").expect("noise file");
        }
        rescan
            .expect_no_report("cache, .git, noise and excluded paths are not Vault changes")
            .await;

        std::fs::write(vault.join("sources/.hatchdoor-layer"), "").expect("layer marker");
        rescan.expect_one_report("a layer marker was added").await;

        std::fs::write(vault.join(".obsidian/.hatchdoor-layer"), "").expect("layer marker");
        rescan
            .expect_one_report("a layer marker counts inside an excluded folder too")
            .await;
    }

    /// The round after a change always differs from the round before it, so
    /// a change waits for one round at most, which is one interval.
    #[test]
    fn one_round_is_enough_to_see_a_file_added_changed_or_removed() {
        let dir = tempdir().expect("temp dir");
        let cache = dir.path().join("cache.sqlite3");
        let note = dir.path().join("Kettle.md");
        let round =
            || fingerprint_vault_folder(dir.path(), &cache, &default_exclude()).expect("readable");

        let empty = round();
        assert_eq!(empty, round(), "two rounds over the same folder agree");

        std::fs::write(&note, "# Kettle\n").expect("add note");
        let added = round();
        assert_ne!(empty, added);

        std::fs::write(&note, "# Kettle\n\nDescale it monthly.\n").expect("change note");
        let changed = round();
        assert_ne!(added, changed);

        std::fs::remove_file(&note).expect("remove note");
        assert_eq!(empty, round());
    }

    /// A subfolder that cannot be listed fails the round, like the root: its
    /// notes must not be reported as removed and then as added again.
    #[cfg(unix)]
    #[tokio::test]
    async fn rescan_treats_an_unreadable_subfolder_as_a_failed_round() {
        use std::os::unix::fs::PermissionsExt;

        let mut rescan = TestRescan::start(default_exclude(), |vault| {
            std::fs::create_dir_all(vault.join("notes")).expect("notes folder");
            std::fs::write(vault.join("notes/Home.md"), "# Home\n").expect("write note");
        })
        .await;
        let notes = rescan.vault_path.join("notes");
        let set_mode = |mode| {
            std::fs::set_permissions(&notes, std::fs::Permissions::from_mode(mode))
                .expect("set folder permissions")
        };

        set_mode(0o000);
        if std::fs::read_dir(&notes).is_ok() {
            // Running as root: permissions do not stop the walk.
            set_mode(0o755);
            return;
        }
        rescan
            .expect_no_report("an unreadable subfolder is not a change")
            .await;
        set_mode(0o755);
        rescan
            .expect_no_report("nothing changed while the subfolder was unreadable")
            .await;
    }

    #[tokio::test]
    async fn rescan_survives_an_unreadable_folder_and_reports_nothing_for_it() {
        let mut rescan = TestRescan::start(default_exclude(), |_| {}).await;
        let moved_away = rescan.vault_path.with_file_name("vault-away");

        std::fs::rename(&rescan.vault_path, &moved_away).expect("take the folder away");
        rescan
            .expect_no_report("a folder that cannot be read is not a change")
            .await;

        std::fs::write(moved_away.join("Kettle.md"), "# Kettle\n").expect("add note");
        std::fs::rename(&moved_away, &rescan.vault_path).expect("bring the folder back");
        rescan
            .expect_one_report("the round after a failed one still runs")
            .await;
    }

    #[tokio::test]
    async fn cancelling_the_watcher_handle_stops_the_rescan() {
        assert_stopping_the_handle_stops_the_rescan(|handle| handle.cancel()).await;
    }

    #[tokio::test]
    async fn dropping_the_watcher_handle_stops_the_rescan() {
        assert_stopping_the_handle_stops_the_rescan(drop).await;
    }

    async fn assert_stopping_the_handle_stops_the_rescan(stop: impl FnOnce(VaultWatcherHandle)) {
        let dir = tempdir().expect("temp dir");
        let vault_path = dir.path().join("vault");
        std::fs::create_dir_all(&vault_path).expect("Vault directory");
        let vault_id = VaultId::from_str("12345678-1234-4567-89ab-1234567890ab").expect("Vault ID");
        let (changes, mut receiver) = broadcast::channel(64);
        let handle = spawn_watcher_with_rescan_interval(
            vault_id,
            vault_path.clone(),
            dir.path().join("cache.sqlite3"),
            default_exclude(),
            changes.clone(),
            TEST_RESCAN_INTERVAL,
        )
        .expect("start per-Vault watcher");
        tokio::time::sleep(TEST_RESCAN_INTERVAL * 5).await;

        stop(handle);
        // Let both tasks reach their cancellation point before the change.
        tokio::time::sleep(TEST_RESCAN_INTERVAL * 2).await;
        std::fs::write(vault_path.join("Kettle.md"), "# Kettle\n").expect("add note");
        tokio::time::sleep(TEST_RESCAN_INTERVAL * 8).await;

        assert!(
            matches!(
                receiver.try_recv(),
                Err(broadcast::error::TryRecvError::Empty)
            ),
            "a stopped watcher must not report, by event or by re-scan"
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
