use std::sync::Arc;

use axum::Router;
use axum::routing::{get, post};
use dotenvy::dotenv;
use tokio::sync::RwLock;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tracing::{error, info};

use hatchdoor::app_state::{AppConfig, AppState, build_cache_with_sqlite, init_logging};
use hatchdoor::cache::SqliteCache;
use hatchdoor::embed::{Embedder, FastembedEmbedder};
use hatchdoor::handlers::{
    graph_handler, health_handler, note_download_handler, note_handler, note_links_handler,
    recently_modified_handler, refresh_handler, resolve_batch_handler, resolve_handler,
    search_handler, spa_index_handler, stats_handler, tree_handler, vault_asset_handler,
    vault_events_handler,
};
use hatchdoor::mcp::{mcp_get_handler, mcp_post_handler};
use hatchdoor::vault_watcher::spawn_vault_watcher;

enum RunMode {
    Serve,
    PrefetchEmbedder,
    Unknown(String),
}

fn parse_run_mode(args: &[String]) -> RunMode {
    match args.get(1).map(String::as_str) {
        None => RunMode::Serve,
        Some("--prefetch-embedder") => RunMode::PrefetchEmbedder,
        Some(other) => RunMode::Unknown(other.to_string()),
    }
}

fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health_handler))
        .route("/mcp", get(mcp_get_handler).post(mcp_post_handler))
        .route("/api/tree", get(tree_handler))
        .route("/api/vault-events", get(vault_events_handler))
        .route("/api/recently-modified", get(recently_modified_handler))
        .route("/api/note/{slug}", get(note_handler))
        .route("/api/note/{slug}/download", get(note_download_handler))
        .route("/api/note/{slug}/links", get(note_links_handler))
        .route("/api/resolve", get(resolve_handler))
        .route("/api/resolve-batch", post(resolve_batch_handler))
        .route("/api/search", get(search_handler))
        .route("/api/stats", get(stats_handler))
        .route("/api/graph", get(graph_handler))
        .route("/api/refresh", post(refresh_handler))
        .route("/", get(spa_index_handler))
        .route("/n/{slug}", get(spa_index_handler))
        .route("/stats", get(spa_index_handler))
        .route("/graph", get(spa_index_handler))
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

async fn run_server() {
    let config = AppConfig::from_env().unwrap_or_else(|e| {
        error!("Configuration error: {e}");
        std::process::exit(1);
    });

    let sqlite = Arc::new(
        SqliteCache::open(&config.cache_db_path, 384).unwrap_or_else(|e| {
            error!(
                cache_db_path = %config.cache_db_path.display(),
                "SQLite cache startup failed: {e}"
            );
            std::process::exit(1);
        }),
    );

    let embedder: Arc<dyn Embedder> =
        Arc::new(FastembedEmbedder::bge_small().unwrap_or_else(|e| {
            error!("Failed to load embedder: {e}");
            std::process::exit(1);
        }));

    let cache = build_cache_with_sqlite(&config.vault_path, sqlite, embedder.as_ref())
        .unwrap_or_else(|e| {
            error!(
                "Failed to index vault at {} into SQLite cache {}: {e}",
                config.vault_path.display(),
                config.cache_db_path.display()
            );
            std::process::exit(1);
        });

    let (vault_events, _) = tokio::sync::broadcast::channel(64);
    let state = AppState {
        vault_path: config.vault_path.clone(),
        cache: Arc::new(RwLock::new(cache)),
        vault_revision: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        vault_events,
        embedder,
    };

    spawn_vault_watcher(
        state.clone(),
        config.vault_path.clone(),
        config.cache_db_path.clone(),
    );

    let app = build_router(state);

    let addr = config.socket_addr().unwrap_or_else(|e| {
        error!("Address error: {e}");
        std::process::exit(1);
    });

    info!(
        host = %config.host,
        port = config.port,
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

fn run_prefetch() {
    info!("Pre-fetching BGE-small-EN weights and tokenizer");
    match FastembedEmbedder::bge_small() {
        Ok(_) => info!("Pre-fetch complete"),
        Err(e) => {
            error!("Pre-fetch failed: {e}");
            std::process::exit(1);
        }
    }
}

#[tokio::main]
async fn main() {
    dotenv().ok();
    init_logging();

    let args: Vec<String> = std::env::args().collect();
    match parse_run_mode(&args) {
        RunMode::Serve => run_server().await,
        RunMode::PrefetchEmbedder => run_prefetch(),
        RunMode::Unknown(flag) => {
            error!("Unknown flag: {flag}");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use std::sync::atomic::Ordering;
    use std::time::Duration;
    use tempfile::TempDir;
    use tokio::sync::RwLock;
    use tokio_stream::StreamExt;
    use tower::ServiceExt;

    use hatchdoor::app_state::build_cache;
    use hatchdoor::embed::{Embedder, StubEmbedder};

    #[test]
    fn cli_recognises_prefetch_embedder_flag() {
        let args = vec!["hatchdoor".to_string(), "--prefetch-embedder".to_string()];
        assert!(matches!(parse_run_mode(&args), RunMode::PrefetchEmbedder));
    }

    #[test]
    fn cli_defaults_to_serve_mode() {
        let args = vec!["hatchdoor".to_string()];
        assert!(matches!(parse_run_mode(&args), RunMode::Serve));
    }

    #[test]
    fn cli_rejects_unknown_flags() {
        let args = vec!["hatchdoor".to_string(), "--bogus".to_string()];
        assert!(matches!(parse_run_mode(&args), RunMode::Unknown(_)));
    }

    fn app_for_tests() -> (Router, TempDir) {
        let (app, tmp, _state) = app_for_tests_with_state();
        (app, tmp)
    }

    fn app_for_tests_with_state() -> (Router, TempDir, AppState) {
        let tmp = TempDir::new().expect("temp dir");
        let vault_root = tmp.path().join("vault");
        std::fs::create_dir_all(&vault_root).expect("create vault");
        std::fs::write(vault_root.join("Home.md"), "# Home\n").expect("write note");
        let embedder: Arc<dyn Embedder> =
            Arc::new(StubEmbedder::new(384));
        let cache = build_cache(&vault_root, embedder.as_ref()).expect("cache");
        let (vault_events, _) = tokio::sync::broadcast::channel(64);
        let state = AppState {
            vault_path: vault_root,
            cache: Arc::new(RwLock::new(cache)),
            vault_revision: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            vault_events,
            embedder,
        };

        (build_router(state.clone()), tmp, state)
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
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
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
    async fn router_serves_vault_events_stream() {
        let (app, _tmp, state) = app_for_tests_with_state();
        state.vault_revision.store(7, Ordering::SeqCst);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/vault-events")
                    .method("GET")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], "text/event-stream");
        let mut stream = response.into_body().into_data_stream();
        let chunk = tokio::time::timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("first SSE event")
            .expect("stream item")
            .expect("body chunk");
        let event = std::str::from_utf8(&chunk).expect("utf8 event");
        assert!(event.contains("event: vault-revision"));
        assert!(event.contains("id: 7"));
        assert!(event.contains(r#"data: {"revision":7}"#));
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
            .clone()
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

        let modified = app
            .oneshot(
                Request::builder()
                    .uri("/api/recently-modified?limit=5")
                    .method("GET")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(modified.status(), StatusCode::OK);
        let body = to_bytes(modified.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(payload["notes"][0]["slug"], "home");
    }
}
