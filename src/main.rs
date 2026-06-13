use std::sync::Arc;

use axum::Router;
use axum::routing::{get, post};
use dotenvy::dotenv;
use tokio::sync::RwLock;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tracing::{error, info};

use hatchdoor::app_state::{AppConfig, AppState, build_cache_with_sqlite, init_logging};
use hatchdoor::auth::{WebToken, require_web_token};
use hatchdoor::cache::SqliteCache;
use hatchdoor::embed::{Embedder, FastembedEmbedder};
use hatchdoor::git::{self, GitConfig};
use hatchdoor::handlers::{
    graph_handler, health_handler, note_download_handler, note_handler, note_links_handler,
    recently_modified_handler, refresh_handler, resolve_batch_handler, resolve_handler,
    search_handler, spa_index_handler, stats_handler, tree_handler, vault_asset_handler,
    vault_events_handler,
};
use hatchdoor::mcp::{McpConfig, mcp_get_handler, mcp_post_handler};
use hatchdoor::vault_watcher::spawn_vault_watcher;

enum RunMode {
    Serve,
    PrefetchEmbedder,
    Healthcheck,
    Unknown(String),
}

fn parse_run_mode(args: &[String]) -> RunMode {
    match args.get(1).map(String::as_str) {
        None => RunMode::Serve,
        Some("--prefetch-embedder") => RunMode::PrefetchEmbedder,
        Some("--healthcheck") => RunMode::Healthcheck,
        Some(other) => RunMode::Unknown(other.to_string()),
    }
}

fn build_router(state: AppState, web_bearer_token: Option<Arc<str>>) -> Router {
    // Routes that expose vault data. When a web token is configured they sit
    // behind the auth layer; the SPA shell, /health, and /mcp stay open.
    let protected = Router::new()
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
        .route("/vault-assets/{*path}", get(vault_asset_handler));

    let protected = match web_bearer_token {
        Some(token) => protected.layer(axum::middleware::from_fn_with_state(
            WebToken(token),
            require_web_token,
        )),
        None => protected,
    };

    Router::new()
        .route("/health", get(health_handler))
        .route("/mcp", get(mcp_get_handler).post(mcp_post_handler))
        .merge(protected)
        .route("/", get(spa_index_handler))
        .route("/n/{slug}", get(spa_index_handler))
        .route("/stats", get(spa_index_handler))
        .route("/graph", get(spa_index_handler))
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

    let mcp_config = Arc::new(McpConfig::from_env_validated().unwrap_or_else(|e| {
        error!("MCP configuration error: {e}");
        std::process::exit(1);
    }));

    if config.host == "0.0.0.0" && config.web_bearer_token.is_none() {
        info!(
            "HOST=0.0.0.0 with no HATCHDOOR_WEB_BEARER_TOKEN set: the API is reachable \
             unauthenticated on all interfaces. Set a token or front it with an authenticating proxy."
        );
    }

    let sqlite = Arc::new(
        SqliteCache::open(&config.cache_db_path, 768).unwrap_or_else(|e| {
            error!(
                cache_db_path = %config.cache_db_path.display(),
                "SQLite cache startup failed: {e}"
            );
            std::process::exit(1);
        }),
    );

    let embedder: Arc<dyn Embedder> =
        Arc::new(FastembedEmbedder::nomic_v1_5().unwrap_or_else(|e| {
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

    let vault_write_lock = Arc::new(tokio::sync::Mutex::new(()));

    let git_sync = match GitConfig::from_env(config.vault_path.clone()) {
        Ok(None) => None,
        Ok(Some(git_config)) => {
            if let Err(e) = git::validate_repo(&git_config) {
                error!("Git sync configuration invalid: {e}");
                std::process::exit(1);
            }
            // The background task flushes any commits stranded by an earlier
            // outage immediately on startup (see spawn_sync_task).
            let handle = git::spawn_sync_task(
                git_config.clone(),
                vault_write_lock.clone(),
                |cfg, paths, msg| git::sync(cfg, paths, msg).map(|report| report.outcome),
            );
            info!("Git sync enabled");
            Some(handle)
        }
        Err(e) => {
            error!("Git sync configuration error: {e}");
            std::process::exit(1);
        }
    };

    let (vault_events, _) = tokio::sync::broadcast::channel(64);
    let state = AppState {
        vault_path: config.vault_path.clone(),
        cache: Arc::new(RwLock::new(cache)),
        vault_revision: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        vault_events,
        embedder,
        vault_write_lock,
        git_sync,
        mcp_config,
        archive_prefix: Arc::from(config.archive_prefix.as_str()),
        refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
    };

    spawn_vault_watcher(
        state.clone(),
        config.vault_path.clone(),
        config.cache_db_path.clone(),
    );

    let web_bearer_token = config.web_bearer_token.clone().map(Arc::from);
    let app = build_router(state, web_bearer_token);

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
    info!("Pre-fetching Nomic Embed Text v1.5 weights and tokenizer");
    match FastembedEmbedder::nomic_v1_5() {
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
        RunMode::Healthcheck => run_healthcheck(),
        RunMode::Unknown(flag) => {
            error!("Unknown flag: {flag}");
            std::process::exit(2);
        }
    }
}

/// Container health probe: hit the local `/health` endpoint over a raw socket
/// (the distroless runtime has no shell or curl) and exit non-zero on failure.
fn run_healthcheck() {
    use std::io::{Read, Write};

    let port = std::env::var("PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(42824);
    let addr = format!("127.0.0.1:{port}");

    let probe = || -> std::io::Result<bool> {
        let mut stream = std::net::TcpStream::connect(&addr)?;
        stream.set_read_timeout(Some(std::time::Duration::from_secs(4)))?;
        stream.set_write_timeout(Some(std::time::Duration::from_secs(4)))?;
        stream
            .write_all(b"GET /health HTTP/1.0\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;
        let mut response = String::new();
        stream.read_to_string(&mut response)?;
        let status_ok = response
            .lines()
            .next()
            .map(|line| line.contains(" 200"))
            .unwrap_or(false);
        Ok(status_ok)
    };

    match probe() {
        Ok(true) => std::process::exit(0),
        Ok(false) => {
            eprintln!("healthcheck: endpoint did not report healthy");
            std::process::exit(1);
        }
        Err(error) => {
            eprintln!("healthcheck: {error}");
            std::process::exit(1);
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
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
        let cache = build_cache(&vault_root, embedder.as_ref()).expect("cache");
        let (vault_events, _) = tokio::sync::broadcast::channel(64);
        let state = AppState {
            vault_path: vault_root,
            cache: Arc::new(RwLock::new(cache)),
            vault_revision: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            vault_events,
            embedder,
            vault_write_lock: Arc::new(tokio::sync::Mutex::new(())),
            git_sync: None,
            mcp_config: Arc::new(hatchdoor::mcp::McpConfig::disabled()),
            archive_prefix: Arc::from("90-archive/"),
            refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
        };

        (build_router(state.clone(), None), tmp, state)
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
    async fn resolve_batch_marks_archived_notes() {
        let tmp = TempDir::new().expect("temp dir");
        let vault_root = tmp.path().join("vault");
        let archive_dir = vault_root.join("90-archive");
        std::fs::create_dir_all(&archive_dir).expect("create archive dir");
        std::fs::write(vault_root.join("Home.md"), "# Home\n").expect("write home");
        std::fs::write(archive_dir.join("Old Setup.md"), "# Old Setup\n")
            .expect("write archived note");
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
        let cache = build_cache(&vault_root, embedder.as_ref()).expect("cache");
        let (vault_events, _) = tokio::sync::broadcast::channel(64);
        let state = AppState {
            vault_path: vault_root,
            cache: Arc::new(RwLock::new(cache)),
            vault_revision: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            vault_events,
            embedder,
            vault_write_lock: Arc::new(tokio::sync::Mutex::new(())),
            git_sync: None,
            mcp_config: Arc::new(hatchdoor::mcp::McpConfig::disabled()),
            archive_prefix: Arc::from("90-archive/"),
            refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
        };
        let app = build_router(state, None);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/resolve-batch")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"targets":["Home","90-archive/Old Setup"]}"#))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
        let results = payload["results"].as_array().expect("results array");

        let home = results
            .iter()
            .find(|r| r["target"] == "Home")
            .expect("home result");
        assert_eq!(home["archived"], false, "Home should not be archived");

        let archived = results
            .iter()
            .find(|r| r["target"] == "90-archive/Old Setup")
            .expect("archived result");
        assert_eq!(
            archived["archived"], true,
            "90-archive note should be archived"
        );
    }

    #[tokio::test]
    async fn web_token_guards_api_routes_but_not_health_or_spa() {
        let (_app, _tmp, state) = app_for_tests_with_state();
        let app = build_router(state, Some(Arc::from("secret-token")));

        let no_token = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/tree")
                    .method("GET")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(no_token.status(), StatusCode::UNAUTHORIZED);

        let with_header = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/tree")
                    .method("GET")
                    .header("authorization", "Bearer secret-token")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(with_header.status(), StatusCode::OK);

        let with_query = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/tree?access_token=secret-token")
                    .method("GET")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(with_query.status(), StatusCode::OK);

        let health = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .method("GET")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(health.status(), StatusCode::OK);

        let root = app
            .oneshot(
                Request::builder()
                    .uri("/")
                    .method("GET")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_ne!(root.status(), StatusCode::UNAUTHORIZED);
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
