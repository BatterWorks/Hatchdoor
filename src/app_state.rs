use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::http::StatusCode;
use axum::Json;
use tokio::sync::RwLock;
use tracing::{debug, error, info};
use tracing_subscriber::EnvFilter;

use crate::api_types::ErrorResponse;
use crate::vault::{ExplorerFolder, VaultIndex};

#[derive(Debug, Clone)]
pub(crate) struct AppConfig {
    pub(crate) vault_path: PathBuf,
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) refresh_seconds: u64,
}

impl AppConfig {
    pub(crate) fn from_env() -> Result<Self, String> {
        let vault_path = env::var("VAULT_PATH").unwrap_or_else(|_| "./vault".to_string());
        let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let port_raw = env::var("PORT").unwrap_or_else(|_| "42824".to_string());
        let refresh_raw = env::var("VAULT_REFRESH_SECONDS").unwrap_or_else(|_| "2".to_string());

        let port = parse_port(&port_raw)?;
        let refresh_seconds = parse_refresh_seconds(&refresh_raw)?;

        Ok(Self {
            vault_path: PathBuf::from(vault_path),
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
}

pub(crate) struct VaultCache {
    pub(crate) index: Arc<VaultIndex>,
    pub(crate) explorer_tree: Arc<ExplorerFolder>,
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

pub(crate) fn build_cache(vault_path: &PathBuf) -> Result<VaultCache, String> {
    debug!(vault_path = %vault_path.display(), "Building vault cache");
    let index = VaultIndex::build(vault_path).map_err(|e| e.to_string())?;
    let explorer_tree = index.explorer_tree();

    Ok(VaultCache {
        index: Arc::new(index),
        explorer_tree: Arc::new(explorer_tree),
        last_refresh: Instant::now(),
    })
}

pub(crate) async fn snapshot(
    state: &AppState,
) -> Result<(Arc<VaultIndex>, Arc<ExplorerFolder>), (StatusCode, Json<ErrorResponse>)> {
    refresh_if_needed(state, false).await?;
    let guard = state.cache.read().await;
    Ok((guard.index.clone(), guard.explorer_tree.clone()))
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

    match build_cache(&state.vault_path) {
        Ok(cache) => {
            if force {
                info!(
                    force_refresh = true,
                    vault_path = %state.vault_path.display(),
                    "Vault cache refreshed"
                );
            } else {
                debug!(
                    force_refresh = false,
                    vault_path = %state.vault_path.display(),
                    "Vault cache refreshed"
                );
            }
            *guard = cache;
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

#[cfg(test)]
mod tests {
    use super::*;

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
            host: "0.0.0.0".to_string(),
            port: 42824,
            refresh_seconds: 2,
        };

        let addr = cfg.socket_addr().expect("valid addr");
        assert_eq!(addr.to_string(), "0.0.0.0:42824");
    }
}
