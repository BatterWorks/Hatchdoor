//! HTTP server composition: router construction, startup security-posture
//! checks, and the `serve` run loop. Kept in the library (rather than the binary
//! root) so the HTTP surface is reachable from integration tests.

use std::sync::{Arc, OnceLock};

use axum::extract::DefaultBodyLimit;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use tokio::sync::RwLock;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::{DefaultOnResponse, TraceLayer};
use tracing::{error, info};

use crate::app_state::{AppState, VaultCache, build_cache_with_sqlite_and_progress};
use crate::auth::{WebOrMcpToken, WebToken, require_web_or_mcp_token, require_web_token};
use crate::cache::SqliteCache;
use crate::config::AppConfig;
use crate::embed::{Embedder, FastembedEmbedder};
use crate::git::{self, GitConfig};
use crate::handlers::{
    archive_note_handler, create_note_handler, delete_note_handler, graph_handler, health_handler,
    move_note_handler, move_rename_note_handler, note_download_handler, note_handler,
    note_links_handler, recently_modified_handler, refresh_handler, rename_note_handler,
    resolve_batch_handler, resolve_handler, search_handler, spa_index_handler, stats_handler,
    tree_handler, update_note_handler, upload_attachment_handler, vault_asset_handler,
    vault_events_handler, write_capabilities_handler,
};
use crate::mcp::{McpConfig, mcp_get_handler, mcp_post_handler};
use crate::startup::StartupTracker;
use crate::vault_watcher::spawn_vault_watcher;

/// Hosts that only accept connections from the local machine. Binding to any
/// other address exposes the port to the network.
fn is_loopback_host(host: &str) -> bool {
    matches!(host.trim(), "127.0.0.1" | "::1" | "[::1]" | "localhost")
}

/// Refuse to serve mutating routes unauthenticated on a public interface. A
/// non-loopback bind with no web token would let anyone on the network
/// create/overwrite/delete vault notes, guarded only by a log line.
pub fn check_web_auth_posture(
    host: &str,
    has_web_token: bool,
    demo_mode: bool,
) -> Result<(), String> {
    if demo_mode {
        return Ok(());
    }
    if !is_loopback_host(host) && !has_web_token {
        return Err(format!(
            "HOST={host} is non-loopback but HATCHDOOR_WEB_BEARER_TOKEN is unset: refusing to \
             start unauthenticated on a public interface. Set HATCHDOOR_WEB_BEARER_TOKEN, or bind \
             to 127.0.0.1. For a read-only public demo, set HATCHDOOR_DEMO_MODE=true."
        ));
    }
    Ok(())
}

pub fn check_demo_mode_posture(
    demo_mode: bool,
    mcp_enabled: bool,
    git_sync_enabled: bool,
) -> Result<(), String> {
    if !demo_mode {
        return Ok(());
    }
    if mcp_enabled {
        return Err(
            "HATCHDOOR_DEMO_MODE=true is incompatible with HATCHDOOR_MCP_ENABLED=true; disable MCP for public demos."
                .to_string(),
        );
    }
    if git_sync_enabled {
        return Err(
            "HATCHDOOR_DEMO_MODE=true is incompatible with HATCHDOOR_GIT_SYNC_ENABLED=true; disable git sync for public demos."
                .to_string(),
        );
    }
    Ok(())
}

/// Multipart framing (boundary lines, field headers, the small
/// `target_relative_path` text field) wrapped around the uploaded file. The
/// attachment body limit is the configured max file size plus this slack, so a
/// file right at the advertised cap is not rejected by the framework before the
/// handler's own precise size check runs. Non-attachment routes keep axum's
/// small default limit.
const ATTACHMENT_MULTIPART_OVERHEAD: u64 = 64 * 1024;

pub fn build_router(state: AppState, web_bearer_token: Option<Arc<str>>) -> Router {
    let attachment_body_limit = state
        .mcp_config
        .max_attachment_bytes
        .saturating_add(ATTACHMENT_MULTIPART_OVERHEAD)
        .min(usize::MAX as u64) as usize;
    // The base64 import_attachment tool carries file bytes inside the JSON-RPC
    // body, which base64 inflates by ~4/3. Size the /mcp body limit from the
    // base64 cap plus that inflation so a legitimately-sized upload is not
    // rejected before the tool's own decoded-size check runs.
    let mcp_body_limit = state
        .mcp_config
        .max_base64_bytes
        .saturating_mul(4)
        .div_ceil(3)
        .saturating_add(ATTACHMENT_MULTIPART_OVERHEAD)
        .min(usize::MAX as u64) as usize;

    // Routes that expose vault data sit behind startup readiness and, when
    // configured, web authentication. The SPA shell and status routes stay open.
    let protected = Router::new()
        .route("/api/tree", get(tree_handler))
        .route("/api/vault-events", get(vault_events_handler))
        .route("/api/recently-modified", get(recently_modified_handler))
        .route("/api/note", post(create_note_handler))
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

    let protected = match web_bearer_token.clone() {
        Some(token) => protected.layer(axum::middleware::from_fn_with_state(
            WebToken(token),
            require_web_token,
        )),
        None => protected,
    };
    let protected = protected.layer(axum::middleware::from_fn_with_state(
        state.clone(),
        require_vault_ready,
    ));

    // The attachment endpoint sits outside the shared `protected` group: an MCP
    // agent that already holds the MCP bearer token can use it directly,
    // without provisioning the separate web token just for this one route. It
    // still accepts the web token too, since the web UI's paste-to-upload flow
    // hits the same endpoint.
    let mcp_bearer_token = state.mcp_config.bearer_token.clone().map(Arc::from);
    let attachment = Router::new().route(
        "/api/attachment",
        post(upload_attachment_handler).layer(DefaultBodyLimit::max(attachment_body_limit)),
    );
    let attachment = if web_bearer_token.is_some() || mcp_bearer_token.is_some() {
        attachment.layer(axum::middleware::from_fn_with_state(
            WebOrMcpToken {
                web: web_bearer_token.clone(),
                mcp: mcp_bearer_token,
            },
            require_web_or_mcp_token,
        ))
    } else {
        attachment
    };
    let attachment = attachment.layer(axum::middleware::from_fn_with_state(
        state.clone(),
        require_vault_ready,
    ));

    let mcp = Router::new()
        .route("/mcp", get(mcp_get_handler).post(mcp_post_handler))
        .layer(DefaultBodyLimit::max(mcp_body_limit))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            require_vault_ready,
        ));

    Router::new()
        .route("/health", get(health_handler))
        .route("/ready", get(readiness_handler))
        .route("/api/startup-status", get(startup_status_handler))
        .merge(mcp)
        .merge(protected)
        .merge(attachment)
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
                // Custom span so the URI logged never contains the raw web token
                // that `<img>`/download URLs may carry as ?access_token=...
                .make_span_with(|request: &axum::http::Request<axum::body::Body>| {
                    let target = match request.uri().query() {
                        Some(query) => format!(
                            "{}?{}",
                            request.uri().path(),
                            crate::auth::redact_query_token(query)
                        ),
                        None => request.uri().path().to_string(),
                    };
                    tracing::info_span!(
                        "request",
                        method = %request.method(),
                        uri = %target,
                        version = ?request.version(),
                    )
                })
                .on_response(DefaultOnResponse::new().include_headers(false)),
        )
        .with_state(state)
}

async fn startup_status_handler(State(state): State<AppState>) -> Response {
    let mut response = Json(state.startup.status()).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

async fn readiness_handler(State(state): State<AppState>) -> Response {
    if state.startup.is_ready() {
        (StatusCode::OK, "ready").into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "not ready").into_response()
    }
}

async fn require_vault_ready(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    if state.startup.is_ready() {
        return next.run(request).await;
    }
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(serde_json::json!({
            "error": "Vault is still being indexed",
            "code": "vault_indexing"
        })),
    )
        .into_response()
}

pub async fn run_server() {
    let config = AppConfig::from_env().unwrap_or_else(|e| {
        error!("Configuration error: {e}");
        std::process::exit(1);
    });

    let mcp_config = Arc::new(McpConfig::from_env_validated().unwrap_or_else(|e| {
        error!("MCP configuration error: {e}");
        std::process::exit(1);
    }));

    if let Err(message) = check_web_auth_posture(
        &config.host,
        config.web_bearer_token.is_some(),
        config.demo_mode,
    ) {
        error!("{message}");
        std::process::exit(1);
    }

    let git_sync_config = GitConfig::from_env(config.vault_path.clone()).unwrap_or_else(|e| {
        error!("Git sync configuration error: {e}");
        std::process::exit(1);
    });

    if let Err(message) = check_demo_mode_posture(
        config.demo_mode,
        mcp_config.enabled,
        git_sync_config.is_some(),
    ) {
        error!("{message}");
        std::process::exit(1);
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

    let scan_config = Arc::new(crate::vault::VaultScanConfig {
        exclude: crate::vault::ExcludeMatcher::new(&config.exclude_patterns).unwrap_or_else(|e| {
            error!("Invalid HATCHDOOR_EXCLUDE configuration: {e}");
            std::process::exit(1);
        }),
    });
    for (pattern, source) in scan_config.exclude.configured_patterns() {
        info!(pattern = %pattern, source, "Noise-exclusion pattern active");
    }
    info!(
        embed_layers = config.embed_layers,
        "Demoted-layer vector embedding (HATCHDOOR_EMBED_LAYERS)"
    );

    let vault_write_lock = Arc::new(tokio::sync::Mutex::new(()));
    if let Some(git_config) = &git_sync_config
        && let Err(e) = git::validate_repo(git_config)
    {
        error!("Git sync configuration invalid: {e}");
        std::process::exit(1);
    }

    let (vault_events, _) = tokio::sync::broadcast::channel(64);
    let (mcp_tools_changed, _) = tokio::sync::broadcast::channel(16);
    let startup = StartupTracker::scanning();
    let state = AppState {
        vault_path: config.vault_path.clone(),
        cache: Arc::new(RwLock::new(VaultCache {
            sqlite: sqlite.clone(),
        })),
        vault_revision: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        vault_events,
        mcp_tools_changed,
        embedder,
        web_auth_enabled: config.web_bearer_token.is_some(),
        demo_mode: config.demo_mode,
        vault_write_lock,
        git_sync: Arc::new(OnceLock::new()),
        mcp_config,
        archive_prefix: Arc::from(config.archive_prefix.as_str()),
        scan_config: scan_config.clone(),
        refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
        startup,
    };

    let web_bearer_token = config.web_bearer_token.clone().map(Arc::from);
    let app = build_router(state.clone(), web_bearer_token);

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

    let indexing_state = state.clone();
    let vault_path = config.vault_path.clone();
    let cache_db_path = config.cache_db_path.clone();
    let indexing_sqlite = sqlite.clone();
    let indexing_embedder = state.embedder.clone();
    tokio::spawn(async move {
        let tracker = indexing_state.startup.clone();
        let progress_tracker = tracker.clone();
        let on_progress = Arc::new(move |progress| {
            progress_tracker.set_indexing(progress);
        });
        let indexing_vault_path = vault_path.clone();
        let indexing_scan_config = indexing_state.scan_config.clone();
        let result = tokio::task::spawn_blocking(move || {
            build_cache_with_sqlite_and_progress(
                &indexing_vault_path,
                indexing_sqlite,
                indexing_embedder.as_ref(),
                Some(on_progress),
                &indexing_scan_config,
            )
        })
        .await;

        match result {
            Ok(Ok(_)) => {
                tracker.set_ready();
                if let Some(git_config) = git_sync_config {
                    // Start sync only after the first consistent index is committed.
                    let handle = git::spawn_sync_task(
                        git_config,
                        indexing_state.vault_write_lock.clone(),
                        git::SyncOps {
                            commit: Box::new(git::commit_local),
                            fetch: Box::new(git::fetch_remote),
                            integrate: Box::new(git::integrate_fetched),
                            push: Box::new(git::push_branch),
                        },
                    );
                    let _ = indexing_state.git_sync.set(handle);
                    info!("Git sync enabled");
                }
                spawn_vault_watcher(indexing_state, vault_path, cache_db_path);
            }
            Ok(Err(e)) => {
                tracker.set_failed();
                error!(
                    "Failed to index vault at {} into SQLite cache {}: {e}. The vault watcher \
                     will retry on the next change — correct the error (e.g. a malformed \
                     .hatchdoor-layer marker) to recover without a restart. Git sync, if \
                     configured, was not started and requires a restart.",
                    vault_path.display(),
                    cache_db_path.display()
                );
                // Spawn the watcher even though the first index failed, so a
                // corrected vault triggers a recovering reindex (run_reindex
                // clears the failed startup state on success). git_sync_config is
                // intentionally dropped here: git sync begins only after a clean
                // startup index.
                spawn_vault_watcher(indexing_state, vault_path, cache_db_path);
            }
            Err(e) => {
                tracker.set_failed();
                error!("Vault indexing task failed: {e}");
            }
        }
    });

    axum::serve(listener, app).await.unwrap_or_else(|e| {
        error!("Server error: {e}");
        std::process::exit(1);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_auth_posture_refuses_public_bind_without_token() {
        // Non-loopback host with no web token must refuse to start.
        assert!(check_web_auth_posture("0.0.0.0", false, false).is_err());
        assert!(check_web_auth_posture("192.168.1.50", false, false).is_err());
        assert!(check_web_auth_posture("::", false, false).is_err());
        // A token makes any host acceptable.
        assert!(check_web_auth_posture("0.0.0.0", true, false).is_ok());
        // Loopback is fine without a token (only reachable from this machine).
        assert!(check_web_auth_posture("127.0.0.1", false, false).is_ok());
        assert!(check_web_auth_posture("localhost", false, false).is_ok());
        assert!(check_web_auth_posture("::1", false, false).is_ok());
    }

    #[test]
    fn web_auth_posture_allows_public_bind_in_demo_mode() {
        assert!(check_web_auth_posture("0.0.0.0", false, true).is_ok());
        assert!(check_web_auth_posture("::", false, true).is_ok());
    }

    #[test]
    fn demo_mode_posture_rejects_external_write_surfaces() {
        assert!(check_demo_mode_posture(true, false, false).is_ok());
        assert!(check_demo_mode_posture(false, true, true).is_ok());

        let mcp_error = check_demo_mode_posture(true, true, false).expect_err("mcp rejected");
        assert!(mcp_error.contains("HATCHDOOR_MCP_ENABLED"));

        let git_error = check_demo_mode_posture(true, false, true).expect_err("git rejected");
        assert!(git_error.contains("HATCHDOOR_GIT_SYNC_ENABLED"));
    }

    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use std::sync::atomic::Ordering;
    use std::time::Duration;
    use tempfile::TempDir;
    use tokio::sync::RwLock;
    use tokio_stream::StreamExt;
    use tower::ServiceExt;

    use crate::app_state::build_cache;
    use crate::embed::{Embedder, StubEmbedder};

    fn app_for_tests() -> (Router, TempDir) {
        let (app, tmp, _state) = app_for_tests_with_web_auth(None);
        (app, tmp)
    }

    fn app_for_tests_with_web_auth(
        web_bearer_token: Option<Arc<str>>,
    ) -> (Router, TempDir, AppState) {
        app_for_tests_with_web_auth_and_demo_mode(web_bearer_token, false)
    }

    fn app_for_tests_with_web_auth_and_demo_mode(
        web_bearer_token: Option<Arc<str>>,
        demo_mode: bool,
    ) -> (Router, TempDir, AppState) {
        let tmp = TempDir::new().expect("temp dir");
        let vault_root = tmp.path().join("vault");
        std::fs::create_dir_all(&vault_root).expect("create vault");
        std::fs::write(vault_root.join("Home.md"), "# Home\n").expect("write note");
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
        let cache = build_cache(&vault_root, embedder.as_ref()).expect("cache");
        let (vault_events, _) = tokio::sync::broadcast::channel(64);
        let (mcp_tools_changed, _) = tokio::sync::broadcast::channel(16);
        let state = AppState {
            vault_path: vault_root,
            cache: Arc::new(RwLock::new(cache)),
            vault_revision: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            vault_events,
            mcp_tools_changed,
            embedder,
            web_auth_enabled: web_bearer_token.is_some(),
            demo_mode,
            vault_write_lock: Arc::new(tokio::sync::Mutex::new(())),
            git_sync: Arc::new(OnceLock::new()),
            mcp_config: Arc::new(crate::mcp::McpConfig::disabled()),
            archive_prefix: Arc::from("90-archive/"),
            scan_config: Arc::new(crate::vault::VaultScanConfig::default()),
            refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
            startup: StartupTracker::ready(),
        };

        (build_router(state.clone(), web_bearer_token), tmp, state)
    }

    fn app_for_tests_with_state() -> (Router, TempDir, AppState) {
        app_for_tests_with_web_auth(None)
    }

    fn app_for_tests_with_web_and_mcp_auth(
        web_bearer_token: Option<Arc<str>>,
        mcp_bearer_token: Option<String>,
    ) -> (Router, TempDir) {
        let tmp = TempDir::new().expect("temp dir");
        let vault_root = tmp.path().join("vault");
        std::fs::create_dir_all(&vault_root).expect("create vault");
        std::fs::write(vault_root.join("Home.md"), "# Home\n").expect("write note");
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
        let cache = build_cache(&vault_root, embedder.as_ref()).expect("cache");
        let (vault_events, _) = tokio::sync::broadcast::channel(64);
        let (mcp_tools_changed, _) = tokio::sync::broadcast::channel(16);
        let mut mcp_config = crate::mcp::McpConfig::disabled();
        mcp_config.bearer_token = mcp_bearer_token;
        let state = AppState {
            vault_path: vault_root,
            cache: Arc::new(RwLock::new(cache)),
            vault_revision: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            vault_events,
            mcp_tools_changed,
            embedder,
            web_auth_enabled: web_bearer_token.is_some(),
            demo_mode: false,
            vault_write_lock: Arc::new(tokio::sync::Mutex::new(())),
            git_sync: Arc::new(OnceLock::new()),
            mcp_config: Arc::new(mcp_config),
            archive_prefix: Arc::from("90-archive/"),
            scan_config: Arc::new(crate::vault::VaultScanConfig::default()),
            refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
            startup: StartupTracker::ready(),
        };

        (build_router(state, web_bearer_token), tmp)
    }

    fn attachment_upload_request(target_relative_path: &str, token: Option<&str>) -> Request<Body> {
        let boundary = "hatchdoor-test-boundary";
        let body = format!(
            "--{boundary}\r\n\
             Content-Disposition: form-data; name=\"target_relative_path\"\r\n\r\n\
             {target_relative_path}\r\n\
             --{boundary}\r\n\
             Content-Disposition: form-data; name=\"file\"; filename=\"pasted.png\"\r\n\
             Content-Type: image/png\r\n\r\n"
        )
        .into_bytes()
        .into_iter()
        .chain(b"png-bytes".iter().copied())
        .chain(format!("\r\n--{boundary}--\r\n").into_bytes())
        .collect::<Vec<_>>();

        let mut builder = Request::builder()
            .uri("/api/attachment")
            .method("POST")
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            );
        if let Some(token) = token {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        builder.body(Body::from(body)).expect("request")
    }

    #[tokio::test]
    async fn attachment_route_accepts_either_web_or_mcp_token() {
        let (app, _tmp) = app_for_tests_with_web_and_mcp_auth(
            Some(Arc::from("web-secret")),
            Some("mcp-secret".to_string()),
        );

        let no_token = app
            .clone()
            .oneshot(attachment_upload_request("Attachments/no-token.png", None))
            .await
            .expect("response");
        assert_eq!(no_token.status(), StatusCode::UNAUTHORIZED);

        let wrong_token = app
            .clone()
            .oneshot(attachment_upload_request(
                "Attachments/wrong-token.png",
                Some("not-a-real-token"),
            ))
            .await
            .expect("response");
        assert_eq!(wrong_token.status(), StatusCode::UNAUTHORIZED);

        let with_web_token = app
            .clone()
            .oneshot(attachment_upload_request(
                "Attachments/via-web-token.png",
                Some("web-secret"),
            ))
            .await
            .expect("response");
        assert_eq!(with_web_token.status(), StatusCode::OK);

        let with_mcp_token = app
            .oneshot(attachment_upload_request(
                "Attachments/via-mcp-token.png",
                Some("mcp-secret"),
            ))
            .await
            .expect("response");
        assert_eq!(with_mcp_token.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn attachment_route_open_when_no_token_configured() {
        let (app, _tmp) = app_for_tests_with_web_and_mcp_auth(None, None);

        let response = app
            .oneshot(attachment_upload_request("Attachments/open.png", None))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn mcp_route_accepts_body_above_axum_default_limit() {
        // axum's default request-body limit is 2 MiB. A base64-encoded attachment
        // can legitimately exceed that, so the /mcp route must raise its limit to
        // fit the base64 cap; otherwise the framework rejects the upload with 413
        // before the handler runs.
        let (app, _tmp) = app_for_tests();
        let body = "a".repeat(3 * 1024 * 1024);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/mcp")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_ne!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
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
    async fn indexing_startup_keeps_shell_live_but_blocks_vault_surfaces() {
        let (_ready_app, _tmp, state) = app_for_tests_with_state();
        state
            .startup
            .set_indexing(crate::startup::IndexingProgressSnapshot {
                notes_completed: 12,
                notes_total: 40,
                chunks_completed: 18,
                chunks_total: 70,
                tokens_completed: 4_000,
                tokens_total: 20_000,
                elapsed_seconds: 20,
            });
        let app = build_router(state, None);

        for path in ["/health", "/api/startup-status"] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_eq!(response.status(), StatusCode::OK, "{path}");
        }

        let readiness = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/ready")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(readiness.status(), StatusCode::SERVICE_UNAVAILABLE);

        for path in ["/api/tree", "/mcp"] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE, "{path}");
            let body = to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("body");
            let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
            assert_eq!(payload["code"], "vault_indexing");
        }
    }

    #[tokio::test]
    async fn startup_status_exposes_human_progress_and_eta() {
        let (_app, _tmp, state) = app_for_tests_with_state();
        state
            .startup
            .set_indexing(crate::startup::IndexingProgressSnapshot {
                notes_completed: 12,
                notes_total: 40,
                chunks_completed: 18,
                chunks_total: 70,
                tokens_completed: 4_000,
                tokens_total: 20_000,
                elapsed_seconds: 20,
            });
        let response = build_router(state, None)
            .oneshot(
                Request::builder()
                    .uri("/api/startup-status")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["cache-control"], "no-store");
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(payload["state"], "indexing");
        assert_eq!(payload["notes_completed"], 12);
        assert_eq!(payload["notes_total"], 40);
        assert_eq!(payload["percent"], 20);
        assert_eq!(payload["eta_seconds"], 80);
    }

    #[tokio::test]
    async fn ready_endpoint_changes_only_after_startup_completes() {
        let (_app, _tmp, state) = app_for_tests_with_state();
        state.startup.set_scanning();
        let app = build_router(state.clone(), None);
        let before = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/ready")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(before.status(), StatusCode::SERVICE_UNAVAILABLE);

        state.startup.set_ready();
        let after = app
            .oneshot(
                Request::builder()
                    .uri("/ready")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(after.status(), StatusCode::OK);
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
        let (mcp_tools_changed, _) = tokio::sync::broadcast::channel(16);
        let state = AppState {
            vault_path: vault_root,
            cache: Arc::new(RwLock::new(cache)),
            vault_revision: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            vault_events,
            mcp_tools_changed,
            embedder,
            web_auth_enabled: false,
            demo_mode: false,
            vault_write_lock: Arc::new(tokio::sync::Mutex::new(())),
            git_sync: Arc::new(OnceLock::new()),
            mcp_config: Arc::new(crate::mcp::McpConfig::disabled()),
            archive_prefix: Arc::from("90-archive/"),
            scan_config: Arc::new(crate::vault::VaultScanConfig::default()),
            refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
            startup: StartupTracker::ready(),
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
    async fn demo_mode_reports_write_capabilities_disabled() {
        let (app, _tmp, _state) = app_for_tests_with_web_auth_and_demo_mode(None, true);

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
        assert_eq!(payload["enabled"], false);
        assert!(
            payload["warnings"]
                .as_array()
                .expect("warnings")
                .iter()
                .any(|warning| warning.as_str().unwrap_or("").contains("demo mode"))
        );
    }

    #[tokio::test]
    async fn router_reports_write_capabilities_disabled_for_read_only_vault() {
        let (app, tmp, state) = app_for_tests_with_state();
        let vault_path = state.vault_path.clone();
        let original_permissions = std::fs::metadata(&vault_path)
            .expect("vault metadata")
            .permissions();
        let mut read_only_permissions = original_permissions.clone();
        read_only_permissions.set_readonly(true);
        std::fs::set_permissions(&vault_path, read_only_permissions).expect("make vault read-only");

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

        std::fs::set_permissions(&vault_path, original_permissions)
            .expect("restore vault permissions");
        drop(tmp);

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(payload["enabled"], false);
        assert!(
            payload["warnings"]
                .as_array()
                .expect("warnings")
                .iter()
                .any(|warning| warning.as_str().unwrap_or("").contains("not writable"))
        );
        assert!(
            payload["warnings"]
                .as_array()
                .expect("warnings")
                .iter()
                .all(|warning| !warning
                    .as_str()
                    .unwrap_or("")
                    .contains("writes are enabled"))
        );
    }

    #[tokio::test]
    async fn router_returns_bad_request_for_empty_search_query() {
        let (app, _tmp) = app_for_tests();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/search?q=%20%20%20")
                    .method("GET")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(payload["error"], "query cannot be empty");
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
    async fn write_api_rejects_create_to_a_noise_path() {
        // A note written to a built-in noise path (*.tmp) would be indexed away;
        // the HTTP create route must refuse it, matching the MCP write path.
        let (app, tmp) = app_for_tests();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/note")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r##"{"relative_path":"Notes/scratch.tmp","content":"# Ignored\n"}"##,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(!tmp.path().join("vault/Notes/scratch.tmp").exists());
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
    async fn demo_mode_rejects_write_api_before_touching_vault() {
        let (app, tmp, state) = app_for_tests_with_web_auth_and_demo_mode(None, true);
        let blocked_path = state.vault_path.join("Demo Write.md");

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/note")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r##"{"relative_path":"Demo Write.md","content":"# Should not exist\n"}"##,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(!blocked_path.exists());
        drop(tmp);
    }

    #[tokio::test]
    async fn vault_assets_are_served_with_private_cache_control() {
        // Assets must be browser-cacheable (they re-render on every note view)
        // but never shared-cacheable: authenticated deployments put
        // ?access_token= in asset URLs.
        let (app, tmp, state) = app_for_tests_with_web_auth(None);
        std::fs::write(state.vault_path.join("diagram.png"), b"png-bytes").expect("write asset");

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/vault-assets/diagram.png")
                    .method("GET")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(axum::http::header::CACHE_CONTROL)
                .and_then(|value| value.to_str().ok()),
            Some("private, max-age=3600")
        );
        drop(tmp);
    }

    #[tokio::test]
    async fn demo_mode_rejects_refresh_endpoint() {
        // A full reindex re-embeds the whole vault; on an unauthenticated public
        // demo that is a request-loop CPU DoS, so demo mode must refuse it.
        let (app, _tmp, state) = app_for_tests_with_web_auth_and_demo_mode(None, true);
        let revision_before = state
            .vault_revision
            .load(std::sync::atomic::Ordering::SeqCst);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/refresh")
                    .method("POST")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            state
                .vault_revision
                .load(std::sync::atomic::Ordering::SeqCst),
            revision_before,
            "demo refresh must not run a reindex"
        );
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
