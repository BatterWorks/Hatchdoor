mod api_types;
mod app_state;
mod cache;
mod handlers;
mod mcp;
mod vault;

use std::sync::Arc;
use std::time::Duration;

use axum::routing::{get, post};
use axum::Router;
use dotenvy::dotenv;
use tokio::sync::RwLock;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tracing::{error, info};

use crate::app_state::{build_cache, build_test_cache, init_logging, AppConfig, AppState};
use crate::cache::SqliteCache;
use crate::handlers::{
    health_handler, note_download_handler, note_handler, note_links_handler, refresh_handler,
    resolve_batch_handler, resolve_handler, search_handler, spa_index_handler, tree_handler,
    vault_asset_handler,
};
use crate::mcp::{mcp_get_handler, mcp_post_handler};

fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health_handler))
        .route("/mcp", get(mcp_get_handler).post(mcp_post_handler))
        .route("/api/tree", get(tree_handler))
        .route("/api/note/{slug}", get(note_handler))
        .route("/api/note/{slug}/download", get(note_download_handler))
        .route("/api/note/{slug}/links", get(note_links_handler))
        .route("/api/resolve", get(resolve_handler))
        .route("/api/resolve-batch", post(resolve_batch_handler))
        .route("/api/search", get(search_handler))
        .route("/api/refresh", post(refresh_handler))
        .route("/", get(spa_index_handler))
        .route("/n/{slug}", get(spa_index_handler))
        .route("/vault-assets/{*path}", get(vault_asset_handler))
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
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().include_headers(false))
                .on_response(DefaultOnResponse::new().include_headers(false)),
        )
        .with_state(state)
}

#[tokio::main]
async fn main() {
    dotenv().ok();
    init_logging();

    let config = AppConfig::from_env().unwrap_or_else(|e| {
        error!("Configuration error: {e}");
        std::process::exit(1);
    });

    let sqlite = Arc::new(SqliteCache::open(&config.cache_db_path).unwrap_or_else(|e| {
        error!(
            cache_db_path = %config.cache_db_path.display(),
            "SQLite cache startup failed: {e}"
        );
        std::process::exit(1);
    }));

    let cache = build_cache(&config.vault_path, sqlite).unwrap_or_else(|e| {
        error!(
            "Failed to index vault at {} into SQLite cache {}: {e}",
            config.vault_path.display(),
            config.cache_db_path.display()
        );
        std::process::exit(1);
    });

    let state = AppState {
        vault_path: config.vault_path.clone(),
        refresh_interval: Duration::from_secs(config.refresh_seconds),
        cache: Arc::new(RwLock::new(cache)),
    };

    let app = build_router(state);

    let addr = config.socket_addr().unwrap_or_else(|e| {
        error!("Address error: {e}");
        std::process::exit(1);
    });

    info!(
        host = %config.host,
        port = config.port,
        refresh_seconds = config.refresh_seconds,
        vault_path = %config.vault_path.display(),
        cache_db_path = %config.cache_db_path.display(),
        "Hatchdoor starting"
    );
    info!("Hatchdoor listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|e| {
            error!("Failed to bind: {e}");
            std::process::exit(1);
        });

    axum::serve(listener, app).await.unwrap_or_else(|e| {
        error!("Server error: {e}");
        std::process::exit(1);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use std::time::Duration;
    use tempfile::TempDir;
    use tokio::sync::RwLock;
    use tower::ServiceExt;

    fn app_for_tests() -> (Router, TempDir) {
        let tmp = TempDir::new().expect("temp dir");
        let vault_root = tmp.path().join("vault");
        std::fs::create_dir_all(&vault_root).expect("create vault");
        std::fs::write(vault_root.join("Home.md"), "# Home\n").expect("write note");
        let cache = build_test_cache(&vault_root).expect("cache");
        let state = AppState {
            vault_path: vault_root,
            refresh_interval: Duration::from_secs(60),
            cache: Arc::new(RwLock::new(cache)),
        };

        (build_router(state), tmp)
    }

    #[tokio::test]
    async fn router_health_route_returns_ok() {
        let (app, _tmp) = app_for_tests();
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .method("GET")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.expect("body");
        assert_eq!(&body[..], b"ok");
    }

    #[tokio::test]
    async fn router_enforces_http_methods_for_api_routes() {
        let (app, _tmp) = app_for_tests();
        let tree_post = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/tree")
                    .method("POST")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(tree_post.status(), StatusCode::METHOD_NOT_ALLOWED);

        let refresh_get = app
            .oneshot(
                Request::builder()
                    .uri("/api/refresh")
                    .method("GET")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(refresh_get.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn router_wires_core_api_routes() {
        let (app, _tmp) = app_for_tests();

        let note = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/note/home")
                    .method("GET")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(note.status(), StatusCode::OK);

        let resolve_batch = app
            .oneshot(
                Request::builder()
                    .uri("/api/resolve-batch")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"targets":["Home"]}"#))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(resolve_batch.status(), StatusCode::OK);
    }
}
