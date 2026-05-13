use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use axum::Json;
use axum::http::StatusCode;
use tokio::sync::{RwLock, broadcast};
use tracing::{debug, error, info};
use tracing_subscriber::EnvFilter;

use crate::api_types::ErrorResponse;
use crate::cache::SqliteCache;
use crate::vault::VaultIndex;

#[derive(Debug, Clone)]
pub(crate) struct AppConfig {
    pub(crate) vault_path: PathBuf,
    pub(crate) cache_db_path: PathBuf,
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) refresh_seconds: u64,
}

impl AppConfig {
    pub(crate) fn from_env() -> Result<Self, String> {
        let vault_path = env::var("VAULT_PATH").unwrap_or_else(|_| "./vault".to_string());
        let cache_db_path = env::var("HATCHDOOR_CACHE_DB")
            .unwrap_or_else(|_| "./data/cache/hatchdoor-cache.sqlite3".to_string());
        let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let port_raw = env::var("PORT").unwrap_or_else(|_| "42824".to_string());
        let refresh_raw = env::var("VAULT_REFRESH_SECONDS").unwrap_or_else(|_| "2".to_string());

        let port = parse_port(&port_raw)?;
        let refresh_seconds = parse_refresh_seconds(&refresh_raw)?;

        Ok(Self {
            vault_path: PathBuf::from(vault_path),
            cache_db_path: PathBuf::from(cache_db_path),
            host,
            port,
            refresh_seconds,
        })
    }

    pub(crate) fn socket_addr(&self) -> Result<SocketAddr, String> {
        format!("{}:{}", self.host, self.port)
            .parse::<SocketAddr>()
            .map_err(|e| format!("invalid bind address: {e}"))
    }
}

pub(crate) fn parse_port(input: &str) -> Result<u16, String> {
    input
        .parse::<u16>()
        .map_err(|e| format!("invalid PORT '{input}': {e}"))
}

pub(crate) fn parse_refresh_seconds(input: &str) -> Result<u64, String> {
    input
        .parse::<u64>()
        .map_err(|e| format!("invalid VAULT_REFRESH_SECONDS '{input}': {e}"))
}

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) vault_path: PathBuf,
    pub(crate) refresh_interval: Duration,
    pub(crate) cache: Arc<RwLock<VaultCache>>,
    pub(crate) vault_revision: Arc<AtomicU64>,
    pub(crate) vault_events: broadcast::Sender<u64>,
}

pub(crate) struct VaultCache {
    pub(crate) sqlite: Arc<SqliteCache>,
    pub(crate) last_refresh: Instant,
}

pub(crate) fn init_logging() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("hatchdoor=info,tower_http=info,axum::rejection=warn"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();
}

#[cfg(test)]
pub(crate) fn build_cache(vault_path: &PathBuf) -> Result<VaultCache, String> {
    let sqlite = Arc::new(SqliteCache::in_memory()?);
    build_cache_with_sqlite(vault_path, sqlite)
}

pub(crate) fn build_cache_with_sqlite(
    vault_path: &PathBuf,
    sqlite: Arc<SqliteCache>,
) -> Result<VaultCache, String> {
    debug!(vault_path = %vault_path.display(), "Building SQLite vault cache");
    let index = VaultIndex::build(vault_path).map_err(|e| e.to_string())?;
    sqlite.replace_from_index(&index)?;

    Ok(VaultCache {
        sqlite,
        last_refresh: Instant::now(),
    })
}

pub(crate) async fn sqlite_cache(
    state: &AppState,
) -> Result<Arc<SqliteCache>, (StatusCode, Json<ErrorResponse>)> {
    let guard = state.cache.read().await;
    Ok(guard.sqlite.clone())
}

pub(crate) async fn refresh_if_needed(
    state: &AppState,
    force: bool,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    {
        let guard = state.cache.read().await;
        if !force && guard.last_refresh.elapsed() < state.refresh_interval {
            return Ok(());
        }
    }

    let mut guard = state.cache.write().await;
    if !force && guard.last_refresh.elapsed() < state.refresh_interval {
        return Ok(());
    }

    match build_cache_with_sqlite(&state.vault_path, guard.sqlite.clone()) {
        Ok(cache) => {
            if force {
                info!(
                    force_refresh = true,
                    vault_path = %state.vault_path.display(),
                    "SQLite vault cache refreshed"
                );
            } else {
                debug!(
                    force_refresh = false,
                    vault_path = %state.vault_path.display(),
                    "SQLite vault cache refreshed"
                );
            }
            *guard = cache;
            broadcast_vault_revision(state);
            Ok(())
        }
        Err(error) => {
            error!(
                force_refresh = force,
                vault_path = %state.vault_path.display(),
                error = %error,
                "Vault refresh failed"
            );
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Vault refresh failed: {error}"),
                }),
            ))
        }
    }
}

fn broadcast_vault_revision(state: &AppState) {
    let revision = state.vault_revision.fetch_add(1, Ordering::SeqCst) + 1;
    let _ = state.vault_events.send(revision);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn parse_port_accepts_valid_u16() {
        assert_eq!(parse_port("42824").expect("valid port"), 42824);
    }

    #[test]
    fn parse_port_rejects_invalid_values() {
        assert!(parse_port("70000").is_err());
        assert!(parse_port("abc").is_err());
    }

    #[test]
    fn parse_refresh_seconds_accepts_valid_u64() {
        assert_eq!(parse_refresh_seconds("2").expect("valid refresh"), 2);
    }

    #[test]
    fn parse_refresh_seconds_rejects_invalid_values() {
        assert!(parse_refresh_seconds("-1").is_err());
        assert!(parse_refresh_seconds("abc").is_err());
    }

    #[test]
    fn socket_addr_builds_expected_address() {
        let cfg = AppConfig {
            vault_path: PathBuf::from("./vault"),
            cache_db_path: PathBuf::from("./data/cache/hatchdoor-cache.sqlite3"),
            host: "0.0.0.0".to_string(),
            port: 42824,
            refresh_seconds: 2,
        };

        let addr = cfg.socket_addr().expect("valid addr");
        assert_eq!(addr.to_string(), "0.0.0.0:42824");
    }

    fn state_with_vault(vault_path: PathBuf, refresh_interval: Duration) -> AppState {
        let cache = build_cache(&vault_path).expect("build cache");
        let (vault_events, _) = broadcast::channel(64);
        AppState {
            vault_path,
            refresh_interval,
            cache: Arc::new(RwLock::new(cache)),
            vault_revision: Arc::new(AtomicU64::new(0)),
            vault_events,
        }
    }

    #[tokio::test]
    async fn refresh_if_needed_skips_when_interval_not_elapsed() {
        let dir = tempdir().expect("temp dir");
        let vault_path = dir.path().join("vault");
        std::fs::create_dir_all(&vault_path).expect("create vault");
        std::fs::write(vault_path.join("Home.md"), "home").expect("write note");

        let mut state = state_with_vault(vault_path, Duration::from_secs(3600));
        state.vault_path = dir.path().join("missing-vault");

        let result = refresh_if_needed(&state, false).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn refresh_if_needed_force_refresh_surfaces_errors() {
        let dir = tempdir().expect("temp dir");
        let vault_path = dir.path().join("vault");
        std::fs::create_dir_all(&vault_path).expect("create vault");
        std::fs::write(vault_path.join("Home.md"), "home").expect("write note");

        let mut state = state_with_vault(vault_path, Duration::from_secs(3600));
        state.vault_path = dir.path().join("missing-vault");

        let result = refresh_if_needed(&state, true).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn sqlite_cache_returns_current_cache_without_reindexing() {
        let dir = tempdir().expect("temp dir");
        let vault_path = dir.path().join("vault");
        std::fs::create_dir_all(&vault_path).expect("create vault");
        std::fs::write(vault_path.join("Home.md"), "home").expect("write note");

        let mut state = state_with_vault(vault_path, Duration::from_secs(0));
        state.vault_path = dir.path().join("missing-vault");

        let result = sqlite_cache(&state).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn refresh_if_needed_broadcasts_revision_after_successful_refresh() {
        let dir = tempdir().expect("temp dir");
        let vault_path = dir.path().join("vault");
        std::fs::create_dir_all(&vault_path).expect("create vault");
        std::fs::write(vault_path.join("Home.md"), "home").expect("write note");
        let state = state_with_vault(vault_path.clone(), Duration::from_secs(3600));
        let mut events = state.vault_events.subscribe();

        std::fs::write(vault_path.join("Second.md"), "second").expect("write note");
        refresh_if_needed(&state, true).await.expect("refresh");

        assert_eq!(events.recv().await.expect("revision"), 1);
    }
}
