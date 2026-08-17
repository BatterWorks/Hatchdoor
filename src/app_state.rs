use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock as StdRwLock};
use std::time::Instant;

use axum::Json;
use axum::http::StatusCode;
use serde::Serialize;
use tokio::sync::{RwLock, broadcast};
use tracing::{debug, error, info};

use crate::api_types::ErrorResponse;
use crate::cache::SqliteCache;
use crate::embed::Embedder;
use crate::startup::{IndexingProgressSnapshot, StartupTracker};
use crate::vault::{VaultIndex, VaultScanConfig, seed_empty_vault};
use crate::vault_migration::LegacyMigrationRecovery;
use crate::vault_registry::{VaultDefinition, VaultRegistryStore};
use crate::vault_runtime::VaultCollectionRuntime;
use crate::vault_runtime::VaultSource;

#[derive(Clone)]
pub struct AppState {
    pub cache_db_path: PathBuf,
    /// Authoritative Vault definitions and the independently activated runtime
    /// collection derived from them.
    pub vault_registry: VaultRegistryStore,
    pub vaults: VaultCollectionRuntime,
    /// The instance-wide background-work admission queue and its per-Vault
    /// managed-Git scheduler. HTTP Vault-collection management (add/edit/
    /// enable/disable/disconnect) reconciles through these after a registry
    /// commit; manual sync/retry request a Git turn directly.
    pub vault_work: crate::vault_work::VaultWorkCoordinator,
    pub managed_git: Arc<crate::git::ManagedGitScheduler>,
    /// Present when safe automatic import could not prove the legacy
    /// deployment. Collection/setup surfaces remain available for recovery.
    /// Cleared by a confirmed "Start with no Vaults"
    /// (`start_with_no_vaults_handler`), so this needs interior mutability
    /// rather than a plain `Option` fixed at startup.
    pub legacy_migration_recovery: Arc<StdRwLock<Option<LegacyMigrationRecovery>>>,
    /// Disposable SQLite handle used while a configured local vault is still
    /// being indexed. It is published through `ready_vault` only after a
    /// successful build.
    pub startup_sqlite: Arc<SqliteCache>,
    /// Published only after a vault path has been validated and its first index
    /// has committed. The application can serve liveness and lifecycle routes
    /// while this is `None`.
    pub ready_vault: Arc<RwLock<Option<ReadyVault>>>,
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
    /// The one active versioning task, if any. A write lock serializes drain →
    /// replacement so two tasks can never touch the repository together.
    pub git_sync: Arc<RwLock<Option<crate::git::GitSyncHandle>>>,
    /// Cache for [`AppState::live_scan_config`], keyed by the snapshot pointer
    /// it was built from. Every live consumer of noise-exclusion configuration
    /// (the watcher, MCP/HTTP writes, reads, diagnostics) must derive its scan
    /// config from the *current* runtime snapshot rather than a boot-time copy,
    /// so a saved `HATCHDOOR_EXCLUDE` change takes effect immediately outside a
    /// reindex. Rebuilding the underlying `ExcludeMatcher` on every request
    /// would be wasteful, so this caches the last built config and only
    /// rebuilds when the bound snapshot has actually changed.
    #[allow(clippy::type_complexity)]
    pub scan_config_cache: Arc<
        StdRwLock<
            Option<(
                Arc<crate::runtime_config::ConfigSnapshot>,
                Arc<VaultScanConfig>,
            )>,
        >,
    >,
    /// Held while a reindex runs so concurrent refreshes coalesce into one.
    pub refresh_lock: Arc<tokio::sync::Mutex<()>>,
    /// Dedicated status for reindexes triggered by live indexing settings.
    /// Unlike startup readiness, this never gates a coherent existing index.
    pub index_status: IndexStatusTracker,
    /// Captured environment plus durable live-applicable settings. Consumers
    /// bind an immutable snapshot once per operation.
    pub runtime_config: crate::runtime_config::RuntimeConfig,
    pub startup: StartupTracker,
}

#[derive(Clone)]
pub struct IndexStatusTracker(Arc<StdRwLock<IndexStatus>>);

#[derive(Clone, Debug, Serialize)]
pub struct IndexStatusResponse {
    pub state: &'static str,
    /// True means the last coherent index was built with older indexing settings.
    /// `state` distinguishes *why* it is stale: `"rebuilding"` (in progress,
    /// self-clearing) vs `"failed"` (stuck until a change or restart retries
    /// it) — there is no separate drift/health axis to track, so this is the
    /// only field a client needs.
    pub stale: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes_completed: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes_total: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunks_completed: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunks_total: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_completed: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_total: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percent: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eta_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_failure: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IndexStatusPhase {
    UpToDate,
    Rebuilding,
    Failed,
}

#[derive(Debug)]
struct IndexStatus {
    phase: IndexStatusPhase,
    target_generation: u64,
    active_generation: Option<u64>,
    progress: Option<IndexingProgressSnapshot>,
    last_failure: Option<String>,
}

impl IndexStatusTracker {
    pub(crate) fn up_to_date() -> Self {
        Self(Arc::new(StdRwLock::new(IndexStatus {
            phase: IndexStatusPhase::UpToDate,
            target_generation: 0,
            active_generation: None,
            progress: None,
            last_failure: None,
        })))
    }

    /// Record a persisted reindex-setting change before the background task is spawned.
    pub(crate) fn queue_rebuild(&self) -> u64 {
        let mut status = self.0.write().expect("index status tracker poisoned");
        status.target_generation += 1;
        status.phase = IndexStatusPhase::Rebuilding;
        status.target_generation
    }

    pub(crate) fn start_rebuild(&self, generation: u64) {
        let mut status = self.0.write().expect("index status tracker poisoned");
        status.phase = IndexStatusPhase::Rebuilding;
        status.active_generation = Some(generation);
        status.progress = None;
    }

    pub(crate) fn report_progress(&self, generation: u64, progress: IndexingProgressSnapshot) {
        let mut status = self.0.write().expect("index status tracker poisoned");
        if status.active_generation == Some(generation) {
            status.progress = Some(progress);
        }
    }

    pub(crate) fn finish_rebuild(&self, generation: u64) {
        let mut status = self.0.write().expect("index status tracker poisoned");
        if status.active_generation == Some(generation) {
            status.active_generation = None;
            status.progress = None;
        }
        if status.target_generation == generation {
            status.phase = IndexStatusPhase::UpToDate;
            status.last_failure = None;
        } else {
            status.phase = IndexStatusPhase::Rebuilding;
        }
    }

    pub(crate) fn fail_rebuild(&self, generation: u64, error: impl Into<String>) {
        let mut status = self.0.write().expect("index status tracker poisoned");
        if status.active_generation == Some(generation) {
            status.active_generation = None;
            status.progress = None;
        }
        status.last_failure = Some(error.into());
        status.phase = if status.target_generation == generation {
            IndexStatusPhase::Failed
        } else {
            IndexStatusPhase::Rebuilding
        };
    }

    pub fn status(&self) -> IndexStatusResponse {
        let status = self.0.read().expect("index status tracker poisoned");
        let (state, stale) = match status.phase {
            IndexStatusPhase::UpToDate => ("up_to_date", false),
            IndexStatusPhase::Rebuilding => ("rebuilding", true),
            IndexStatusPhase::Failed => ("failed", true),
        };
        let progress = status.progress;
        IndexStatusResponse {
            state,
            stale,
            notes_completed: progress.map(|value| value.notes_completed),
            notes_total: progress.map(|value| value.notes_total),
            chunks_completed: progress.map(|value| value.chunks_completed),
            chunks_total: progress.map(|value| value.chunks_total),
            tokens_completed: progress.map(|value| value.tokens_completed),
            tokens_total: progress.map(|value| value.tokens_total),
            percent: progress.map(progress_percent),
            eta_seconds: progress.and_then(progress_eta_seconds),
            last_failure: status.last_failure.clone(),
        }
    }
}

fn progress_percent(progress: IndexingProgressSnapshot) -> u8 {
    if progress.tokens_total == 0 {
        return 0;
    }
    ((progress.tokens_completed.saturating_mul(100) / progress.tokens_total).min(100)) as u8
}

fn progress_eta_seconds(progress: IndexingProgressSnapshot) -> Option<u64> {
    if progress.tokens_completed == 0 || progress.tokens_completed >= progress.tokens_total {
        return None;
    }
    let remaining = progress.tokens_total - progress.tokens_completed;
    Some(
        progress.elapsed_seconds.saturating_mul(remaining as u64)
            / progress.tokens_completed as u64,
    )
}

impl AppState {
    /// Bind the current live configuration once at an operation boundary.
    pub fn runtime_snapshot(&self) -> Arc<crate::runtime_config::ConfigSnapshot> {
        self.runtime_config.snapshot()
    }

    pub fn runtime_mcp_config(
        snapshot: &crate::runtime_config::ConfigSnapshot,
    ) -> Result<crate::mcp::McpConfig, String> {
        crate::mcp::McpConfig::from_snapshot(snapshot)
    }

    pub fn runtime_archive_prefix(
        snapshot: &crate::runtime_config::ConfigSnapshot,
    ) -> Result<Arc<str>, String> {
        snapshot
            .setting("HATCHDOOR_ARCHIVE_PREFIX")
            .map(|setting| Arc::from(setting.value.as_str()))
            .ok_or_else(|| "runtime configuration is missing HATCHDOOR_ARCHIVE_PREFIX".to_string())
    }

    /// Resolve the archive folder for one Vault: its own configured folder
    /// (`VaultDefinition::archive_folder`) if set, else the instance-wide
    /// `HATCHDOOR_ARCHIVE_PREFIX` default. `definition` is `None` when the
    /// Vault has no reconciled runtime yet — that degrades to the
    /// instance-wide default rather than failing, since the caller's own
    /// Vault-existence check (a not-found/unavailable status) is the
    /// authoritative error for that case.
    pub fn vault_archive_prefix(
        definition: Option<&VaultDefinition>,
        snapshot: &crate::runtime_config::ConfigSnapshot,
    ) -> Result<Arc<str>, String> {
        if let Some(folder) = definition.and_then(VaultDefinition::archive_folder) {
            return Ok(Arc::from(folder));
        }
        Self::runtime_archive_prefix(snapshot)
    }

    pub fn runtime_scan_config(
        snapshot: &crate::runtime_config::ConfigSnapshot,
    ) -> Result<Arc<VaultScanConfig>, String> {
        let patterns = snapshot
            .setting("HATCHDOOR_EXCLUDE")
            .map(|setting| crate::config::parse_exclude_patterns(&setting.value))
            .ok_or_else(|| "runtime configuration is missing HATCHDOOR_EXCLUDE".to_string())?;
        Ok(Arc::new(VaultScanConfig {
            exclude: crate::vault::ExcludeMatcher::new(&patterns)?,
        }))
    }

    /// Derive every configuration value consumed by one cache build from the
    /// same immutable snapshot. A settings save may publish a new snapshot at
    /// any time, so a reindex must not call a live accessor after binding this
    /// operation's generation.
    pub(crate) fn runtime_index_options(
        snapshot: &crate::runtime_config::ConfigSnapshot,
    ) -> Result<(Arc<VaultScanConfig>, bool), String> {
        let scan_config = Self::runtime_scan_config(snapshot)?;
        let embed_layers =
            crate::runtime_config::is_truthy(snapshot.required("HATCHDOOR_EMBED_LAYERS")?);
        Ok((scan_config, embed_layers))
    }

    /// Derive the noise-exclusion scan config from the *current* runtime
    /// snapshot. Every live consumer (the watcher, MCP/HTTP write refusals,
    /// reads, diagnostics) must call this instead of caching a boot-time copy,
    /// so a saved `HATCHDOOR_EXCLUDE` change is honored immediately. The
    /// underlying `ExcludeMatcher` is only rebuilt when the bound snapshot
    /// actually changed since the last call.
    pub fn live_scan_config(&self) -> Result<Arc<VaultScanConfig>, String> {
        let snapshot = self.runtime_snapshot();
        {
            let cache = self
                .scan_config_cache
                .read()
                .expect("scan config cache poisoned");
            if let Some((cached_snapshot, cached_config)) = cache.as_ref()
                && Arc::ptr_eq(cached_snapshot, &snapshot)
            {
                return Ok(cached_config.clone());
            }
        }
        let config = Self::runtime_scan_config(&snapshot)?;
        let mut cache = self
            .scan_config_cache
            .write()
            .expect("scan config cache poisoned");
        *cache = Some((snapshot, config.clone()));
        Ok(config)
    }

    /// Record a vault write for git sync. No-op when sync is disabled.
    ///
    /// Awaits the read lock directly rather than spawning a detached task: a
    /// spawned task races a concurrent stop-drain with no ordering guarantee
    /// (a record enqueued "before" a stop could be applied "after" it from the
    /// scheduler's point of view), and needlessly required a Tokio runtime for
    /// what used to be a synchronous call. Awaiting here keeps every write's
    /// enqueue ordered with respect to the git lifecycle that guards the same
    /// lock.
    pub async fn record_vault_write(&self, record: crate::git::WriteRecord) {
        if let Some(handle) = self.git_sync.read().await.as_ref() {
            handle.record(record);
        }
    }

    pub async fn ready_vault(&self) -> Result<ReadyVault, (StatusCode, Json<ErrorResponse>)> {
        self.ready_vault
            .read()
            .await
            .clone()
            .ok_or_else(vault_unavailable)
    }

    pub async fn publish_ready_vault(&self, vault_path: PathBuf, cache: VaultCache) {
        *self.ready_vault.write().await = Some(ReadyVault { vault_path, cache });
    }

    pub async fn vault_path(&self) -> Option<PathBuf> {
        self.ready_vault
            .read()
            .await
            .as_ref()
            .map(|ready| ready.vault_path.clone())
    }

    pub fn configured_local_vault_path(&self) -> Option<PathBuf> {
        match self.startup.runtime().source() {
            VaultSource::Local { vault_path } => Some(vault_path.clone()),
        }
    }
}

#[derive(Clone)]
pub struct ReadyVault {
    pub vault_path: PathBuf,
    pub cache: VaultCache,
}

#[derive(Clone)]
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
        true,
    )
}

pub fn build_cache_with_sqlite_and_progress(
    vault_path: &PathBuf,
    sqlite: Arc<SqliteCache>,
    embedder: &dyn Embedder,
    on_progress: Option<Arc<dyn Fn(IndexingProgressSnapshot) + Send + Sync>>,
    scan_config: &VaultScanConfig,
    embed_layers: bool,
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
    sqlite.replace_from_index_with_progress(&index, embedder, on_progress, embed_layers)?;

    Ok(VaultCache { sqlite })
}

pub async fn sqlite_cache(
    state: &AppState,
) -> Result<Arc<SqliteCache>, (StatusCode, Json<ErrorResponse>)> {
    Ok(state.ready_vault().await?.cache.sqlite)
}

pub fn vault_unavailable() -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(ErrorResponse {
            error: "Vault is not ready".to_string(),
        }),
    )
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

/// Guaranteed refresh for paths that must see their own change reflected (MCP
/// writes, the vault watcher): waits for any in-flight reindex, then reindexes.
pub async fn refresh_now(state: &AppState) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let _refresh_guard = state.refresh_lock.lock().await;
    run_reindex(state, None).await.map_err(internal_error)
}

/// Start the versioning task selected at startup if it is not already active.
/// A watcher recovery calls this before it releases readiness, so recovering a
/// broken initial index cannot leave the HTTP/MCP process ready without its
/// configured background capability.
pub(crate) async fn ensure_startup_git_sync(state: &AppState) {
    let Some(config) = state.startup_git_config.as_ref().clone() else {
        return;
    };
    let mut active_git_sync = state.git_sync.write().await;
    if active_git_sync.is_some() {
        return;
    }
    let handle = crate::git::spawn_sync_task(
        config,
        state.vault_write_lock.clone(),
        crate::git::SyncOps {
            commit: Box::new(crate::git::commit_local),
            fetch: Box::new(crate::git::fetch_remote),
            integrate: Box::new(crate::git::integrate_fetched),
            push: Box::new(crate::git::push_branch),
        },
    );
    *active_git_sync = Some(handle);
    info!("Git sync enabled");
}

/// Spawn an index-settings rebuild after its new settings have been persisted.
/// Each request receives a generation; queued requests run serially and the
/// final completed generation always reflects the latest stored configuration.
pub fn schedule_settings_reindex(state: AppState) {
    let generation = state.index_status.queue_rebuild();
    tokio::spawn(async move {
        let _refresh_guard = state.refresh_lock.lock().await;
        state.index_status.start_rebuild(generation);
        let progress_tracker = state.index_status.clone();
        let on_progress = Arc::new(move |progress| {
            progress_tracker.report_progress(generation, progress);
        });
        match run_reindex(&state, Some(on_progress)).await {
            Ok(()) => state.index_status.finish_rebuild(generation),
            Err(error) => {
                error!(generation, %error, "Settings-triggered index rebuild failed");
                state.index_status.fail_rebuild(
                    generation,
                    "The index rebuild failed. Check the Hatchdoor logs for details.",
                );
            }
        }
    });
}

async fn run_reindex(
    state: &AppState,
    on_progress: Option<Arc<dyn Fn(IndexingProgressSnapshot) + Send + Sync>>,
) -> Result<(), String> {
    let ready_vault = state.ready_vault.read().await.clone();
    let was_ready = ready_vault.is_some();
    let (sqlite, vault_path) = match ready_vault {
        Some(ready) => (ready.cache.sqlite, ready.vault_path),
        None => (
            state.startup_sqlite.clone(),
            state
                .configured_local_vault_path()
                .ok_or_else(|| "Vault is not ready".to_string())?,
        ),
    };
    let current_sqlite = sqlite.clone();
    let logged_vault_path = vault_path.clone();
    let embedder = state.embedder.clone();
    let runtime_snapshot = state.runtime_snapshot();
    let (scan_config, embed_layers) = AppState::runtime_index_options(&runtime_snapshot)?;

    // The marker-set hash the last build persisted. Compared against the value
    // after this reindex to detect a runtime layer change (a marker added,
    // removed, renamed, or its description edited), which changes the MCP
    // `tools/list` `layers` enum.
    let previous_marker_hash = sqlite.get_metadata("marker_set_hash").ok().flatten();

    // The reindex writes inside a single SQLite transaction; WAL lets readers on
    // pooled connections keep serving the prior snapshot until it commits, so we
    // no longer hold the cache write lock for the whole rebuild (F-03).
    tokio::task::spawn_blocking(move || {
        info!("Scanning vault for notes…");
        let scan_started = Instant::now();
        let index =
            VaultIndex::build_with_config(&vault_path, &scan_config).map_err(|e| e.to_string())?;
        debug!(
            notes = index.ordered_slugs.len(),
            elapsed_ms = scan_started.elapsed().as_secs_f64() * 1_000.0,
            "Vault scan performance"
        );
        sqlite.replace_from_index_with_progress(
            &index,
            embedder.as_ref(),
            on_progress,
            embed_layers,
        )
    })
    .await
    .map_err(|error| format!("background reindex task panicked: {error}"))??;

    if !was_ready {
        state
            .publish_ready_vault(
                logged_vault_path.clone(),
                VaultCache {
                    sqlite: current_sqlite.clone(),
                },
            )
            .await;
    }

    debug!(vault_path = %logged_vault_path.display(), "SQLite vault cache refreshed");

    let current_marker_hash = current_sqlite
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
    // corrected `.hatchdoor-layer` marker — restores every configured background
    // capability before it brings the vault routes back online. When run_reindex
    // is reached, startup is either Ready (normal writes/refresh) or Failed
    // (recovery); the read routes are gated behind readiness, so a refresh can
    // only arrive in those two states.
    if !state.startup.is_ready() {
        ensure_startup_git_sync(state).await;
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
    use std::path::Path;
    use tempfile::tempdir;

    use crate::git::{GitConfig, GitMode};

    #[test]
    fn index_status_keeps_search_stale_until_the_latest_queued_rebuild_finishes() {
        let status = IndexStatusTracker::up_to_date();
        let first = status.queue_rebuild();
        status.start_rebuild(first);
        let second = status.queue_rebuild();

        status.finish_rebuild(first);
        let during_second = status.status();
        assert_eq!(during_second.state, "rebuilding");
        assert!(during_second.stale);

        status.start_rebuild(second);
        status.finish_rebuild(second);
        let current = status.status();
        assert_eq!(current.state, "up_to_date");
        assert!(!current.stale);
    }

    #[test]
    fn index_status_exposes_progress_eta_and_the_last_background_failure() {
        let status = IndexStatusTracker::up_to_date();
        let generation = status.queue_rebuild();
        status.start_rebuild(generation);
        status.report_progress(
            generation,
            IndexingProgressSnapshot {
                notes_completed: 12,
                notes_total: 40,
                chunks_completed: 18,
                chunks_total: 70,
                tokens_completed: 4_000,
                tokens_total: 20_000,
                elapsed_seconds: 20,
            },
        );

        let rebuilding = status.status();
        assert_eq!(rebuilding.state, "rebuilding");
        assert_eq!(rebuilding.percent, Some(20));
        assert_eq!(rebuilding.eta_seconds, Some(80));
        assert!(rebuilding.last_failure.is_none());

        status.fail_rebuild(generation, "embedding failed");
        let failed = status.status();
        assert_eq!(failed.state, "failed");
        assert!(failed.stale);
        assert_eq!(failed.last_failure.as_deref(), Some("embedding failed"));
    }

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
            true,
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
        let (vault_work, _vault_worker) = crate::vault_work::VaultWorkCoordinator::new();
        let managed_git = Arc::new(crate::git::ManagedGitScheduler::new(vault_work.clone()));
        AppState {
            cache_db_path: state_root.join("cache.sqlite3"),
            vault_registry: VaultRegistryStore::new(state_root.join("state/vaults.json")),
            vaults: VaultCollectionRuntime::new(),
            vault_work,
            managed_git,
            legacy_migration_recovery: Arc::new(StdRwLock::new(None)),
            startup_sqlite: cache.sqlite.clone(),
            ready_vault: Arc::new(RwLock::new(Some(ReadyVault { vault_path, cache }))),
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
            git_sync: Arc::new(RwLock::new(None)),
            scan_config_cache: Arc::new(StdRwLock::new(None)),
            refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
            index_status: IndexStatusTracker::up_to_date(),
            runtime_config: crate::runtime_config::RuntimeConfig::for_tests(),
            startup: StartupTracker::ready(),
        }
    }

    #[tokio::test]
    async fn sqlite_cache_returns_current_cache_without_reindexing() {
        let dir = tempdir().expect("temp dir");
        let vault_path = dir.path().join("vault");
        std::fs::create_dir_all(&vault_path).expect("create vault");
        std::fs::write(vault_path.join("Home.md"), "home").expect("write note");

        let state = state_with_vault(vault_path);
        state.ready_vault.write().await.as_mut().unwrap().vault_path =
            dir.path().join("missing-vault");

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
    async fn reindex_binds_the_current_exclude_snapshot() {
        let dir = tempdir().expect("temp dir");
        let vault_path = dir.path().join("vault");
        std::fs::create_dir_all(&vault_path).expect("create vault");
        std::fs::write(vault_path.join("Home.md"), "home").expect("write home");
        std::fs::write(vault_path.join("Ignored.md"), "ignored").expect("write ignored");
        let state = state_with_vault(vault_path);

        state
            .runtime_config
            .save([("HATCHDOOR_EXCLUDE".to_string(), "Ignored.md".to_string())])
            .expect("save exclude setting");
        refresh_now(&state).await.expect("refresh");

        assert!(
            sqlite_cache(&state)
                .await
                .expect("ready cache")
                .read_note_by_slug("ignored")
                .expect("read")
                .is_none(),
            "the reindex must bind the saved exclusion snapshot"
        );
    }

    #[test]
    fn reindex_options_remain_on_one_bound_snapshot_after_settings_publish() {
        let dir = tempdir().expect("temp dir");
        let vault_path = dir.path().join("vault");
        std::fs::create_dir_all(&vault_path).expect("create vault");
        let state = state_with_vault(vault_path);

        let bound_snapshot = state.runtime_snapshot();
        state
            .runtime_config
            .save([
                ("HATCHDOOR_EXCLUDE".to_string(), "Ignored.md".to_string()),
                ("HATCHDOOR_EMBED_LAYERS".to_string(), "false".to_string()),
            ])
            .expect("publish newer settings between operation reads");

        let (bound_scan, bound_embed_layers) =
            AppState::runtime_index_options(&bound_snapshot).expect("bound index options");
        assert!(
            !bound_scan
                .exclude
                .is_excluded(Path::new("Ignored.md"), false),
            "the scan config must remain on the operation's original generation"
        );
        assert!(
            bound_embed_layers,
            "all index options must remain on the operation's original generation"
        );

        let current_snapshot = state.runtime_snapshot();
        let (current_scan, current_embed_layers) =
            AppState::runtime_index_options(&current_snapshot).expect("current index options");
        assert!(
            current_scan
                .exclude
                .is_excluded(Path::new("Ignored.md"), false)
        );
        assert!(!current_embed_layers);
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
        let mut state = state_with_vault(vault_path.clone());
        state.startup_git_config = Arc::new(Some(GitConfig {
            vault_path,
            mode: GitMode::Local,
            remote: "origin".to_string(),
            branch: "main".to_string(),
            username: "hatchdoor".to_string(),
            token: String::new(),
            debounce_seconds: 60,
            author_name: "Hatchdoor".to_string(),
            author_email: "hatchdoor@localhost".to_string(),
        }));
        state.startup.set_failed();
        assert!(!state.startup.is_ready(), "precondition: startup is failed");

        refresh_now(&state).await.expect("recovery refresh");

        assert!(
            state.startup.is_ready(),
            "a successful reindex must clear the failed startup state"
        );
        let first_handle = state
            .git_sync
            .read()
            .await
            .clone()
            .expect("recovery must restore configured Git before Ready");

        refresh_now(&state)
            .await
            .expect("subsequent refresh after recovery");
        let second_handle = state
            .git_sync
            .read()
            .await
            .clone()
            .expect("configured Git remains active");
        assert!(Arc::ptr_eq(&first_handle.status(), &second_handle.status()));
        first_handle
            .stop(std::time::Duration::from_secs(1))
            .await
            .expect("stop test Git task");
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
        let state = state_with_vault(vault_path);
        state.startup.set_failed();
        state.ready_vault.write().await.as_mut().unwrap().vault_path =
            dir.path().join("missing-vault");

        let result = refresh_now(&state).await;

        assert!(result.is_err(), "reindex over a missing vault must error");
        assert!(
            !state.startup.is_ready(),
            "a failed reindex must not clear the failed startup state"
        );
    }

    #[tokio::test]
    async fn refresh_broadcasts_revision_after_successful_reindex() {
        let dir = tempdir().expect("temp dir");
        let vault_path = dir.path().join("vault");
        std::fs::create_dir_all(&vault_path).expect("create vault");
        std::fs::write(vault_path.join("Home.md"), "home").expect("write note");
        let state = state_with_vault(vault_path.clone());
        let mut events = state.vault_events.subscribe();

        std::fs::write(vault_path.join("Second.md"), "second").expect("write note");
        refresh_now(&state).await.expect("refresh");

        assert_eq!(events.recv().await.expect("revision"), 1);
    }

    #[test]
    fn live_scan_config_reflects_a_saved_exclude_change_without_a_reindex() {
        // S1 regression: a saved HATCHDOOR_EXCLUDE must reach every live
        // consumer (watcher, write refusals, reads, diagnostics) through
        // `live_scan_config`, not only through the boot-time snapshot the
        // reindex binds.
        let dir = tempdir().expect("temp dir");
        let vault_path = dir.path().join("vault");
        std::fs::create_dir_all(&vault_path).expect("create vault");
        let state = state_with_vault(vault_path);

        let before = state.live_scan_config().expect("scan config");
        assert!(!before.exclude.is_excluded(Path::new("Secret.md"), false));

        state
            .runtime_config
            .save([("HATCHDOOR_EXCLUDE".to_string(), "Secret.md".to_string())])
            .expect("save exclude setting");

        let after = state
            .live_scan_config()
            .expect("scan config reflects the save");
        assert!(
            after.exclude.is_excluded(Path::new("Secret.md"), false),
            "the saved HATCHDOOR_EXCLUDE pattern must take effect immediately"
        );
    }

    /// Issue #130: a Vault's own configured archive folder overrides the
    /// instance-wide `HATCHDOOR_ARCHIVE_PREFIX` default when present, and the
    /// default applies unchanged both when the Vault has none configured and
    /// when no Vault definition is available at all (e.g. no reconciled
    /// runtime yet).
    #[test]
    fn vault_archive_prefix_prefers_the_vaults_own_folder_and_falls_back_to_the_instance_default() {
        let directory = tempdir().expect("temp dir");
        let vault_path = directory.path().join("vault");
        std::fs::create_dir_all(&vault_path).expect("vault dir");
        let store =
            crate::vault_registry::VaultRegistryStore::new(directory.path().join("vaults.json"));
        let committed = store
            .add(
                0,
                crate::vault_registry::NewVaultDefinition {
                    name: "Team Vault".to_string(),
                    enabled: true,
                    source: crate::vault_registry::VaultSource::Local {
                        path: vault_path.clone(),
                    },
                    exclude_patterns: Vec::new(),
                    https_credentials: None,
                    archive_folder: Some("Team Archive".to_string()),
                    commit_identity: None,
                },
            )
            .expect("add Vault with its own archive folder");
        let with_override = committed.definitions().next().expect("definition");

        std::fs::create_dir_all(directory.path().join("plain")).expect("plain vault dir");
        let plain_committed = store
            .add(
                committed.revision(),
                crate::vault_registry::NewVaultDefinition {
                    name: "Plain Vault".to_string(),
                    enabled: true,
                    source: crate::vault_registry::VaultSource::Local {
                        path: directory.path().join("plain"),
                    },
                    exclude_patterns: Vec::new(),
                    https_credentials: None,
                    archive_folder: None,
                    commit_identity: None,
                },
            )
            .expect("add Vault without an archive folder");
        let without_override = plain_committed
            .definitions()
            .find(|definition| definition.name() == "Plain Vault")
            .expect("plain definition");

        let snapshot = crate::runtime_config::RuntimeConfig::for_tests().snapshot();

        assert_eq!(
            AppState::vault_archive_prefix(Some(&with_override), &snapshot)
                .expect("resolve override")
                .as_ref(),
            "Team Archive/"
        );
        assert_eq!(
            AppState::vault_archive_prefix(Some(&without_override), &snapshot)
                .expect("resolve default")
                .as_ref(),
            "90-archive/"
        );
        assert_eq!(
            AppState::vault_archive_prefix(None, &snapshot)
                .expect("resolve default with no definition")
                .as_ref(),
            "90-archive/"
        );
    }
}
