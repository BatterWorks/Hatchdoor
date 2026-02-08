mod vault;

use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use axum::{Json, Router};
use dotenvy::dotenv;
use serde::{Deserialize, Serialize};
use tower_http::services::{ServeDir, ServeFile};

use crate::vault::{ExplorerFolder, Note, VaultIndex};

#[derive(Debug, Clone)]
struct AppConfig {
    vault_path: PathBuf,
    host: String,
    port: u16,
}

impl AppConfig {
    fn from_env() -> Result<Self, String> {
        let vault_path = env::var("VAULT_PATH").unwrap_or_else(|_| "./vault".to_string());
        let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let port_raw = env::var("PORT").unwrap_or_else(|_| "42824".to_string());
        let port = parse_port(&port_raw)?;

        Ok(Self {
            vault_path: PathBuf::from(vault_path),
            host,
            port,
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

#[derive(Clone)]
struct AppState {
    index: Arc<VaultIndex>,
    explorer_tree: Arc<ExplorerFolder>,
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

#[tokio::main]
async fn main() {
    dotenv().ok();

    let config = AppConfig::from_env().unwrap_or_else(|e| {
        eprintln!("Configuration error: {e}");
        std::process::exit(1);
    });

    let index = VaultIndex::build(&config.vault_path).unwrap_or_else(|e| {
        eprintln!(
            "Failed to index vault at {}: {e}",
            config.vault_path.display()
        );
        std::process::exit(1);
    });

    let state = AppState {
        explorer_tree: Arc::new(index.explorer_tree()),
        index: Arc::new(index),
    };

    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/api/tree", get(tree_handler))
        .route("/api/note/{slug}", get(note_handler))
        .route("/api/resolve", get(resolve_handler))
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

async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

async fn tree_handler(State(state): State<AppState>) -> impl IntoResponse {
    Json((*state.explorer_tree).clone())
}

async fn note_handler(
    Path(slug): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    match state.index.read_note_by_slug(&slug) {
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
    let slug = state
        .index
        .resolve_wikilink(&query.target)
        .map(|entry| entry.slug.clone());

    Json(ResolveResponse { slug })
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
    fn socket_addr_builds_expected_address() {
        let cfg = AppConfig {
            vault_path: PathBuf::from("./vault"),
            host: "0.0.0.0".to_string(),
            port: 42824,
        };

        let addr = cfg.socket_addr().expect("valid addr");
        assert_eq!(addr.to_string(), "0.0.0.0:42824");
    }
}
