use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use axum::Json;
use axum::http::StatusCode;
use tokio::sync::{RwLock, broadcast};
use tracing::{debug, error, info};

use crate::api_types::ErrorResponse;
use crate::cache::SqliteCache;
use crate::embed::Embedder;
use crate::startup::{IndexingProgressSnapshot, StartupTracker};
use crate::vault::{VaultIndex, seed_empty_vault};

#[derive(Clone)]
pub struct AppState {
    pub vault_path: PathBuf,
    pub cache: Arc<RwLock<VaultCache>>,
    pub vault_revision: Arc<AtomicU64>,
    pub vault_events: broadcast::Sender<u64>,
    pub embedder: Arc<dyn Embedder>,
    /// True when the web API is protected by `HATCHDOOR_WEB_BEARER_TOKEN`.
    pub web_auth_enabled: bool,
    /// True when public demo browsing is enabled and app-level writes are blocked.
    pub demo_mode: bool,
    /// Serializes vault file mutations against git sync tree operations.
    pub vault_write_lock: Arc<tokio::sync::Mutex<()>>,
    /// Present only when git sync is enabled.
    pub git_sync: Arc<OnceLock<crate::git::GitSyncHandle>>,
    /// Validated MCP configuration, parsed once at startup.
    pub mcp_config: Arc<crate::mcp::McpConfig>,
    /// Folder prefix treated as archived in resolve results.
    pub archive_prefix: Arc<str>,
    /// Held while a reindex runs so concurrent refreshes coalesce into one.
    pub refresh_lock: Arc<tokio::sync::Mutex<()>>,
    pub startup: StartupTracker,
}

impl AppState {
    /// Record a vault write for git sync. No-op when sync is disabled.
    pub fn record_vault_write(&self, record: crate::git::WriteRecord) {
        if let Some(handle) = self.git_sync.get() {
            handle.record(record);
        }
    }
}

pub struct VaultCache {
    pub sqlite: Arc<SqliteCache>,
}

pub fn build_cache(vault_path: &PathBuf, embedder: &dyn Embedder) -> Result<VaultCache, String> {
    let sqlite = Arc::new(SqliteCache::in_memory(384)?);
    build_cache_with_sqlite(vault_path, sqlite, embedder)
}

pub fn build_cache_with_sqlite(
    vault_path: &PathBuf,
    sqlite: Arc<SqliteCache>,
    embedder: &dyn Embedder,
) -> Result<VaultCache, String> {
    build_cache_with_sqlite_and_progress(vault_path, sqlite, embedder, None)
}

pub fn build_cache_with_sqlite_and_progress(
    vault_path: &PathBuf,
    sqlite: Arc<SqliteCache>,
    embedder: &dyn Embedder,
    on_progress: Option<Arc<dyn Fn(IndexingProgressSnapshot) + Send + Sync>>,
) -> Result<VaultCache, String> {
    debug!(vault_path = %vault_path.display(), "Building SQLite vault cache");
    if seed_empty_vault(vault_path).map_err(|e| e.to_string())? {
        info!(
            vault_path = %vault_path.display(),
            "Seeded fresh vault with Hatchdoor starter notes"
        );
    }
    info!("Scanning vault for notes…");
    let scan_started = Instant::now();
    let index = VaultIndex::build(vault_path).map_err(|e| e.to_string())?;
    debug!(
        notes = index.ordered_slugs.len(),
        elapsed_ms = scan_started.elapsed().as_secs_f64() * 1_000.0,
        "Vault scan performance"
    );
    sqlite.replace_from_index_with_embedder_and_progress(&index, embedder, on_progress)?;

    Ok(VaultCache { sqlite })
}

pub async fn sqlite_cache(
    state: &AppState,
) -> Result<Arc<SqliteCache>, (StatusCode, Json<ErrorResponse>)> {
    let guard = state.cache.read().await;
    Ok(guard.sqlite.clone())
}

/// Build a generic `500` response, logging the real detail rather than leaking
/// it (absolute paths, internal error strings) to the client.
pub fn internal_error(detail: impl AsRef<str>) -> (StatusCode, Json<ErrorResponse>) {
    error!(detail = %detail.as_ref(), "Internal server error");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse {
            error: "Internal server error".to_string(),
        }),
    )
}

/// Run blocking (SQLite / embedding) work off the async runtime so it never
/// hogs a tokio worker or stalls other requests.
pub async fn run_blocking<T, F>(f: F) -> Result<T, (StatusCode, Json<ErrorResponse>)>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(internal_error(error)),
        Err(join_error) => Err(internal_error(format!(
            "background task panicked: {join_error}"
        ))),
    }
}

/// Coalescing refresh for the public `/api/refresh` endpoint: if a reindex is
/// already running, skip rather than queue another full pass behind it. This
/// defuses a request loop that would otherwise pin a CPU core (F-02).
pub async fn refresh_coalescing(state: &AppState) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let _refresh_guard = match state.refresh_lock.try_lock() {
        Ok(guard) => guard,
        Err(_) => {
            debug!("Refresh already in progress; coalescing request");
            return Ok(());
        }
    };
    run_reindex(state).await
}

/// Guaranteed refresh for paths that must see their own change reflected (MCP
/// writes, the vault watcher): waits for any in-flight reindex, then reindexes.
pub async fn refresh_now(state: &AppState) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let _refresh_guard = state.refresh_lock.lock().await;
    run_reindex(state).await
}

async fn run_reindex(state: &AppState) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let sqlite = state.cache.read().await.sqlite.clone();
    let vault_path = state.vault_path.clone();
    let embedder = state.embedder.clone();

    // The reindex writes inside a single SQLite transaction; WAL lets readers on
    // pooled connections keep serving the prior snapshot until it commits, so we
    // no longer hold the cache write lock for the whole rebuild (F-03).
    run_blocking(move || {
        info!("Scanning vault for notes…");
        let scan_started = Instant::now();
        let index = VaultIndex::build(&vault_path).map_err(|e| e.to_string())?;
        debug!(
            notes = index.ordered_slugs.len(),
            elapsed_ms = scan_started.elapsed().as_secs_f64() * 1_000.0,
            "Vault scan performance"
        );
        sqlite.replace_from_index_with_embedder(&index, embedder.as_ref())
    })
    .await?;

    debug!(vault_path = %state.vault_path.display(), "SQLite vault cache refreshed");
    broadcast_vault_revision(state);
    Ok(())
}

fn broadcast_vault_revision(state: &AppState) {
    let revision = state.vault_revision.fetch_add(1, Ordering::SeqCst) + 1;
    let _ = state.vault_events.send(revision);
}

#[cfg(test)]
pub fn test_embedder() -> Arc<dyn Embedder> {
    Arc::new(crate::embed::StubEmbedder::new(384))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn build_cache_seeds_fresh_vault_before_indexing() {
        let dir = tempdir().expect("temp dir");
        let vault_path = dir.path().join("vault");
        let embedder = test_embedder();

        let cache = build_cache(&vault_path, embedder.as_ref()).expect("build cache");

        assert!(vault_path.join("README.md").is_file());
        let note = cache
            .sqlite
            .read_note_by_slug("readme")
            .expect("read note")
            .expect("seeded welcome note");
        assert_eq!(note.relative_path, "README");
        assert!(note.content.contains("# Welcome to Hatchdoor"));
    }

    fn state_with_vault(vault_path: PathBuf) -> AppState {
        let embedder = test_embedder();
        let cache = build_cache(&vault_path, embedder.as_ref()).expect("build cache");
        let (vault_events, _) = broadcast::channel(64);
        AppState {
            vault_path,
            cache: Arc::new(RwLock::new(cache)),
            vault_revision: Arc::new(AtomicU64::new(0)),
            vault_events,
            embedder,
            web_auth_enabled: false,
            demo_mode: false,
            vault_write_lock: Arc::new(tokio::sync::Mutex::new(())),
            git_sync: Arc::new(OnceLock::new()),
            mcp_config: Arc::new(crate::mcp::McpConfig::disabled()),
            archive_prefix: Arc::from("90-archive/"),
            refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
            startup: StartupTracker::ready(),
        }
    }

    #[tokio::test]
    async fn refresh_coalescing_surfaces_errors() {
        let dir = tempdir().expect("temp dir");
        let vault_path = dir.path().join("vault");
        std::fs::create_dir_all(&vault_path).expect("create vault");
        std::fs::write(vault_path.join("Home.md"), "home").expect("write note");

        let mut state = state_with_vault(vault_path);
        state.vault_path = dir.path().join("missing-vault");

        let result = refresh_coalescing(&state).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn sqlite_cache_returns_current_cache_without_reindexing() {
        let dir = tempdir().expect("temp dir");
        let vault_path = dir.path().join("vault");
        std::fs::create_dir_all(&vault_path).expect("create vault");
        std::fs::write(vault_path.join("Home.md"), "home").expect("write note");

        let mut state = state_with_vault(vault_path);
        state.vault_path = dir.path().join("missing-vault");

        let result = sqlite_cache(&state).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn refresh_coalescing_broadcasts_revision_after_successful_refresh() {
        let dir = tempdir().expect("temp dir");
        let vault_path = dir.path().join("vault");
        std::fs::create_dir_all(&vault_path).expect("create vault");
        std::fs::write(vault_path.join("Home.md"), "home").expect("write note");
        let state = state_with_vault(vault_path.clone());
        let mut events = state.vault_events.subscribe();

        std::fs::write(vault_path.join("Second.md"), "second").expect("write note");
        refresh_coalescing(&state).await.expect("refresh");

        assert_eq!(events.recv().await.expect("revision"), 1);
    }
}
