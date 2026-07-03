use std::sync::Arc;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{get, patch, post};
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
    archive_note_handler, create_note_handler, delete_note_handler, graph_handler, health_handler,
    move_note_handler, move_rename_note_handler, note_download_handler, note_handler,
    note_links_handler, recently_modified_handler, refresh_handler, rename_note_handler,
    resolve_batch_handler, resolve_handler, search_handler, spa_index_handler, stats_handler,
    tree_handler, update_note_handler, upload_attachment_handler, vault_asset_handler,
    vault_events_handler, write_capabilities_handler,
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

/// Multipart framing (boundary lines, field headers, the small
/// `target_relative_path` text field) wrapped around the uploaded file. The
/// attachment body limit is the configured max file size plus this slack, so a
/// file right at the advertised cap is not rejected by the framework before the
/// handler's own precise size check runs. Non-attachment routes keep axum's
/// small default limit.
const ATTACHMENT_MULTIPART_OVERHEAD: u64 = 64 * 1024;

fn build_router(state: AppState, web_bearer_token: Option<Arc<str>>) -> Router {
    let attachment_body_limit = state
        .mcp_config
        .max_attachment_bytes
        .saturating_add(ATTACHMENT_MULTIPART_OVERHEAD)
        .min(usize::MAX as u64) as usize;

    // Routes that expose vault data. When a web token is configured they sit
    // behind the auth layer; the SPA shell, /health, and /mcp stay open.
    let protected = Router::new()
        .route("/api/tree", get(tree_handler))
        .route("/api/vault-events", get(vault_events_handler))
        .route("/api/recently-modified", get(recently_modified_handler))
        .route("/api/note", post(create_note_handler))
        .route(
            "/api/attachment",
            post(upload_attachment_handler).layer(DefaultBodyLimit::max(attachment_body_limit)),
        )
        .route(
            "/api/note/{slug}",
            get(note_handler)
                .put(update_note_handler)
                .delete(delete_note_handler),
        )
        .route("/api/note/{slug}/rename", patch(rename_note_handler))
        .route("/api/note/{slug}/move", patch(move_note_handler))
        .route("/api/note/{slug}/archive", patch(archive_note_handler))
        .route(
            "/api/note/{slug}/move-rename",
            patch(move_rename_note_handler),
        )
        .route("/api/note/{slug}/download", get(note_download_handler))
        .route("/api/note/{slug}/links", get(note_links_handler))
        .route("/api/resolve", get(resolve_handler))
        .route("/api/resolve-batch", post(resolve_batch_handler))
        .route("/api/search", get(search_handler))
        .route("/api/stats", get(stats_handler))
        .route("/api/graph", get(graph_handler))
        .route("/api/refresh", post(refresh_handler))
        .route("/api/write-capabilities", get(write_capabilities_handler))
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
                git::SyncOps {
                    commit: Box::new(|cfg, paths, msg| git::commit_local(cfg, paths, msg)),
                    fetch: Box::new(git::fetch_remote),
                    integrate: Box::new(git::integrate_fetched),
                    push: Box::new(git::push_branch),
                },
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
        web_auth_enabled: config.web_bearer_token.is_some(),
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
        let (app, tmp, _state) = app_for_tests_with_web_auth(None);
        (app, tmp)
    }

    fn app_for_tests_with_web_auth(
        web_bearer_token: Option<Arc<str>>,
    ) -> (Router, TempDir, AppState) {
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
            web_auth_enabled: web_bearer_token.is_some(),
            vault_write_lock: Arc::new(tokio::sync::Mutex::new(())),
            git_sync: None,
            mcp_config: Arc::new(hatchdoor::mcp::McpConfig::disabled()),
            archive_prefix: Arc::from("90-archive/"),
            refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
        };

        (build_router(state.clone(), web_bearer_token), tmp, state)
    }

    fn app_for_tests_with_state() -> (Router, TempDir, AppState) {
        app_for_tests_with_web_auth(None)
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
            web_auth_enabled: false,
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
        let (app, _tmp, _state) = app_for_tests_with_web_auth(Some(Arc::from("secret-token")));

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

    #[tokio::test]
    async fn router_wires_write_capabilities_route() {
        let (app, _tmp) = app_for_tests();
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/write-capabilities")
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
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(payload["enabled"], true);
        assert!(
            payload["warnings"]
                .as_array()
                .expect("warnings")
                .iter()
                .any(|warning| warning.as_str().unwrap_or("").contains("unauthenticated"))
        );
    }

    #[tokio::test]
    async fn router_wires_write_capabilities_with_web_token() {
        let (_app, _tmp, state) = app_for_tests_with_web_auth(Some(Arc::from("secret-token")));
        let app = build_router(state, Some(Arc::from("secret-token")));

        let unauthorized = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/write-capabilities")
                    .method("GET")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let authorized = app
            .oneshot(
                Request::builder()
                    .uri("/api/write-capabilities")
                    .method("GET")
                    .header("authorization", "Bearer secret-token")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(authorized.status(), StatusCode::OK);
        let body = to_bytes(authorized.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(payload["enabled"], true);
        assert!(payload["warnings"].as_array().expect("warnings").is_empty());
    }

    #[tokio::test]
    async fn router_uploads_attachment_into_vault() {
        let (app, tmp) = app_for_tests();
        let boundary = "hatchdoor-test-boundary";
        let body = format!(
            "--{boundary}\r\n\
             Content-Disposition: form-data; name=\"target_relative_path\"\r\n\r\n\
             Attachments/pasted.png\r\n\
             --{boundary}\r\n\
             Content-Disposition: form-data; name=\"file\"; filename=\"pasted.png\"\r\n\
             Content-Type: image/png\r\n\r\n"
        )
        .into_bytes()
        .into_iter()
        .chain(b"png-bytes".iter().copied())
        .chain(format!("\r\n--{boundary}--\r\n").into_bytes())
        .collect::<Vec<_>>();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/attachment")
                    .method("POST")
                    .header(
                        "content-type",
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(body))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(
            json["attachment"]["relative_path"],
            "Attachments/pasted.png"
        );
        assert_eq!(json["attachment"]["size_bytes"], 9);
        assert_eq!(
            std::fs::read(tmp.path().join("vault/Attachments/pasted.png")).expect("file"),
            b"png-bytes"
        );
    }

    #[tokio::test]
    async fn router_accepts_attachment_between_2mb_and_configured_max() {
        // The default McpConfig caps attachments at 10 MB. A 3 MB upload is well
        // within that, but exceeds axum's built-in 2 MB body limit — without an
        // explicit DefaultBodyLimit the framework rejects it before the handler
        // (and its real size check) ever runs.
        let (app, tmp) = app_for_tests();
        let boundary = "hatchdoor-test-boundary";
        let file_bytes = vec![b'x'; 3 * 1024 * 1024];
        let body = format!(
            "--{boundary}\r\n\
             Content-Disposition: form-data; name=\"target_relative_path\"\r\n\r\n\
             Attachments/big.png\r\n\
             --{boundary}\r\n\
             Content-Disposition: form-data; name=\"file\"; filename=\"big.png\"\r\n\
             Content-Type: image/png\r\n\r\n"
        )
        .into_bytes()
        .into_iter()
        .chain(file_bytes.iter().copied())
        .chain(format!("\r\n--{boundary}--\r\n").into_bytes())
        .collect::<Vec<_>>();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/attachment")
                    .method("POST")
                    .header(
                        "content-type",
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(body))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            std::fs::read(tmp.path().join("vault/Attachments/big.png"))
                .expect("file")
                .len(),
            3 * 1024 * 1024
        );
    }

    #[tokio::test]
    async fn write_api_updates_note_and_rejects_stale_hash() {
        let (app, _tmp) = app_for_tests();

        let note_response = app
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
        let note_body = to_bytes(note_response.into_body(), usize::MAX)
            .await
            .expect("note body");
        let note_payload: serde_json::Value = serde_json::from_slice(&note_body).expect("json");
        let hash = note_payload["note"]["content_hash"].as_str().expect("hash");

        let update = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/note/home")
                    .method("PUT")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r##"{{"content":"# Home\nupdated\n","expected_content_hash":"{hash}"}}"##
                    )))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(update.status(), StatusCode::OK);

        let stale = app
            .oneshot(
                Request::builder()
                    .uri("/api/note/home")
                    .method("PUT")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r##"{{"content":"# Home\nstale overwrite\n","expected_content_hash":"{hash}"}}"##
                    )))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(stale.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn write_api_rejects_update_payload_missing_expected_hash() {
        let (app, _tmp) = app_for_tests();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/note/home")
                    .method("PUT")
                    .header("content-type", "application/json")
                    .body(Body::from(r##"{"content":"# Home\nupdated\n"}"##))
                    .expect("request"),
            )
            .await
            .expect("response");

        // Well-formed JSON missing a required field is a 422 (Unprocessable
        // Entity) — the real status axum's Json extractor reports. write_payload
        // now preserves it instead of masking every rejection as 400.
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn write_api_oversized_json_body_reports_413_not_400() {
        // A JSON write body over axum's 2 MB limit is a length-limit rejection
        // (413), not a malformed-body one (400). write_payload must preserve the
        // rejection's real status for clients/proxies that key off status codes.
        let (app, _tmp) = app_for_tests();
        let big = "x".repeat(3 * 1024 * 1024);
        let body = format!(r#"{{"relative_path":"Big.md","content":"{big}"}}"#);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/note")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn write_api_rejects_create_path_traversal() {
        let (app, _tmp) = app_for_tests();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/note")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r##"{"relative_path":"../escape.md","content":"# Nope\n"}"##,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn write_api_delete_rejects_stale_hash() {
        let (app, _tmp) = app_for_tests();

        let note_response = app
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
        let note_body = to_bytes(note_response.into_body(), usize::MAX)
            .await
            .expect("note body");
        let note_payload: serde_json::Value = serde_json::from_slice(&note_body).expect("json");
        let original_hash = note_payload["note"]["content_hash"].as_str().expect("hash");

        let update = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/note/home")
                    .method("PUT")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r##"{{"content":"# Home\nfresh content\n","expected_content_hash":"{original_hash}"}}"##
                    )))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(update.status(), StatusCode::OK);

        let stale_delete = app
            .oneshot(
                Request::builder()
                    .uri("/api/note/home")
                    .method("DELETE")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"expected_content_hash":"{original_hash}"}}"#
                    )))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(stale_delete.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn write_api_successful_write_updates_vault_revision() {
        let (app, _tmp, state) = app_for_tests_with_state();
        let before = state.vault_revision.load(Ordering::SeqCst);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/note")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r##"{"relative_path":"Projects/Revision Note.md","content":"# Revision Note\n"}"##,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let after = state.vault_revision.load(Ordering::SeqCst);
        assert!(
            after > before,
            "vault revision should advance after refresh"
        );
    }

    #[tokio::test]
    async fn write_api_creates_renames_moves_and_deletes_note() {
        let (app, _tmp) = app_for_tests();

        let create = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/note")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r##"{"relative_path":"Projects/New Note.md","content":"# New Note\n"}"##,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(create.status(), StatusCode::OK);
        let create_body = to_bytes(create.into_body(), usize::MAX)
            .await
            .expect("body");
        let created: serde_json::Value = serde_json::from_slice(&create_body).expect("json");
        let created_object = created.as_object().expect("object");
        for field in [
            "ok",
            "slug",
            "relative_path",
            "content_hash",
            "quality_warnings",
            "rewritten_notes",
            "moved_assets",
            "trashed_path",
            "git_sync_warning",
        ] {
            assert!(created_object.contains_key(field), "missing field {field}");
        }
        assert_eq!(created["ok"], true);
        let slug = created["slug"].as_str().expect("slug");
        let hash = created["content_hash"].as_str().expect("hash");

        let duplicate_create = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/note")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r##"{"relative_path":"Projects/New Note.md","content":"# Duplicate\n"}"##,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(duplicate_create.status(), StatusCode::CONFLICT);

        let rename = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/note/{slug}/rename"))
                    .method("PATCH")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"new_title":"Renamed Note","expected_content_hash":"{hash}"}}"#
                    )))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(rename.status(), StatusCode::OK);
        let rename_body = to_bytes(rename.into_body(), usize::MAX)
            .await
            .expect("body");
        let renamed: serde_json::Value = serde_json::from_slice(&rename_body).expect("json");
        let renamed_slug = renamed["slug"].as_str().expect("renamed slug");
        let renamed_hash = renamed["content_hash"].as_str().expect("renamed hash");

        let move_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/note/{renamed_slug}/move"))
                    .method("PATCH")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"target_folder":"Archive","expected_content_hash":"{renamed_hash}"}}"#
                    )))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(move_response.status(), StatusCode::OK);
        let move_body = to_bytes(move_response.into_body(), usize::MAX)
            .await
            .expect("body");
        let moved: serde_json::Value = serde_json::from_slice(&move_body).expect("json");
        let moved_slug = moved["slug"].as_str().expect("moved slug");
        let moved_hash = moved["content_hash"].as_str().expect("moved hash");

        let archive = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/note/{moved_slug}/archive"))
                    .method("PATCH")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"expected_content_hash":"{moved_hash}"}}"#
                    )))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(archive.status(), StatusCode::OK);
        let archive_body = to_bytes(archive.into_body(), usize::MAX)
            .await
            .expect("body");
        let archived: serde_json::Value = serde_json::from_slice(&archive_body).expect("json");
        let archived_slug = archived["slug"].as_str().expect("archived slug");
        let archived_hash = archived["content_hash"].as_str().expect("archived hash");
        assert_eq!(archived["relative_path"], "90-archive/Renamed Note");

        let delete = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/note/{archived_slug}"))
                    .method("DELETE")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"expected_content_hash":"{archived_hash}"}}"#
                    )))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(delete.status(), StatusCode::OK);
    }
}
