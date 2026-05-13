use std::path::{Path, PathBuf};
use std::time::Duration;

use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::app_state::{AppState, refresh_if_needed};

pub(crate) const WATCH_DEBOUNCE: Duration = Duration::from_millis(500);

pub(crate) fn spawn_vault_watcher(state: AppState, vault_path: PathBuf, cache_db_path: PathBuf) {
    tokio::spawn(async move {
        if let Err(error) = run_vault_watcher(state, vault_path, cache_db_path).await {
            warn!("Vault watcher disabled: {error}");
        }
    });
}

async fn run_vault_watcher(
    state: AppState,
    vault_path: PathBuf,
    cache_db_path: PathBuf,
) -> Result<(), String> {
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    let mut watcher = RecommendedWatcher::new(
        move |result| {
            if event_tx.send(result).is_err() {
                debug!("Vault watcher receiver closed");
            }
        },
        Config::default(),
    )
    .map_err(|error| format!("failed to create watcher: {error}"))?;

    watcher
        .watch(&vault_path, RecursiveMode::Recursive)
        .map_err(|error| format!("failed to watch {}: {error}", vault_path.display()))?;
    info!(vault_path = %vault_path.display(), "Vault watcher started");

    while let Some(result) = event_rx.recv().await {
        match result {
            Ok(event) if should_refresh_for_event(&event, &cache_db_path) => {
                debounce_events(&mut event_rx, &cache_db_path).await;
                if let Err((status, body)) = refresh_if_needed(&state, true).await {
                    error!(
                        status = status.as_u16(),
                        error = %body.0.error,
                        "Vault watcher refresh failed"
                    );
                }
            }
            Ok(_) => {}
            Err(error) => warn!("Vault watcher event error: {error}"),
        }
    }

    Ok(())
}

async fn debounce_events(
    event_rx: &mut mpsc::UnboundedReceiver<notify::Result<Event>>,
    cache_db_path: &Path,
) {
    let timer = tokio::time::sleep(WATCH_DEBOUNCE);
    tokio::pin!(timer);

    loop {
        tokio::select! {
            _ = &mut timer => break,
            Some(result) = event_rx.recv() => {
                match result {
                    Ok(event) if should_refresh_for_event(&event, cache_db_path) => {
                        timer.as_mut().reset(tokio::time::Instant::now() + WATCH_DEBOUNCE);
                    }
                    Ok(_) => {}
                    Err(error) => warn!("Vault watcher event error: {error}"),
                }
            }
        }
    }
}

pub(crate) fn should_refresh_for_event(event: &Event, cache_db_path: &Path) -> bool {
    event
        .paths
        .iter()
        .any(|path| !is_cache_path(path, cache_db_path))
}

fn is_cache_path(path: &Path, cache_db_path: &Path) -> bool {
    path == cache_db_path
        || path
            .canonicalize()
            .ok()
            .zip(cache_db_path.canonicalize().ok())
            .is_some_and(|(path, cache)| path == cache)
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::{EventKind, event::ModifyKind};
    use tempfile::tempdir;

    #[test]
    fn should_refresh_for_event_ignores_cache_database() {
        let dir = tempdir().expect("temp dir");
        let cache = dir.path().join("cache.sqlite3");
        let event = Event::new(EventKind::Modify(ModifyKind::Data(
            notify::event::DataChange::Content,
        )))
        .add_path(cache.clone());

        assert!(!should_refresh_for_event(&event, &cache));
    }

    #[test]
    fn should_refresh_for_event_accepts_vault_file_changes() {
        let dir = tempdir().expect("temp dir");
        let cache = dir.path().join("cache.sqlite3");
        let event = Event::new(EventKind::Modify(ModifyKind::Data(
            notify::event::DataChange::Content,
        )))
        .add_path(dir.path().join("Home.md"));

        assert!(should_refresh_for_event(&event, &cache));
    }

    #[tokio::test]
    async fn debounce_events_coalesces_pending_refresh_events() {
        let dir = tempdir().expect("temp dir");
        let cache = dir.path().join("cache.sqlite3");
        let (tx, mut rx) = mpsc::unbounded_channel();
        let first = Event::new(EventKind::Modify(ModifyKind::Data(
            notify::event::DataChange::Content,
        )))
        .add_path(dir.path().join("Home.md"));
        let second = Event::new(EventKind::Modify(ModifyKind::Data(
            notify::event::DataChange::Content,
        )))
        .add_path(dir.path().join("Second.md"));

        tx.send(Ok(first)).expect("first event");
        tx.send(Ok(second)).expect("second event");
        debounce_events(&mut rx, &cache).await;

        assert!(rx.try_recv().is_err());
    }
}
