mod api_types;
mod app_state;
mod handlers;
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

use crate::app_state::{build_cache, init_logging, AppConfig, AppState};
use crate::handlers::{
    health_handler, note_download_handler, note_handler, note_links_handler, refresh_handler,
    resolve_batch_handler, resolve_handler, search_handler, spa_index_handler, tree_handler,
    vault_asset_handler,
};

#[tokio::main]
async fn main() {
    dotenv().ok();
    init_logging();

    let config = AppConfig::from_env().unwrap_or_else(|e| {
        error!("Configuration error: {e}");
        std::process::exit(1);
    });

    let cache = build_cache(&config.vault_path).unwrap_or_else(|e| {
        error!(
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
        .with_state(state);

    let addr = config.socket_addr().unwrap_or_else(|e| {
        error!("Address error: {e}");
        std::process::exit(1);
    });

    info!(
        host = %config.host,
        port = config.port,
        refresh_seconds = config.refresh_seconds,
        vault_path = %config.vault_path.display(),
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
