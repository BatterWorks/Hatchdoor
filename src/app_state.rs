use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::Json;
use axum::http::StatusCode;
use tokio::sync::{RwLock, broadcast};
use tracing::{debug, error, info};
use tracing_subscriber::EnvFilter;

use crate::api_types::ErrorResponse;
use crate::cache::SqliteCache;
use crate::embed::Embedder;
use crate::vault::VaultIndex;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub vault_path: PathBuf,
    pub cache_db_path: PathBuf,
    pub host: String,
    pub port: u16,
}

impl AppConfig {
    pub fn from_env() -> Result<Self, String> {
        let vault_path = env::var("VAULT_PATH").unwrap_or_else(|_| "./vault".to_string());
        let cache_db_path = env::var("HATCHDOOR_CACHE_DB")
            .unwrap_or_else(|_| "./data/cache/hatchdoor-cache.sqlite3".to_string());
        let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let port_raw = env::var("PORT").unwrap_or_else(|_| "42824".to_string());

        let port = parse_port(&port_raw)?;

        Ok(Self {
            vault_path: PathBuf::from(vault_path),
            cache_db_path: PathBuf::from(cache_db_path),
            host,
            port,
        })
    }

    pub fn socket_addr(&self) -> Result<SocketAddr, String> {
        format!("{}:{}", self.host, self.port)
            .parse::<SocketAddr>()
            .map_err(|e| format!("invalid bind address: {e}"))
    }
}

pub fn parse_port(input: &str) -> Result<u16, String> {
    input
        .parse::<u16>()
        .map_err(|e| format!("invalid PORT '{input}': {e}"))
}

#[derive(Clone)]
pub struct AppState {
    pub vault_path: PathBuf,
    pub cache: Arc<RwLock<VaultCache>>,
    pub vault_revision: Arc<AtomicU64>,
    pub vault_events: broadcast::Sender<u64>,
    pub embedder: Arc<dyn Embedder>,
}

pub struct VaultCache {
    pub sqlite: Arc<SqliteCache>,
}

pub fn init_logging() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("hatchdoor=info,tower_http=info,axum::rejection=warn"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();
}

pub fn build_cache(
    vault_path: &PathBuf,
    embedder: &dyn Embedder,
) -> Result<VaultCache, String> {
    let sqlite = Arc::new(SqliteCache::in_memory()?);
    build_cache_with_sqlite(vault_path, sqlite, embedder)
}

pub fn build_cache_with_sqlite(
    vault_path: &PathBuf,
    sqlite: Arc<SqliteCache>,
    embedder: &dyn Embedder,
) -> Result<VaultCache, String> {
    debug!(vault_path = %vault_path.display(), "Building SQLite vault cache");
    let index = VaultIndex::build(vault_path).map_err(|e| e.to_string())?;
    sqlite.replace_from_index_with_embedder(&index, embedder)?;

    Ok(VaultCache { sqlite })
}

pub async fn sqlite_cache(
    state: &AppState,
) -> Result<Arc<SqliteCache>, (StatusCode, Json<ErrorResponse>)> {
    let guard = state.cache.read().await;
    Ok(guard.sqlite.clone())
}

pub async fn refresh_if_needed(
    state: &AppState,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let mut guard = state.cache.write().await;
    match build_cache_with_sqlite(
        &state.vault_path,
        guard.sqlite.clone(),
        state.embedder.as_ref(),
    ) {
        Ok(cache) => {
            info!(vault_path = %state.vault_path.display(), "SQLite vault cache refreshed");
            *guard = cache;
            broadcast_vault_revision(state);
            Ok(())
        }
        Err(error) => {
            error!(
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
pub fn test_embedder() -> Arc<dyn Embedder> {
    Arc::new(crate::embed::StubEmbedder::new(384))
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
    fn socket_addr_builds_expected_address() {
        let cfg = AppConfig {
            vault_path: PathBuf::from("./vault"),
            cache_db_path: PathBuf::from("./data/cache/hatchdoor-cache.sqlite3"),
            host: "0.0.0.0".to_string(),
            port: 42824,
        };

        let addr = cfg.socket_addr().expect("valid addr");
        assert_eq!(addr.to_string(), "0.0.0.0:42824");
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
        }
    }

    #[tokio::test]
    async fn refresh_if_needed_surfaces_errors() {
        let dir = tempdir().expect("temp dir");
        let vault_path = dir.path().join("vault");
        std::fs::create_dir_all(&vault_path).expect("create vault");
        std::fs::write(vault_path.join("Home.md"), "home").expect("write note");

        let mut state = state_with_vault(vault_path);
        state.vault_path = dir.path().join("missing-vault");

        let result = refresh_if_needed(&state).await;
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
    async fn refresh_if_needed_broadcasts_revision_after_successful_refresh() {
        let dir = tempdir().expect("temp dir");
        let vault_path = dir.path().join("vault");
        std::fs::create_dir_all(&vault_path).expect("create vault");
        std::fs::write(vault_path.join("Home.md"), "home").expect("write note");
        let state = state_with_vault(vault_path.clone());
        let mut events = state.vault_events.subscribe();

        std::fs::write(vault_path.join("Second.md"), "second").expect("write note");
        refresh_if_needed(&state).await.expect("refresh");

        assert_eq!(events.recv().await.expect("revision"), 1);
    }
}
