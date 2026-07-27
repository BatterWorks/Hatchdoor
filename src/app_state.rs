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
use crate::vault::{VaultIndex, VaultScanConfig, seed_empty_vault};

#[derive(Clone)]
pub struct AppState {
    pub vault_path: PathBuf,
    pub cache_db_path: PathBuf,
    pub cache: Arc<RwLock<VaultCache>>,
    pub vault_revision: Arc<AtomicU64>,
    pub vault_events: broadcast::Sender<u64>,
    /// Fires when a reindex changes the vault's layer marker set, so the MCP
    /// `tools/list` (its per-vault `layers` enum) is now different. A future
    /// streaming MCP transport turns each signal into a
    /// `notifications/tools/list_changed`; today it is the tested seam that
    /// backs the advertised `tools.listChanged` capability.
    pub mcp_tools_changed: broadcast::Sender<()>,
    pub embedder: Arc<dyn Embedder>,
    /// Concrete startup slot behind `embedder`; populated only after a model is
    /// selected and downloaded.
    pub runtime_embedder: Arc<crate::embed::RuntimeEmbedder>,
    pub model_setup: Arc<crate::model_setup::ModelSetup>,
    pub model_setup_started: Arc<std::sync::atomic::AtomicBool>,
    pub startup_git_config: Arc<Option<crate::git::GitConfig>>,
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
    /// Noise-exclusion configuration (built-in defaults plus `HATCHDOOR_EXCLUDE`),
    /// applied to every index build on the server path so the watcher, writes and
    /// startup all see the same excluded set.
    pub scan_config: Arc<VaultScanConfig>,
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
    build_cache_with_sqlite_and_progress(
        vault_path,
        sqlite,
        embedder,
        None,
        &VaultScanConfig::default(),
    )
}

pub fn build_cache_with_sqlite_and_progress(
    vault_path: &PathBuf,
    sqlite: Arc<SqliteCache>,
    embedder: &dyn Embedder,
    on_progress: Option<Arc<dyn Fn(IndexingProgressSnapshot) + Send + Sync>>,
    scan_config: &VaultScanConfig,
) -> Result<VaultCache, String> {
    debug!(vault_path = %vault_path.display(), "Building SQLite vault cache");
    if seed_empty_vault(vault_path, &scan_config.exclude).map_err(|e| e.to_string())? {
        info!(
            vault_path = %vault_path.display(),
            "Seeded fresh vault with Hatchdoor starter notes"
        );
    }
    info!("Scanning vault for notes…");
    let scan_started = Instant::now();
    let index =
        VaultIndex::build_with_config(vault_path, scan_config).map_err(|e| e.to_string())?;
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
    let scan_config = state.scan_config.clone();

    // The marker-set hash the last build persisted. Compared against the value
    // after this reindex to detect a runtime layer change (a marker added,
    // removed, renamed, or its description edited), which changes the MCP
    // `tools/list` `layers` enum.
    let previous_marker_hash = sqlite.get_metadata("marker_set_hash").ok().flatten();

    // The reindex writes inside a single SQLite transaction; WAL lets readers on
    // pooled connections keep serving the prior snapshot until it commits, so we
    // no longer hold the cache write lock for the whole rebuild (F-03).
    run_blocking(move || {
        info!("Scanning vault for notes…");
        let scan_started = Instant::now();
        let index =
            VaultIndex::build_with_config(&vault_path, &scan_config).map_err(|e| e.to_string())?;
        debug!(
            notes = index.ordered_slugs.len(),
            elapsed_ms = scan_started.elapsed().as_secs_f64() * 1_000.0,
            "Vault scan performance"
        );
        sqlite.replace_from_index_with_embedder(&index, embedder.as_ref())
    })
    .await?;

    debug!(vault_path = %state.vault_path.display(), "SQLite vault cache refreshed");

    let current_marker_hash = state
        .cache
        .read()
        .await
        .sqlite
        .get_metadata("marker_set_hash")
        .ok()
        .flatten();
    if previous_marker_hash != current_marker_hash {
        info!("Layer marker set changed; MCP clients should re-list tools");
        // No live receiver over the current stateless HTTP transport; a future
        // streaming transport subscribes and emits notifications/tools/list_changed.
        let _ = state.mcp_tools_changed.send(());
    }

    // A successful reindex after a failed startup — e.g. the watcher picking up a
    // corrected `.hatchdoor-layer` marker — clears the failed state and brings
    // the vault routes back online. When run_reindex is reached, startup is
    // either Ready (normal writes/refresh) or Failed (recovery); the read routes
    // are gated behind readiness, so a refresh can only arrive in those two
    // states. Git sync, if configured, still requires a restart after a failed
    // startup: it is started only on the clean-startup path.
    if !state.startup.is_ready() {
        info!("Vault reindex succeeded; clearing failed startup state");
        state.startup.set_ready();
    }

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
    fn build_cache_honours_user_exclude_pattern_on_the_real_build_path() {
        use crate::vault::ExcludeMatcher;
        let dir = tempdir().expect("temp dir");
        let vault_path = dir.path().join("vault");
        std::fs::create_dir_all(vault_path.join("build")).expect("build dir");
        std::fs::write(vault_path.join("Home.md"), "# Home\n").expect("write home");
        std::fs::write(vault_path.join("build/Generated.md"), "# Generated\n")
            .expect("write generated");

        let embedder = test_embedder();
        let sqlite = Arc::new(SqliteCache::in_memory(384).expect("cache"));
        let scan_config = VaultScanConfig {
            exclude: ExcludeMatcher::new(&["build/".to_string()]).expect("matcher"),
        };
        let cache = build_cache_with_sqlite_and_progress(
            &vault_path,
            sqlite,
            embedder.as_ref(),
            None,
            &scan_config,
        )
        .expect("build cache");

        assert!(
            cache
                .sqlite
                .read_note_by_slug("home")
                .expect("read")
                .is_some(),
            "the default note must be indexed"
        );
        assert!(
            cache
                .sqlite
                .read_note_by_slug("generated")
                .expect("read")
                .is_none(),
            "a note under a HATCHDOOR_EXCLUDE pattern must be excluded on the real build path"
        );
    }

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
        let state_root = vault_path.parent().expect("vault parent").to_path_buf();
        let cache = build_cache(&vault_path, embedder.as_ref()).expect("build cache");
        let (vault_events, _) = broadcast::channel(64);
        let (mcp_tools_changed, _) = broadcast::channel(16);
        AppState {
            vault_path,
            cache_db_path: state_root.join("cache.sqlite3"),
            cache: Arc::new(RwLock::new(cache)),
            vault_revision: Arc::new(AtomicU64::new(0)),
            vault_events,
            mcp_tools_changed,
            embedder,
            runtime_embedder: Arc::new(crate::embed::RuntimeEmbedder::new()),
            model_setup: Arc::new(crate::model_setup::ModelSetup::new(
                state_root.join("models"),
            )),
            model_setup_started: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            startup_git_config: Arc::new(None),
            web_auth_enabled: false,
            demo_mode: false,
            vault_write_lock: Arc::new(tokio::sync::Mutex::new(())),
            git_sync: Arc::new(OnceLock::new()),
            mcp_config: Arc::new(crate::mcp::McpConfig::disabled()),
            archive_prefix: Arc::from("90-archive/"),
            scan_config: Arc::new(VaultScanConfig::default()),
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
    async fn reindex_signals_mcp_tools_changed_only_when_the_marker_set_changes() {
        let dir = tempdir().expect("temp dir");
        let vault_path = dir.path().join("vault");
        std::fs::create_dir_all(vault_path.join("sources")).expect("sources dir");
        std::fs::write(vault_path.join("Home.md"), "home").expect("write note");
        let state = state_with_vault(vault_path.clone());
        let mut tools_changed = state.mcp_tools_changed.subscribe();

        // An ordinary content reindex (no marker change) must NOT signal a
        // tool-list change: the `layers` enum is unaffected.
        std::fs::write(vault_path.join("Second.md"), "second").expect("write note");
        refresh_now(&state).await.expect("refresh");
        assert!(
            tools_changed.try_recv().is_err(),
            "a content-only reindex must not signal a tools/list change"
        );

        // Adding a layer marker changes the vault's layers, so the tool list's
        // `layers` enum is now different and the signal must fire.
        std::fs::write(vault_path.join("sources/.hatchdoor-layer"), "sources").expect("marker");
        std::fs::write(vault_path.join("sources/Clip.md"), "clip").expect("write note");
        refresh_now(&state).await.expect("refresh");
        assert!(
            tools_changed.try_recv().is_ok(),
            "adding a layer marker must signal a tools/list change"
        );
    }

    #[tokio::test]
    async fn a_successful_reindex_clears_a_failed_startup_state() {
        // Mirrors E3's recovery path: a startup that failed (e.g. a malformed
        // .hatchdoor-layer marker) leaves the tracker Failed; the watcher's next
        // successful reindex must bring the server back to Ready.
        let dir = tempdir().expect("temp dir");
        let vault_path = dir.path().join("vault");
        std::fs::create_dir_all(&vault_path).expect("create vault");
        std::fs::write(vault_path.join("Home.md"), "home").expect("write note");
        let state = state_with_vault(vault_path);
        state.startup.set_failed();
        assert!(!state.startup.is_ready(), "precondition: startup is failed");

        refresh_now(&state).await.expect("recovery refresh");

        assert!(
            state.startup.is_ready(),
            "a successful reindex must clear the failed startup state"
        );
    }

    #[tokio::test]
    async fn a_failed_reindex_leaves_startup_failed() {
        // The inverse: while the vault is still broken (here, a missing vault
        // path standing in for an un-buildable index), the reindex errors and the
        // failed state must persist rather than flip to Ready.
        let dir = tempdir().expect("temp dir");
        let vault_path = dir.path().join("vault");
        std::fs::create_dir_all(&vault_path).expect("create vault");
        std::fs::write(vault_path.join("Home.md"), "home").expect("write note");
        let mut state = state_with_vault(vault_path);
        state.startup.set_failed();
        state.vault_path = dir.path().join("missing-vault");

        let result = refresh_now(&state).await;

        assert!(result.is_err(), "reindex over a missing vault must error");
        assert!(
            !state.startup.is_ready(),
            "a failed reindex must not clear the failed startup state"
        );
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
