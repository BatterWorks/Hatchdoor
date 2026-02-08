mod vault;

use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::{Json, Router};
use dotenvy::dotenv;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tower_http::services::{ServeDir, ServeFile};

use crate::vault::{ExplorerFolder, Note, VaultIndex};

#[derive(Debug, Clone)]
struct AppConfig {
    vault_path: PathBuf,
    host: String,
    port: u16,
    refresh_seconds: u64,
}

impl AppConfig {
    fn from_env() -> Result<Self, String> {
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

    fn socket_addr(&self) -> Result<SocketAddr, String> {
        format!("{}:{}", self.host, self.port)
            .parse::<SocketAddr>()
            .map_err(|e| format!("invalid bind address: {e}"))
    }
}

fn parse_port(input: &str) -> Result<u16, String> {
    input
        .parse::<u16>()
        .map_err(|e| format!("invalid PORT '{input}': {e}"))
}

fn parse_refresh_seconds(input: &str) -> Result<u64, String> {
    input
        .parse::<u64>()
        .map_err(|e| format!("invalid VAULT_REFRESH_SECONDS '{input}': {e}"))
}

#[derive(Clone)]
struct AppState {
    vault_path: PathBuf,
    refresh_interval: Duration,
    cache: Arc<RwLock<VaultCache>>,
}

struct VaultCache {
    index: Arc<VaultIndex>,
    explorer_tree: Arc<ExplorerFolder>,
    last_refresh: Instant,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

#[derive(Debug, Serialize)]
struct NoteResponse {
    note: Note,
}

#[derive(Debug, Deserialize)]
struct ResolveQuery {
    target: String,
}

#[derive(Debug, Serialize)]
struct ResolveResponse {
    slug: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ResolveBatchRequest {
    targets: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ResolveBatchResponse {
    results: Vec<ResolveTargetResult>,
}

#[derive(Debug, Serialize)]
struct ResolveTargetResult {
    target: String,
    slug: Option<String>,
}

#[derive(Debug, Serialize)]
struct RefreshResponse {
    refreshed: bool,
}

#[tokio::main]
async fn main() {
    dotenv().ok();

    let config = AppConfig::from_env().unwrap_or_else(|e| {
        eprintln!("Configuration error: {e}");
        std::process::exit(1);
    });

    let cache = build_cache(&config.vault_path).unwrap_or_else(|e| {
        eprintln!(
            "Failed to index vault at {}: {e}",
            config.vault_path.display()
        );
        std::process::exit(1);
    });

    let state = AppState {
        vault_path: config.vault_path.clone(),
        refresh_interval: Duration::from_secs(config.refresh_seconds),
        cache: Arc::new(RwLock::new(cache)),
    };

    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/api/tree", get(tree_handler))
        .route("/api/note/{slug}", get(note_handler))
        .route("/api/resolve", get(resolve_handler))
        .route("/api/resolve-batch", post(resolve_batch_handler))
        .route("/api/refresh", post(refresh_handler))
        .route("/", get(spa_index_handler))
        .route("/n/{slug}", get(spa_index_handler))
        .route_service(
            "/manifest.webmanifest",
            ServeFile::new("frontend/dist/manifest.webmanifest"),
        )
        .route_service(
            "/registerSW.js",
            ServeFile::new("frontend/dist/registerSW.js"),
        )
        .route_service("/sw.js", ServeFile::new("frontend/dist/sw.js"))
        .nest_service("/assets", ServeDir::new("frontend/dist/assets"))
        .fallback_service(ServeDir::new("frontend/dist"))
        .with_state(state);

    let addr = config.socket_addr().unwrap_or_else(|e| {
        eprintln!("Address error: {e}");
        std::process::exit(1);
    });

    println!("Hatchdoor listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|e| {
            eprintln!("Failed to bind: {e}");
            std::process::exit(1);
        });

    axum::serve(listener, app).await.unwrap_or_else(|e| {
        eprintln!("Server error: {e}");
        std::process::exit(1);
    });
}

fn build_cache(vault_path: &PathBuf) -> Result<VaultCache, String> {
    let index = VaultIndex::build(vault_path).map_err(|e| e.to_string())?;
    let explorer_tree = index.explorer_tree();

    Ok(VaultCache {
        index: Arc::new(index),
        explorer_tree: Arc::new(explorer_tree),
        last_refresh: Instant::now(),
    })
}

async fn snapshot(
    state: &AppState,
) -> Result<(Arc<VaultIndex>, Arc<ExplorerFolder>), (StatusCode, Json<ErrorResponse>)> {
    refresh_if_needed(state, false).await?;
    let guard = state.cache.read().await;
    Ok((guard.index.clone(), guard.explorer_tree.clone()))
}

async fn refresh_if_needed(
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
            *guard = cache;
            Ok(())
        }
        Err(error) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Vault refresh failed: {error}"),
            }),
        )),
    }
}

async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

async fn tree_handler(State(state): State<AppState>) -> impl IntoResponse {
    match snapshot(&state).await {
        Ok((_index, tree)) => (StatusCode::OK, Json((*tree).clone())).into_response(),
        Err(err) => err.into_response(),
    }
}

async fn note_handler(
    Path(slug): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let (index, _tree) = match snapshot(&state).await {
        Ok(s) => s,
        Err(err) => return err.into_response(),
    };

    match index.read_note_by_slug(&slug) {
        Ok(Some(note)) => (StatusCode::OK, Json(NoteResponse { note })).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Note not found: {slug}"),
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed reading note {slug}: {e}"),
            }),
        )
            .into_response(),
    }
}

async fn resolve_handler(
    Query(query): Query<ResolveQuery>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let (index, _tree) = match snapshot(&state).await {
        Ok(s) => s,
        Err(err) => return err.into_response(),
    };

    let slug = index
        .resolve_wikilink(&query.target)
        .map(|entry| entry.slug.clone());

    (StatusCode::OK, Json(ResolveResponse { slug })).into_response()
}

async fn resolve_batch_handler(
    State(state): State<AppState>,
    Json(payload): Json<ResolveBatchRequest>,
) -> impl IntoResponse {
    let (index, _tree) = match snapshot(&state).await {
        Ok(s) => s,
        Err(err) => return err.into_response(),
    };

    let results = payload
        .targets
        .into_iter()
        .map(|target| ResolveTargetResult {
            slug: index
                .resolve_wikilink(&target)
                .map(|entry| entry.slug.clone()),
            target,
        })
        .collect();

    (StatusCode::OK, Json(ResolveBatchResponse { results })).into_response()
}

async fn refresh_handler(State(state): State<AppState>) -> impl IntoResponse {
    match refresh_if_needed(&state, true).await {
        Ok(()) => (StatusCode::OK, Json(RefreshResponse { refreshed: true })).into_response(),
        Err(err) => err.into_response(),
    }
}

async fn spa_index_handler() -> impl IntoResponse {
    match std::fs::read_to_string("frontend/dist/index.html") {
        Ok(html) => (StatusCode::OK, Html(html)).into_response(),
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Html(
                "<h1>Frontend not built</h1><p>Run <code>cd frontend && npm install && npm run build</code>, then restart the server.</p>"
                    .to_string(),
            ),
        )
            .into_response(),
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
