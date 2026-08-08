//! HTTP server composition: router construction, startup security-posture
//! checks, and the `serve` run loop. Kept in the library (rather than the binary
//! root) so the HTTP surface is reachable from integration tests.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::Extension;
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
use tracing::{debug, error, info, warn};

use crate::app_state::{AppState, build_cache_with_sqlite_and_progress};
use crate::auth::{WebOrLiveMcpToken, WebToken, require_web_or_live_mcp_token, require_web_token};
use crate::cache::SqliteCache;
use crate::config::AppConfig;
use crate::embed::{Embedder, FastembedEmbedder, RuntimeEmbedder};
use crate::git::{self, GitConfig};
use crate::handlers::{
    MAX_IN_MEMORY_UPLOAD_BYTES, archive_note_handler, create_note_handler, create_vault_handler,
    delete_note_handler, diagnostics_handler, disable_vault_handler, disconnect_vault_handler,
    edit_vault_handler, enable_vault_handler, generate_mcp_token_handler, get_git_status_handler,
    get_index_status_handler, get_settings_handler, graph_handler, health_handler,
    list_vaults_handler, move_note_handler, move_rename_note_handler, note_download_handler,
    note_handler, note_links_handler, patch_settings_handler, recently_modified_handler,
    refresh_handler, rename_note_handler, resolve_batch_handler, resolve_handler,
    retry_vault_handler, reveal_mcp_token_handler, reveal_web_token_handler, search_handler,
    spa_index_handler, stats_handler, sync_vault_handler, tree_handler, update_note_handler,
    upload_attachment_handler, vault_asset_handler, vault_collection_events_handler,
    vault_events_handler, vault_scoped_asset_handler, vault_scoped_note_download_handler,
    vault_scoped_note_handler, vault_scoped_note_links_handler, vault_scoped_resolve_batch_handler,
    vault_scoped_resolve_handler, write_capabilities_handler,
};
use crate::mcp::{McpConfig, mcp_get_handler, mcp_post_handler};
use crate::model_setup::{ModelSetup, SelectedModel};
use crate::runtime_config::{RuntimeConfig, live_settings_defaults, settings_file_path};
use crate::startup::StartupTracker;
use crate::vault_migration::{LegacyMigrationInput, LegacyMigrationOutcome, migrate_legacy_vault};
use crate::vault_registry::{VaultRegistryState, VaultRegistryStore};
use crate::vault_runtime::{
    VaultCollectionRuntime, VaultRuntime, VaultSource, dispatch_managed_git_turn,
};
use crate::vault_watcher::spawn_vault_watcher;
use crate::vault_work::{VaultWorkCoordinator, VaultWorkError, VaultWorkKind};

/// Hosts that only accept connections from the local machine. Binding to any
/// other address exposes the port to the network.
fn is_loopback_host(host: &str) -> bool {
    matches!(host.trim(), "127.0.0.1" | "::1" | "[::1]" | "localhost")
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("install Ctrl-C shutdown signal handler");
    };

    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("install SIGTERM shutdown signal handler");
        tokio::select! {
            () = ctrl_c => {}
            _ = terminate.recv() => {}
        }
    }

    #[cfg(not(unix))]
    ctrl_c.await;
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
        let token = crate::auth::generate_bearer_token()?;
        return Err(format!(
            "HOST={host} is non-loopback but HATCHDOOR_WEB_BEARER_TOKEN is unset: refusing to \
             start unauthenticated on a public interface. Paste this freshly generated token into \
             .env, then restart: HATCHDOOR_WEB_BEARER_TOKEN={token} . Or bind to 127.0.0.1. For \
             a read-only public demo, set HATCHDOOR_DEMO_MODE=true."
        ));
    }
    Ok(())
}

pub fn check_demo_mode_posture(
    demo_mode: bool,
    mcp_enabled: bool,
    git_writes_enabled: bool,
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
    if git_writes_enabled {
        return Err(
            "HATCHDOOR_DEMO_MODE=true is incompatible with Git writeback; disable HATCHDOOR_GIT_SYNC_ENABLED and managed bidirectional mode for public demos."
                .to_string(),
        );
    }
    Ok(())
}

fn initial_model_for_startup(demo_mode: bool, selected: SelectedModel) -> SelectedModel {
    // A public, read-only demo has no person available to accept Gemma's terms.
    // It may use an already-selected model, otherwise use the no-terms Nomic
    // fallback without persisting that choice for a later normal deployment.
    if demo_mode && selected == SelectedModel::TermsRequired {
        SelectedModel::Nomic
    } else {
        selected
    }
}

/// Multipart framing (boundary lines, field headers, the small
/// `target_relative_path` text field) wrapped around the uploaded file. The
/// attachment body limit is the configured max file size plus this slack, so a
/// file right at the advertised cap is not rejected by the framework before the
/// handler's own precise size check runs. Non-attachment routes keep axum's
/// small default limit.
const ATTACHMENT_MULTIPART_OVERHEAD: u64 = 64 * 1024;

pub fn build_router(state: AppState, web_bearer_token: Option<Arc<str>>) -> Router {
    // This is only an outer DoS guard. The upload handler binds the current
    // runtime snapshot and enforces the operator-selected limit precisely.
    let attachment_body_limit = MAX_IN_MEMORY_UPLOAD_BYTES
        .saturating_add(ATTACHMENT_MULTIPART_OVERHEAD)
        .min(usize::MAX as u64) as usize;
    // The base64 import_attachment tool carries file bytes inside the JSON-RPC
    // body, which base64 inflates by ~4/3. Size the /mcp body limit from the
    // base64 cap plus that inflation so a legitimately-sized upload is not
    // rejected before the tool's own decoded-size check runs.
    let mcp_body_limit = MAX_IN_MEMORY_UPLOAD_BYTES
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
        .route("/api/diagnostics", get(diagnostics_handler))
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
    let protected = protected.layer(axum::middleware::from_fn_with_state(
        state.clone(),
        reject_demo_layer_query,
    ));

    // The attachment endpoint sits outside the shared `protected` group: an MCP
    // agent that already holds the MCP bearer token can use it directly,
    // without provisioning the separate web token just for this one route. It
    // still accepts the web token too, since the web UI's paste-to-upload flow
    // hits the same endpoint.
    let attachment = Router::new().route(
        "/api/attachment",
        post(upload_attachment_handler).layer(DefaultBodyLimit::max(attachment_body_limit)),
    );
    let attachment = attachment.layer(axum::middleware::from_fn_with_state(
        WebOrLiveMcpToken {
            web: web_bearer_token.clone(),
            runtime_config: state.runtime_config.clone(),
        },
        require_web_or_live_mcp_token,
    ));
    let attachment = attachment.layer(axum::middleware::from_fn_with_state(
        state.clone(),
        require_vault_ready,
    ));

    let model_setup = Router::new()
        .route("/api/model/accept-gemma", post(accept_gemma_handler))
        .route("/api/model/decline-gemma", post(decline_gemma_handler))
        .route("/api/model/retry", post(retry_model_setup_handler));
    let model_setup = match web_bearer_token.clone() {
        Some(token) => model_setup.layer(axum::middleware::from_fn_with_state(
            WebToken(token),
            require_web_token,
        )),
        None => model_setup,
    };
    let model_setup = model_setup.layer(axum::middleware::from_fn_with_state(
        state.clone(),
        reject_demo_model_setup,
    ));

    // Settings deliberately do not exist in demo mode, rather than existing
    // and refusing writes. They are operator controls, not demo content.
    let settings = if state.demo_mode {
        Router::new()
    } else {
        let settings = Router::new()
            .route(
                "/api/settings",
                get(get_settings_handler).patch(patch_settings_handler),
            )
            .route("/api/index-status", get(get_index_status_handler))
            .route("/api/git-status", get(get_git_status_handler))
            .route(
                "/api/settings/web-token/reveal",
                post(reveal_web_token_handler),
            )
            .route(
                "/api/settings/mcp-token/generate",
                post(generate_mcp_token_handler),
            )
            .route(
                "/api/settings/mcp-token/reveal",
                post(reveal_mcp_token_handler),
            )
            .layer(Extension(web_bearer_token.clone()));
        match web_bearer_token.clone() {
            Some(token) => settings.layer(axum::middleware::from_fn_with_state(
                WebToken(token),
                require_web_token,
            )),
            None => settings,
        }
    };

    // MCP remains reachable during initial setup. It advertises the full stable
    // tool list, while tools::dispatch blocks vault operations until ready.
    let mcp = Router::new()
        .route("/mcp", get(mcp_get_handler).post(mcp_post_handler))
        .layer(DefaultBodyLimit::max(mcp_body_limit));

    // Vault-collection discovery, management, events, and exact content reads
    // are deliberately not gated by `require_vault_ready`: that middleware
    // guards the legacy single-configured-Vault readiness signal, while
    // connecting the first Vault and recovering a corrupt registry must stay
    // reachable at zero enabled Vaults. Exact reads gate per-request on their
    // own targeted Vault instead (`vault_not_found`/`vault_disabled`/
    // `vault_unavailable` from `VaultReadCore`), which is the per-Vault
    // equivalent this surface needs. Demo-mode posture mirrors settings and
    // the collection-management routes above: a demo deployment serves its
    // single legacy vault over the existing unscoped routes, unaffected by
    // this exclusion.
    let vaults_v1 = if state.demo_mode {
        Router::new()
    } else {
        let vaults_v1 = Router::new()
            .route(
                "/api/v1/vaults",
                get(list_vaults_handler).post(create_vault_handler),
            )
            .route(
                "/api/v1/vaults/events",
                get(vault_collection_events_handler),
            )
            .route(
                "/api/v1/vaults/{vault_id}",
                patch(edit_vault_handler).delete(disconnect_vault_handler),
            )
            .route(
                "/api/v1/vaults/{vault_id}/enable",
                post(enable_vault_handler),
            )
            .route(
                "/api/v1/vaults/{vault_id}/disable",
                post(disable_vault_handler),
            )
            .route("/api/v1/vaults/{vault_id}/sync", post(sync_vault_handler))
            .route("/api/v1/vaults/{vault_id}/retry", post(retry_vault_handler))
            .route(
                "/api/v1/vaults/{vault_id}/notes/{slug}",
                get(vault_scoped_note_handler),
            )
            .route(
                "/api/v1/vaults/{vault_id}/notes/{slug}/links",
                get(vault_scoped_note_links_handler),
            )
            .route(
                "/api/v1/vaults/{vault_id}/notes/{slug}/download",
                get(vault_scoped_note_download_handler),
            )
            .route(
                "/api/v1/vaults/{vault_id}/resolve",
                get(vault_scoped_resolve_handler),
            )
            .route(
                "/api/v1/vaults/{vault_id}/resolve-batch",
                post(vault_scoped_resolve_batch_handler),
            )
            .route(
                "/api/v1/vaults/{vault_id}/assets/{*path}",
                get(vault_scoped_asset_handler),
            );
        match web_bearer_token.clone() {
            Some(token) => vaults_v1.layer(axum::middleware::from_fn_with_state(
                WebToken(token),
                require_web_token,
            )),
            None => vaults_v1,
        }
    };

    Router::new()
        .route("/health", get(health_handler))
        .route("/ready", get(readiness_handler))
        .route("/api/startup-status", get(startup_status_handler))
        .route("/api/vault-status", get(vault_status_handler))
        .merge(model_setup)
        .merge(settings)
        .merge(vaults_v1)
        .merge(mcp)
        .merge(protected)
        .merge(attachment)
        .route("/", get(spa_index_handler))
        .route("/n/{slug}", get(spa_index_handler))
        // Canonical Vault-qualified browser Note URL (issue #62): unambiguous
        // when multiple Vaults contain the same slug. Frontend consumption is
        // #67; the server only needs to serve the SPA shell for it, mirroring
        // the existing unscoped `/n/{slug}` registration.
        .route("/v/{vault_id}/n/{slug}", get(spa_index_handler))
        .route("/stats", get(spa_index_handler))
        .route("/graph", get(spa_index_handler))
        .route("/settings", get(spa_index_handler))
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

async fn vault_status_handler(State(state): State<AppState>) -> Response {
    let mut response = Json(state.startup.snapshot()).into_response();
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

async fn accept_gemma_handler(State(state): State<AppState>) -> Response {
    match state.model_setup.accept_gemma() {
        Ok(()) => {
            spawn_model_startup(state, SelectedModel::Gemma);
            StatusCode::ACCEPTED.into_response()
        }
        Err(error) => model_setup_error(error),
    }
}

async fn decline_gemma_handler(State(state): State<AppState>) -> Response {
    match state.model_setup.decline_gemma() {
        Ok(()) => {
            spawn_model_startup(state, SelectedModel::Nomic);
            StatusCode::ACCEPTED.into_response()
        }
        Err(error) => model_setup_error(error),
    }
}

async fn retry_model_setup_handler(State(state): State<AppState>) -> Response {
    match state.model_setup.selected() {
        Ok(selected @ (SelectedModel::Gemma | SelectedModel::Nomic)) => {
            spawn_model_startup(state, selected);
            StatusCode::ACCEPTED.into_response()
        }
        Ok(SelectedModel::TermsRequired) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": "Gemma terms must be accepted or declined first." })),
        )
            .into_response(),
        Err(error) => model_setup_error(error),
    }
}

fn model_setup_error(error: String) -> Response {
    error!("Model setup error: {error}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({ "error": "Model setup could not be started." })),
    )
        .into_response()
}

pub(crate) fn spawn_model_startup(state: AppState, selected: SelectedModel) {
    if state.model_setup_started.swap(true, Ordering::AcqRel) {
        return;
    }
    let tracker = state.startup.clone();
    let model_name = selected.id().unwrap_or("search model");
    tracker.set_downloading(model_name, None, None);
    info!(
        model = model_name,
        "Downloading and loading startup embedding model"
    );

    tokio::spawn(async move {
        let model_dir = state.model_setup.model_cache_dir(selected);
        let runtime = state.runtime_embedder.clone();
        let model_setup = state.model_setup.clone();
        let download_tracker = tracker.clone();
        let load_result = tokio::task::spawn_blocking(move || -> Result<(), String> {
            model_setup.prepare_download(selected)?;
            for attempt in 0..=1 {
                let loaded = (|| -> Result<Arc<dyn Embedder>, String> {
                    if selected == SelectedModel::Gemma {
                        let progress = {
                            let download_tracker = download_tracker.clone();
                            Arc::new(move |downloaded, total| {
                                download_tracker.set_downloading(
                                    model_name,
                                    Some(downloaded),
                                    Some(total),
                                );
                            })
                        };
                        model_setup.fetch_gemma_at_pinned_revision(progress)?;
                    }
                    match selected {
                        SelectedModel::Gemma => Ok(Arc::new(
                            FastembedEmbedder::embedding_gemma_300m_q4_in(model_dir.clone())?,
                        )),
                        SelectedModel::Nomic => Ok(Arc::new(FastembedEmbedder::nomic_v1_5_in(
                            model_dir.clone(),
                        )?)),
                        SelectedModel::TermsRequired => {
                            Err("model terms have not been accepted".to_string())
                        }
                    }
                })();
                match loaded.and_then(|embedder| {
                    model_setup.record_integrity(selected)?;
                    Ok(embedder)
                }) {
                    Ok(embedder) => {
                        runtime.set(embedder, selected == SelectedModel::Gemma);
                        return Ok(());
                    }
                    Err(error) if attempt == 0 => {
                        warn!(
                            model = model_name,
                            "Model setup attempt failed; retrying once: {error}"
                        );
                        model_setup.reset_download_cache(selected)?;
                    }
                    Err(error) => return Err(error),
                }
            }
            unreachable!("the retry loop always returns")
        })
        .await;

        match load_result {
            Ok(Ok(())) => {
                tracker.set_scanning();
                let progress_tracker = tracker.clone();
                let on_progress = Arc::new(move |progress| progress_tracker.set_indexing(progress));
                let Some(vault_path) = state.configured_local_vault_path() else {
                    state.model_setup_started.store(false, Ordering::Release);
                    state.startup.runtime().set_unavailable(
                        "managed_vault_not_acquired",
                        "Managed Git vault acquisition is not implemented in this foundation slice.",
                    );
                    return;
                };
                let indexing_vault_path = vault_path.clone();
                let indexing_sqlite = state.startup_sqlite.clone();
                let indexing_embedder = state.embedder.clone();
                let index_state = state.clone();
                let index_result = tokio::task::spawn_blocking(move || {
                    let runtime_snapshot = index_state.runtime_config.snapshot();
                    let embed_layers = runtime_snapshot
                        .setting("HATCHDOOR_EMBED_LAYERS")
                        .map(|setting| crate::runtime_config::is_truthy(&setting.value))
                        .unwrap_or(true);
                    let scan_config = index_state.live_scan_config()?;
                    build_cache_with_sqlite_and_progress(
                        &indexing_vault_path,
                        indexing_sqlite,
                        indexing_embedder.as_ref(),
                        Some(on_progress),
                        &scan_config,
                        embed_layers,
                    )
                })
                .await;
                match index_result {
                    Ok(Ok(cache)) => {
                        state.publish_ready_vault(vault_path.clone(), cache).await;
                        tracker.set_ready();
                        info!(
                            model = model_name,
                            "Model setup and vault indexing complete"
                        );
                        let git_config = GitConfig::from_snapshot(
                            vault_path.clone(),
                            &state.runtime_snapshot(),
                        )
                        .unwrap_or_else(|error| {
                            warn!("Git versioning configuration changed before startup: {error}");
                            None
                        });
                        let mut active_git_sync = state.git_sync.write().await;
                        if active_git_sync.is_none()
                            && let Some(git_config) = git_config
                        {
                            let handle = git::spawn_sync_task(
                                git_config,
                                state.vault_write_lock.clone(),
                                git::SyncOps {
                                    commit: Box::new(git::commit_local),
                                    fetch: Box::new(git::fetch_remote),
                                    integrate: Box::new(git::integrate_fetched),
                                    push: Box::new(git::push_branch),
                                },
                            );
                            *active_git_sync = Some(handle);
                            info!("Git sync enabled");
                        }
                        spawn_vault_watcher(
                            state.clone(),
                            vault_path.clone(),
                            state.cache_db_path.clone(),
                        );
                    }
                    Ok(Err(error)) => {
                        state.model_setup_started.store(false, Ordering::Release);
                        tracker.set_failed();
                        error!("Failed to index vault after model setup: {error}");
                        spawn_vault_watcher(state.clone(), vault_path, state.cache_db_path.clone());
                    }
                    Err(error) => {
                        state.model_setup_started.store(false, Ordering::Release);
                        tracker.set_failed();
                        error!("Vault indexing task failed: {error}");
                    }
                }
            }
            Ok(Err(error)) => {
                state.model_setup_started.store(false, Ordering::Release);
                tracker.set_model_setup_failed();
                error!(model = model_name, "Model download/load failed: {error}");
            }
            Err(error) => {
                state.model_setup_started.store(false, Ordering::Release);
                tracker.set_model_setup_failed();
                error!(model = model_name, "Model setup task failed: {error}");
            }
        }
    });
}

async fn require_vault_ready(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    if state.startup.is_ready() {
        return next.run(request).await;
    }
    let snapshot = state.startup.snapshot();
    let (error, code) = match snapshot.phase {
        crate::vault_runtime::VaultPhase::Unavailable => (
            "The configured vault is unavailable",
            snapshot
                .error
                .as_ref()
                .map(|error| error.code.as_str())
                .unwrap_or("vault_unavailable"),
        ),
        _ => ("Vault is still being indexed", "vault_indexing"),
    };
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(serde_json::json!({
            "error": error,
            "code": code
        })),
    )
        .into_response()
}

async fn reject_demo_model_setup(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    if state.demo_mode {
        return StatusCode::NOT_FOUND.into_response();
    }
    next.run(request).await
}

/// The HTTP surface is intentionally default-only. In demo mode reject an
/// attempted layer selector explicitly rather than silently ignoring it, which
/// could make a caller believe demoted data was being queried safely.
async fn reject_demo_layer_query(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let requests_layers = request.uri().query().is_some_and(|query| {
        query
            .split('&')
            .filter_map(|part| part.split_once('=').map(|(key, _)| key).or(Some(part)))
            .any(|key| key == "layers")
    });
    if state.demo_mode && requests_layers {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "layers is unavailable in demo mode"
            })),
        )
            .into_response();
    }
    next.run(request).await
}

pub async fn run_server() {
    let mut config = AppConfig::from_env().unwrap_or_else(|e| {
        error!("Configuration error: {e}");
        std::process::exit(1);
    });

    let settings_file_override = std::env::var("HATCHDOOR_SETTINGS_FILE").ok();
    let runtime_config = RuntimeConfig::load_from_process(
        settings_file_path(&config.cache_db_path, settings_file_override.as_deref()),
        live_settings_defaults(),
    )
    .unwrap_or_else(|e| {
        error!("Settings configuration error: {e}");
        std::process::exit(1);
    });
    let startup_snapshot = runtime_config.snapshot();
    // Stated only when it is true of this instance: on a boot with nothing
    // pinned the sentence describes nothing, and running pinned is a
    // legitimate posture rather than something to warn about every start.
    let pinned_count = startup_snapshot.pinned_count();
    if pinned_count > 0 {
        info!(
            pinned_count,
            "{pinned_count} settings are pinned by environment variables and cannot be changed from the settings page; edit .env and restart to change them"
        );
    }
    config
        .apply_runtime_snapshot(&startup_snapshot)
        .unwrap_or_else(|e| {
            error!("Application settings configuration error: {e}");
            std::process::exit(1);
        });

    let mcp_config = McpConfig::from_snapshot(&startup_snapshot)
        .and_then(|config| {
            config.validate()?;
            Ok(config)
        })
        .unwrap_or_else(|e| {
            error!("MCP configuration error: {e}");
            std::process::exit(1);
        });

    if let Err(message) = check_web_auth_posture(
        &config.host,
        config.web_bearer_token.is_some(),
        config.demo_mode,
    ) {
        error!("{message}");
        std::process::exit(1);
    }

    let git_sync_config = match &config.vault_source {
        VaultSource::Local { vault_path } => {
            GitConfig::from_snapshot(vault_path.clone(), &startup_snapshot)
        }
        VaultSource::ManagedGit(_) => {
            let legacy_mode_enabled = startup_snapshot
                .setting("HATCHDOOR_GIT_SYNC_ENABLED")
                .is_some_and(|setting| {
                    !matches!(
                        setting.value.trim().to_ascii_lowercase().as_str(),
                        "" | "off" | "false" | "0" | "no"
                    )
                });
            if legacy_mode_enabled {
                Err("HATCHDOOR_GIT_SYNC_ENABLED cannot be combined with HATCHDOOR_VAULT_SOURCE=git; managed source mode owns Git synchronization".to_string())
            } else {
                Ok(None)
            }
        }
    }
    .unwrap_or_else(|e| {
        error!("Git sync configuration error: {e}");
        std::process::exit(1);
    });

    let managed_writeback = matches!(
        &config.vault_source,
        VaultSource::ManagedGit(source)
            if source.mode == crate::vault_runtime::ManagedGitMode::Bidirectional
    );
    if let Err(message) = check_demo_mode_posture(
        config.demo_mode,
        mcp_config.enabled,
        git_sync_config.is_some() || managed_writeback,
    ) {
        error!("{message}");
        std::process::exit(1);
    }

    // Migration may persist the registry and discard a recognized legacy
    // cache, so run it only after startup security/configuration refusals and
    // before opening SQLite.
    let vault_registry = VaultRegistryStore::at_default_path();
    let legacy_vault_path = match &config.vault_source {
        VaultSource::Local { vault_path } => vault_path.clone(),
        VaultSource::ManagedGit(_) => std::env::var("VAULT_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("./vault")),
    };
    let migration = migrate_legacy_vault(
        &vault_registry,
        &runtime_config,
        LegacyMigrationInput {
            vault_path: legacy_vault_path,
            cache_db_path: config.cache_db_path.clone(),
            environment: std::env::vars().collect(),
        },
    )
    .unwrap_or_else(|error| {
        error!("Legacy Vault migration failed: {error}");
        std::process::exit(1);
    });
    let (registry_state, legacy_migration_recovery) = match migration {
        LegacyMigrationOutcome::NoLegacyDeployment => (
            vault_registry.load().unwrap_or_else(|error| {
                error!("Vault registry startup failed: {error}");
                std::process::exit(1);
            }),
            None,
        ),
        LegacyMigrationOutcome::ExistingRegistry {
            state,
            ignored_environment_keys,
        } => {
            if !ignored_environment_keys.is_empty() {
                warn!(
                    keys = ?ignored_environment_keys,
                    "Ignoring legacy environment Vault settings because the registry already exists"
                );
            }
            (state, None)
        }
        LegacyMigrationOutcome::Imported {
            snapshot,
            vault_id,
            cleanup_warnings,
            ignored_environment_keys,
        } => {
            info!(%vault_id, "Imported legacy deployment into the Vault registry");
            for warning in cleanup_warnings {
                warn!("{warning}");
            }
            if !ignored_environment_keys.is_empty() {
                warn!(keys = ?ignored_environment_keys, "Legacy environment Vault settings were not persisted");
            }
            (VaultRegistryState::Ready(snapshot), None)
        }
        LegacyMigrationOutcome::Recovery {
            recovery,
            ignored_environment_keys,
        } => {
            warn!(
                code = recovery.code(),
                message = recovery.message(),
                keys = ?ignored_environment_keys,
                "Legacy Vault migration requires operator recovery"
            );
            (
                vault_registry.load().unwrap_or_else(|error| {
                    error!("Vault registry startup failed: {error}");
                    std::process::exit(1);
                }),
                Some(recovery),
            )
        }
    };

    let sqlite = Arc::new(
        SqliteCache::open(&config.cache_db_path, 768).unwrap_or_else(|e| {
            error!(
                cache_db_path = %config.cache_db_path.display(),
                "SQLite cache startup failed: {e}"
            );
            std::process::exit(1);
        }),
    );

    let model_setup = Arc::new(ModelSetup::new(ModelSetup::default_models_dir()));
    let selected_model = model_setup.selected().unwrap_or_else(|error| {
        error!("Model setup state is invalid: {error}");
        std::process::exit(1);
    });
    let selected_model = initial_model_for_startup(config.demo_mode, selected_model);
    let runtime_embedder = Arc::new(RuntimeEmbedder::new());
    let embedder: Arc<dyn Embedder> = runtime_embedder.clone();

    let scan_config = AppState::runtime_scan_config(&startup_snapshot).unwrap_or_else(|e| {
        error!("Invalid HATCHDOOR_EXCLUDE configuration: {e}");
        std::process::exit(1);
    });
    for (pattern, source) in scan_config.exclude.configured_patterns() {
        info!(pattern = %pattern, source, "Noise-exclusion pattern active");
    }
    info!(
        embed_layers = config.embed_layers,
        "Demoted-layer vector embedding (HATCHDOOR_EMBED_LAYERS)"
    );

    let vault_write_lock = Arc::new(tokio::sync::Mutex::new(()));
    if let Some(git_config) = &git_sync_config {
        let validation = match git_config.mode {
            crate::git::GitMode::Local => git::validate_local_repo(git_config),
            crate::git::GitMode::Remote => git::validate_repo(git_config),
        };
        if let Err(error) = validation {
            error!("Git versioning configuration invalid: {error}");
            std::process::exit(1);
        }
    }

    let (vault_events, _) = tokio::sync::broadcast::channel(64);
    let (mcp_tools_changed, _) = tokio::sync::broadcast::channel(16);
    let vaults = VaultCollectionRuntime::with_watching(config.cache_db_path.clone());
    // #90 establishes durable reconstruction and lifecycle admission. #97
    // owns the concrete worker loop and Git operation dispatch, spawned
    // below once `vault_work`/`managed_git` and the collection/registry
    // handles they dispatch against all exist. Index/Repair dispatch remains
    // for a later cache/repair packet.
    let (vault_work, vault_worker) = VaultWorkCoordinator::new();
    let managed_git = Arc::new(crate::git::ManagedGitScheduler::new(vault_work.clone()));
    let git_author_name =
        crate::git::config::non_empty_setting(&startup_snapshot, "HATCHDOOR_GIT_AUTHOR_NAME")
            .unwrap_or_else(|| "Hatchdoor".to_string());
    let git_author_email =
        crate::git::config::non_empty_setting(&startup_snapshot, "HATCHDOOR_GIT_AUTHOR_EMAIL")
            .unwrap_or_else(|| "hatchdoor@localhost".to_string());
    match &registry_state {
        VaultRegistryState::Ready(snapshot) => {
            vaults
                .reconcile_and_reconstruct(&vault_registry, snapshot, &vault_work, &managed_git)
                .await
        }
        VaultRegistryState::Recovery(recovery) => warn!(
            message = recovery.message(),
            "Vault registry requires operator recovery; no Vault runtimes were activated"
        ),
    }
    let runtime = VaultRuntime::new(config.vault_source.clone());
    let startup = StartupTracker::new(runtime);
    if matches!(config.vault_source, VaultSource::Local { .. }) {
        if selected_model == SelectedModel::TermsRequired {
            startup.set_terms_required();
        } else {
            startup.set_scanning();
        }
    }
    let shutdown_vaults = vaults.clone();
    let state = AppState {
        cache_db_path: config.cache_db_path.clone(),
        vault_registry,
        vaults,
        vault_work: vault_work.clone(),
        managed_git: managed_git.clone(),
        legacy_migration_recovery,
        startup_sqlite: sqlite.clone(),
        ready_vault: Arc::new(RwLock::new(None)),
        vault_revision: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        vault_events,
        mcp_tools_changed,
        embedder,
        runtime_embedder,
        model_setup,
        model_setup_started: Arc::new(AtomicBool::new(false)),
        startup_git_config: Arc::new(git_sync_config.clone()),
        web_auth_enabled: config.web_bearer_token.is_some(),
        demo_mode: config.demo_mode,
        vault_write_lock,
        git_sync: Arc::new(RwLock::new(None)),
        scan_config_cache: Arc::new(std::sync::RwLock::new(Some((
            startup_snapshot.clone(),
            scan_config.clone(),
        )))),
        refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
        index_status: crate::app_state::IndexStatusTracker::up_to_date(),
        runtime_config,
        startup,
    };

    let web_bearer_token = config.web_bearer_token.clone().map(Arc::from);
    let app = build_router(state.clone(), web_bearer_token);
    let shutdown_state = state.clone();
    let (shutdown_started, mut shutdown_received) = tokio::sync::watch::channel(false);
    let shutdown_task = tokio::spawn({
        let vault_work = vault_work.clone();
        async move {
            shutdown_signal().await;
            vault_work.shutdown();
            shutdown_started.send_replace(true);
        }
    });

    // The one global consumer of `vault_work`/`vault_worker`: dispatches
    // `Git` turns through #97's managed-Git scheduler and marks `Index`/
    // `Repair` explicitly not-yet-implemented so they release a Vault's
    // single FIFO position instead of blocking a later Git turn behind a
    // request nobody drains (a later cache/repair packet supplies those).
    // Exits on its own once `vault_work.shutdown()` drains to quiescence.
    let dispatch_task = tokio::spawn({
        let mut vault_worker = vault_worker;
        let dispatch_vaults = state.vaults.clone();
        let dispatch_registry = state.vault_registry.clone();
        let dispatch_managed_git = managed_git.clone();
        async move {
            while let Some(outcome) = vault_worker
                .run_next(|request| {
                    let vaults = dispatch_vaults.clone();
                    let registry = dispatch_registry.clone();
                    let managed_git = dispatch_managed_git.clone();
                    let author_name = git_author_name.clone();
                    let author_email = git_author_email.clone();
                    async move {
                        match request.kind() {
                            VaultWorkKind::Git => {
                                dispatch_managed_git_turn(
                                    &vaults,
                                    &registry,
                                    &managed_git,
                                    &author_name,
                                    &author_email,
                                    request,
                                )
                                .await
                            }
                            VaultWorkKind::Index | VaultWorkKind::Repair => {
                                Err(VaultWorkError::new(
                                    "vault_work_kind_not_yet_implemented",
                                    format!("{:?} dispatch is not implemented yet", request.kind()),
                                    false,
                                ))
                            }
                        }
                    }
                })
                .await
            {
                if let Err(error) = outcome.result {
                    // Expected and permanent until a later cache/repair
                    // packet lands (see the `Index`/`Repair` arm above); log
                    // it quietly rather than as a recurring warning on every
                    // Vault activation.
                    if error.code() == "vault_work_kind_not_yet_implemented" {
                        debug!(
                            vault_id = %outcome.request.vault_id(),
                            kind = ?outcome.request.kind(),
                            "Vault background work kind not yet implemented"
                        );
                    } else {
                        warn!(
                            vault_id = %outcome.request.vault_id(),
                            kind = ?outcome.request.kind(),
                            code = error.code(),
                            message = error.message(),
                            "Vault background work turn failed"
                        );
                    }
                }
            }
        }
    });
    let scheduler_tick_task =
        crate::git::spawn_scheduler_tick(managed_git.clone(), crate::git::DEFAULT_TICK_INTERVAL);

    let addr = config.socket_addr().unwrap_or_else(|e| {
        error!("Address error: {e}");
        std::process::exit(1);
    });

    info!(
        host = %config.host,
        port = config.port,
        vault_source = ?config.vault_source.kind(),
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

    match config.vault_source {
        VaultSource::Local { .. } if selected_model != SelectedModel::TermsRequired => {
            spawn_model_startup(state.clone(), selected_model);
        }
        VaultSource::ManagedGit(_) => {
            state.startup.runtime().set_unavailable(
                "managed_vault_not_acquired",
                "Managed Git vault acquisition is not implemented in this foundation slice.",
            );
        }
        VaultSource::Local { .. } => {}
    }

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = shutdown_received.changed().await;
        })
        .await
        .unwrap_or_else(|e| {
            error!("Server error: {e}");
            std::process::exit(1);
        });

    shutdown_vaults.shutdown(&vault_work).await;
    // The scheduler's own timer has nothing left to protect once the
    // coordinator has stopped accepting work; the dispatch loop drains and
    // exits on its own now that `shutdown()` above reached quiescence.
    scheduler_tick_task.abort();
    if let Err(error) = dispatch_task.await {
        error!(%error, "Vault background work dispatch loop exited unexpectedly");
    }
    if let Some(git_sync) = shutdown_state.git_sync.read().await.clone()
        && let Err(error) = git_sync.stop(std::time::Duration::from_secs(30)).await
    {
        error!(%error, "Git sync did not reach its shutdown boundary");
    }
    if let Err(error) = shutdown_task.await {
        error!(%error, "Server shutdown task exited unexpectedly");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::ReadyVault;

    #[test]
    fn web_auth_posture_refuses_public_bind_without_token() {
        // Non-loopback host with no web token must refuse to start.
        for host in ["0.0.0.0", "192.168.1.50", "::"] {
            let error =
                check_web_auth_posture(host, false, false).expect_err("public bind rejected");

            assert!(error.contains(&format!("HOST={host}")));
            assert!(error.contains("HATCHDOOR_WEB_BEARER_TOKEN="));
            assert!(error.contains("Paste this freshly generated token into .env"));

            let generated_token = error
                .split("HATCHDOOR_WEB_BEARER_TOKEN=")
                .nth(1)
                .and_then(|value| value.split_whitespace().next())
                .expect("generated token in .env assignment");
            assert_eq!(generated_token.len(), 43);
            assert!(
                generated_token
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
            );
        }
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

    #[test]
    fn demo_mode_uses_nomic_when_gemma_terms_have_not_been_accepted() {
        assert_eq!(
            initial_model_for_startup(true, SelectedModel::TermsRequired),
            SelectedModel::Nomic
        );
        assert_eq!(
            initial_model_for_startup(true, SelectedModel::Gemma),
            SelectedModel::Gemma
        );
        assert_eq!(
            initial_model_for_startup(false, SelectedModel::TermsRequired),
            SelectedModel::TermsRequired
        );
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
        // These settings tests exercise the explicit fresh-repository consent
        // flow. Keep the fixture outside Cargo's TMPDIR: the project build
        // directory may sit inside Hatchdoor's own Git checkout, which would
        // intentionally count as an enclosing existing repository for Local
        // history.
        let tmp = TempDir::new_in("/tmp").expect("temp dir");
        let vault_root = tmp.path().join("vault");
        std::fs::create_dir_all(&vault_root).expect("create vault");
        std::fs::write(vault_root.join("Home.md"), "# Home\n").expect("write note");
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
        let cache = build_cache(&vault_root, embedder.as_ref()).expect("cache");
        let (vault_events, _) = tokio::sync::broadcast::channel(64);
        let (mcp_tools_changed, _) = tokio::sync::broadcast::channel(16);
        let (vault_work, _vault_worker) = crate::vault_work::VaultWorkCoordinator::new();
        let managed_git =
            std::sync::Arc::new(crate::git::ManagedGitScheduler::new(vault_work.clone()));
        let state = AppState {
            cache_db_path: tmp.path().join("cache.sqlite3"),
            vault_registry: VaultRegistryStore::new(tmp.path().join("state/vaults.json")),
            vaults: VaultCollectionRuntime::new(),
            vault_work,
            managed_git,
            legacy_migration_recovery: None,
            startup_sqlite: cache.sqlite.clone(),
            ready_vault: Arc::new(RwLock::new(Some(ReadyVault {
                vault_path: vault_root,
                cache,
            }))),
            vault_revision: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            vault_events,
            mcp_tools_changed,
            embedder,
            runtime_embedder: Arc::new(RuntimeEmbedder::new()),
            model_setup: Arc::new(ModelSetup::new(tmp.path().join("models"))),
            model_setup_started: Arc::new(AtomicBool::new(true)),
            startup_git_config: Arc::new(None),
            web_auth_enabled: web_bearer_token.is_some(),
            demo_mode,
            vault_write_lock: Arc::new(tokio::sync::Mutex::new(())),
            git_sync: Arc::new(RwLock::new(None)),
            scan_config_cache: Arc::new(std::sync::RwLock::new(None)),
            refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
            index_status: crate::app_state::IndexStatusTracker::up_to_date(),
            runtime_config: crate::runtime_config::RuntimeConfig::for_tests(),
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
        app_for_tests_with_web_and_mcp_auth_and_write_mode(web_bearer_token, mcp_bearer_token, true)
    }

    fn app_for_tests_with_web_and_mcp_auth_and_write_mode(
        web_bearer_token: Option<Arc<str>>,
        mcp_bearer_token: Option<String>,
        mcp_write_enabled: bool,
    ) -> (Router, TempDir) {
        let tmp = TempDir::new().expect("temp dir");
        let vault_root = tmp.path().join("vault");
        std::fs::create_dir_all(&vault_root).expect("create vault");
        std::fs::write(vault_root.join("Home.md"), "# Home\n").expect("write note");
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
        let cache = build_cache(&vault_root, embedder.as_ref()).expect("cache");
        let (vault_events, _) = tokio::sync::broadcast::channel(64);
        let (mcp_tools_changed, _) = tokio::sync::broadcast::channel(16);
        let runtime_config = crate::runtime_config::RuntimeConfig::for_tests();
        if let Some(mcp_bearer_token) = mcp_bearer_token {
            runtime_config
                .save([
                    ("HATCHDOOR_MCP_ENABLED".to_string(), "true".to_string()),
                    (
                        "HATCHDOOR_MCP_WRITE_ENABLED".to_string(),
                        mcp_write_enabled.to_string(),
                    ),
                    ("HATCHDOOR_MCP_BEARER_TOKEN".to_string(), mcp_bearer_token),
                ])
                .expect("save MCP token");
        }
        let (vault_work, _vault_worker) = crate::vault_work::VaultWorkCoordinator::new();
        let managed_git =
            std::sync::Arc::new(crate::git::ManagedGitScheduler::new(vault_work.clone()));
        let state = AppState {
            cache_db_path: tmp.path().join("cache.sqlite3"),
            vault_registry: VaultRegistryStore::new(tmp.path().join("state/vaults.json")),
            vaults: VaultCollectionRuntime::new(),
            vault_work,
            managed_git,
            legacy_migration_recovery: None,
            startup_sqlite: cache.sqlite.clone(),
            ready_vault: Arc::new(RwLock::new(Some(ReadyVault {
                vault_path: vault_root,
                cache,
            }))),
            vault_revision: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            vault_events,
            mcp_tools_changed,
            embedder,
            runtime_embedder: Arc::new(RuntimeEmbedder::new()),
            model_setup: Arc::new(ModelSetup::new(tmp.path().join("models"))),
            model_setup_started: Arc::new(AtomicBool::new(true)),
            startup_git_config: Arc::new(None),
            web_auth_enabled: web_bearer_token.is_some(),
            demo_mode: false,
            vault_write_lock: Arc::new(tokio::sync::Mutex::new(())),
            git_sync: Arc::new(RwLock::new(None)),
            scan_config_cache: Arc::new(std::sync::RwLock::new(None)),
            refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
            index_status: crate::app_state::IndexStatusTracker::up_to_date(),
            runtime_config,
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
    async fn attachment_route_accepts_mcp_token_with_mcp_write_disabled() {
        // S9 regression: the attachment route's token check must not gate on
        // `mcp.enabled && mcp.write_enabled` — issue #60 only asked to read the
        // token per-request and mount the check unconditionally. A token that
        // worked for `/api/attachment` while MCP write mode is off must keep
        // working.
        let (app, _tmp) = app_for_tests_with_web_and_mcp_auth_and_write_mode(
            Some(Arc::from("web-secret")),
            Some("mcp-secret".to_string()),
            false,
        );

        let response = app
            .oneshot(attachment_upload_request(
                "Attachments/via-mcp-token.png",
                Some("mcp-secret"),
            ))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
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
    async fn mcp_settings_apply_atomically_and_rotate_attachment_authorization() {
        let (app, _tmp, state) = app_for_tests_with_state();

        let invalid = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/settings")
                    .method("PATCH")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"updates":{"HATCHDOOR_MCP_ENABLED":"true"}}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            state
                .runtime_snapshot()
                .setting("HATCHDOOR_MCP_ENABLED")
                .expect("setting")
                .value,
            "false"
        );

        let enabled = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/settings")
                    .method("PATCH")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"updates":{"HATCHDOOR_MCP_ENABLED":"true","HATCHDOOR_MCP_WRITE_ENABLED":"true","HATCHDOOR_MCP_BEARER_TOKEN":"first-token"}}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(enabled.status(), StatusCode::OK);

        let mcp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/mcp")
                    .method("POST")
                    .header("authorization", "Bearer first-token")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(mcp.status(), StatusCode::OK);

        let forbidden_origin = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/mcp")
                    .method("POST")
                    .header("authorization", "Bearer first-token")
                    .header("origin", "https://evil.example")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(forbidden_origin.status(), StatusCode::FORBIDDEN);

        let rejected = app
            .clone()
            .oneshot(attachment_upload_request("Attachments/rejected.png", None))
            .await
            .expect("response");
        assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);

        let first_token = app
            .clone()
            .oneshot(attachment_upload_request(
                "Attachments/first-token.png",
                Some("first-token"),
            ))
            .await
            .expect("response");
        assert_eq!(first_token.status(), StatusCode::OK);

        let read_only = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/settings")
                    .method("PATCH")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"updates":{"HATCHDOOR_MCP_WRITE_ENABLED":"false"}}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(read_only.status(), StatusCode::OK);

        // S9: the attachment route's token check does not gate on
        // `mcp.write_enabled` — a token that worked while MCP write mode was
        // on must keep working with write mode off.
        let read_only_attachment = app
            .clone()
            .oneshot(attachment_upload_request(
                "Attachments/read-only.png",
                Some("first-token"),
            ))
            .await
            .expect("response");
        assert_eq!(read_only_attachment.status(), StatusCode::OK);

        let limits = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/settings")
                    .method("PATCH")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"updates":{"HATCHDOOR_MAX_ATTACHMENT_BYTES":"1","HATCHDOOR_MCP_MAX_BASE64_BYTES":"1","HATCHDOOR_MCP_WRITE_ENABLED":"true"}}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(limits.status(), StatusCode::OK);

        let limited_attachment = app
            .clone()
            .oneshot(attachment_upload_request(
                "Attachments/limited.png",
                Some("first-token"),
            ))
            .await
            .expect("response");
        assert_eq!(limited_attachment.status(), StatusCode::BAD_REQUEST);

        let limited_base64 = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/mcp")
                    .method("POST")
                    .header("authorization", "Bearer first-token")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"import_attachment","arguments":{"target_relative_path":"Attachments/limited.png","content":"cG5nLWJ5dGVz"}}}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(limited_base64.status(), StatusCode::OK);
        let limited_base64 = axum::body::to_bytes(limited_base64.into_body(), usize::MAX)
            .await
            .expect("body");
        assert!(String::from_utf8_lossy(&limited_base64).contains("1-byte"));

        let disabled = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/settings")
                    .method("PATCH")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"updates":{"HATCHDOOR_MCP_ENABLED":"false"}}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(disabled.status(), StatusCode::OK);

        // Disabling MCP revokes its bearer token for the attachment route even
        // when the configured token remains in the live settings snapshot.
        // Authentication runs before the one-byte limit set above, so this is
        // `401`, not `400`.
        let disabled_attachment = app
            .clone()
            .oneshot(attachment_upload_request(
                "Attachments/disabled.png",
                Some("first-token"),
            ))
            .await
            .expect("response");
        assert_eq!(disabled_attachment.status(), StatusCode::UNAUTHORIZED);

        let rotated = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/settings")
                    .method("PATCH")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"updates":{"HATCHDOOR_MCP_ENABLED":"true","HATCHDOOR_MCP_BEARER_TOKEN":"second-token","HATCHDOOR_MAX_ATTACHMENT_BYTES":"10485760"}}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(rotated.status(), StatusCode::OK);

        let old_token = app
            .clone()
            .oneshot(attachment_upload_request(
                "Attachments/old-token.png",
                Some("first-token"),
            ))
            .await
            .expect("response");
        assert_eq!(old_token.status(), StatusCode::UNAUTHORIZED);

        let new_token = app
            .oneshot(attachment_upload_request(
                "Attachments/new-token.png",
                Some("second-token"),
            ))
            .await
            .expect("response");
        assert_eq!(new_token.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn mcp_token_candidate_is_not_persisted_and_reveal_requires_equal_capability() {
        let (app, _tmp, state) = app_for_tests_with_web_auth(Some(Arc::from("web-secret")));

        let candidate = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/settings/mcp-token/generate")
                    .method("POST")
                    .header("authorization", "Bearer web-secret")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(candidate.status(), StatusCode::OK);
        assert_eq!(candidate.headers()["cache-control"], "no-store");
        let candidate = axum::body::to_bytes(candidate.into_body(), usize::MAX)
            .await
            .expect("body");
        let candidate: serde_json::Value = serde_json::from_slice(&candidate).expect("json");
        assert_eq!(candidate["value"].as_str().map(str::len), Some(43));
        assert_eq!(
            state
                .runtime_snapshot()
                .setting("HATCHDOOR_MCP_BEARER_TOKEN")
                .expect("setting")
                .value,
            ""
        );

        let saved = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/settings")
                    .method("PATCH")
                    .header("authorization", "Bearer web-secret")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"updates":{"HATCHDOOR_MCP_ENABLED":"true","HATCHDOOR_MCP_BEARER_TOKEN":"web-secret"}}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(saved.status(), StatusCode::OK);

        let reveal = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/settings/mcp-token/reveal")
                    .method("POST")
                    .header("authorization", "Bearer web-secret")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(reveal.status(), StatusCode::OK);
        assert_eq!(reveal.headers()["cache-control"], "no-store");

        let rotated = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/settings")
                    .method("PATCH")
                    .header("authorization", "Bearer web-secret")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"updates":{"HATCHDOOR_MCP_BEARER_TOKEN":"mcp-secret"}}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(rotated.status(), StatusCode::OK);

        let hidden = app
            .oneshot(
                Request::builder()
                    .uri("/api/settings/mcp-token/reveal")
                    .method("POST")
                    .header("authorization", "Bearer web-secret")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(hidden.status(), StatusCode::NOT_FOUND);
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
    async fn unavailable_managed_vault_keeps_liveness_status_and_spa_available() {
        let (_app, _tmp, mut state) = app_for_tests_with_state();
        *state.ready_vault.write().await = None;
        state.startup = StartupTracker::new(VaultRuntime::new(VaultSource::ManagedGit(
            crate::vault_runtime::ManagedGitSource {
                repository_url: "https://example.test/vault.git".to_string(),
                checkout_path: "/data/repositories/vault".into(),
                branch: Some("main".to_string()),
                vault_subdirectory: None,
                mode: crate::vault_runtime::ManagedGitMode::PullOnly,
            },
        )));
        state.startup.runtime().set_unavailable(
            "managed_vault_not_acquired",
            "Managed vault has not been acquired.",
        );
        let app = build_router(state, None);

        for path in ["/health", "/api/startup-status", "/api/vault-status"] {
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

        let spa = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        let spa_status = spa.status();
        let spa_body = to_bytes(spa.into_body(), usize::MAX)
            .await
            .expect("SPA body");
        assert!(
            spa_status == StatusCode::OK
                || (spa_status == StatusCode::SERVICE_UNAVAILABLE
                    && std::str::from_utf8(&spa_body)
                        .expect("SPA html")
                        .contains("Frontend not built")),
            "SPA route must reach the SPA handler rather than vault readiness middleware"
        );

        let status = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/vault-status")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        let payload: serde_json::Value = serde_json::from_slice(
            &to_bytes(status.into_body(), usize::MAX)
                .await
                .expect("status body"),
        )
        .expect("status json");
        assert_eq!(payload["phase"], "unavailable");
        assert_eq!(payload["source"], "managed-git");
        assert_eq!(payload["mode"], "pull-only");
        assert_eq!(payload["capabilities"]["mutate"], false);
        assert!(!payload.to_string().contains("/data/repositories/vault"));

        let vault_response = app
            .oneshot(
                Request::builder()
                    .uri("/api/tree")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(vault_response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let payload: serde_json::Value = serde_json::from_slice(
            &to_bytes(vault_response.into_body(), usize::MAX)
                .await
                .expect("vault response body"),
        )
        .expect("vault response json");
        assert_eq!(payload["code"], "managed_vault_not_acquired");
    }

    #[tokio::test]
    async fn pull_only_lifecycle_disables_browser_mutation() {
        let (_app, _tmp, mut state) = app_for_tests_with_state();
        state.startup = StartupTracker::new(VaultRuntime::ready(VaultSource::ManagedGit(
            crate::vault_runtime::ManagedGitSource {
                repository_url: "https://example.test/vault.git".to_string(),
                checkout_path: "/data/repositories/vault".into(),
                branch: None,
                vault_subdirectory: None,
                mode: crate::vault_runtime::ManagedGitMode::PullOnly,
            },
        )));
        let app = build_router(state, None);

        let capabilities = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/write-capabilities")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        let payload: serde_json::Value = serde_json::from_slice(
            &to_bytes(capabilities.into_body(), usize::MAX)
                .await
                .expect("capabilities body"),
        )
        .expect("capabilities json");
        assert_eq!(payload["enabled"], false);

        let mutation = app
            .oneshot(
                Request::builder()
                    .uri("/api/note")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"relative_path":"Blocked.md","content":"no"}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(mutation.status(), StatusCode::FORBIDDEN);
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

        for path in ["/api/tree"] {
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
        let (vault_work, _vault_worker) = crate::vault_work::VaultWorkCoordinator::new();
        let managed_git =
            std::sync::Arc::new(crate::git::ManagedGitScheduler::new(vault_work.clone()));
        let state = AppState {
            cache_db_path: tmp.path().join("cache.sqlite3"),
            vault_registry: VaultRegistryStore::new(tmp.path().join("state/vaults.json")),
            vaults: VaultCollectionRuntime::new(),
            vault_work,
            managed_git,
            legacy_migration_recovery: None,
            startup_sqlite: cache.sqlite.clone(),
            ready_vault: Arc::new(RwLock::new(Some(ReadyVault {
                vault_path: vault_root,
                cache,
            }))),
            vault_revision: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            vault_events,
            mcp_tools_changed,
            embedder,
            runtime_embedder: Arc::new(RuntimeEmbedder::new()),
            model_setup: Arc::new(ModelSetup::new(tmp.path().join("models"))),
            model_setup_started: Arc::new(AtomicBool::new(true)),
            startup_git_config: Arc::new(None),
            web_auth_enabled: false,
            demo_mode: false,
            vault_write_lock: Arc::new(tokio::sync::Mutex::new(())),
            git_sync: Arc::new(RwLock::new(None)),
            scan_config_cache: Arc::new(std::sync::RwLock::new(None)),
            refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
            index_status: crate::app_state::IndexStatusTracker::up_to_date(),
            runtime_config: crate::runtime_config::RuntimeConfig::for_tests(),
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

        let model_setup_without_token = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/model/retry")
                    .method("POST")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(model_setup_without_token.status(), StatusCode::UNAUTHORIZED);

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
    async fn settings_routes_require_web_auth_and_are_absent_in_demo_mode() {
        let (protected, _tmp, state) = app_for_tests_with_web_auth(Some(Arc::from("web-secret")));
        let unauthenticated = protected
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/settings")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);
        let authenticated = protected
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/settings")
                    .header("authorization", "Bearer web-secret")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(authenticated.status(), StatusCode::OK);

        state.index_status.queue_rebuild();
        let index_status = protected
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/index-status")
                    .header("authorization", "Bearer web-secret")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(index_status.status(), StatusCode::OK);
        assert_eq!(index_status.headers()["cache-control"], "no-store");

        let git_status = protected
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/git-status")
                    .header("authorization", "Bearer web-secret")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(git_status.status(), StatusCode::OK);
        assert_eq!(git_status.headers()["cache-control"], "no-store");

        let (demo, _tmp, _) = app_for_tests_with_web_auth_and_demo_mode(None, true);
        let absent = demo
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/settings")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(absent.status(), StatusCode::NOT_FOUND);
        let index_status_absent = demo
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/index-status")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(index_status_absent.status(), StatusCode::NOT_FOUND);
        let git_status_absent = demo
            .oneshot(
                Request::builder()
                    .uri("/api/git-status")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(git_status_absent.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn reindex_settings_persist_before_a_background_rebuild_and_then_converge() {
        let (app, _tmp, state) = app_for_tests_with_state();
        let held_refresh_lock = state.refresh_lock.lock().await;

        let missing_confirmation = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/settings")
                    .method("PATCH")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"updates":{"HATCHDOOR_EXCLUDE":"generated/**"}}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(missing_confirmation.status(), StatusCode::CONFLICT);
        let missing_confirmation_body =
            axum::body::to_bytes(missing_confirmation.into_body(), usize::MAX)
                .await
                .expect("body");
        let missing_confirmation_json: serde_json::Value =
            serde_json::from_slice(&missing_confirmation_body).expect("json");
        assert_eq!(
            missing_confirmation_json["confirmation_required"],
            "reindex"
        );

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/settings")
                    .method("PATCH")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"updates":{"HATCHDOOR_EXCLUDE":"generated/**"},"confirm":["reindex"]}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            state
                .runtime_snapshot()
                .setting("HATCHDOOR_EXCLUDE")
                .expect("setting")
                .value,
            "generated/**"
        );
        assert_eq!(state.index_status.status().state, "rebuilding");
        assert!(state.index_status.status().stale);

        let second_response = app
            .oneshot(
                Request::builder()
                    .uri("/api/settings")
                    .method("PATCH")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"updates":{"HATCHDOOR_EMBED_LAYERS":"false"},"confirm":["reindex"]}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(second_response.status(), StatusCode::OK);
        assert_eq!(
            state
                .runtime_snapshot()
                .setting("HATCHDOOR_EMBED_LAYERS")
                .expect("setting")
                .value,
            "false"
        );

        drop(held_refresh_lock);
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if state.index_status.status().state == "up_to_date" {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("background rebuild finishes");
        assert!(!state.index_status.status().stale);
    }

    #[tokio::test]
    async fn enabling_local_versioning_requires_confirmation_then_succeeds() {
        // S4/S5: the server is the authority on the git_init confirmation (a
        // 409 carrying only the machine-readable consequence, not prose —
        // the page owns the words), and a resend with `confirm` containing
        // it must actually create the local repository.
        let (app, tmp, state) = app_for_tests_with_state();

        let missing_confirmation = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/settings")
                    .method("PATCH")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"updates":{"HATCHDOOR_GIT_SYNC_ENABLED":"local"}}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(missing_confirmation.status(), StatusCode::CONFLICT);
        let body = axum::body::to_bytes(missing_confirmation.into_body(), usize::MAX)
            .await
            .expect("body");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(json["confirmation_required"], "git_init");
        assert!(
            json.get("error").is_none(),
            "the server must not send prose for this consequence: {json}"
        );
        assert!(!tmp.path().join("vault/.git").exists());

        let confirmed = app
            .oneshot(
                Request::builder()
                    .uri("/api/settings")
                    .method("PATCH")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"updates":{"HATCHDOOR_GIT_SYNC_ENABLED":"local"},"confirm":["git_init"]}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(confirmed.status(), StatusCode::OK);
        assert!(tmp.path().join("vault/.git").exists());
        assert_eq!(
            state
                .runtime_snapshot()
                .setting("HATCHDOOR_GIT_SYNC_ENABLED")
                .expect("setting")
                .value,
            "local"
        );
    }

    #[tokio::test]
    async fn downgrade_onto_a_non_repo_vault_accumulates_both_consents() {
        // S4's two-consent case: switching remote -> local when the vault is
        // no longer a git repository needs both `git_downgrade` (leaving
        // remote sync) and `git_init` (creating fresh local history).
        // Accepting one must not drop the other on the next round-trip: the
        // page is expected to accumulate accepted consequences into one list
        // across successive 409s, and the server must honor that list rather
        // than only ever remembering the single most recent consent.
        let (app, tmp, state) = app_for_tests_with_state();
        state
            .runtime_config
            .save([
                (
                    "HATCHDOOR_GIT_SYNC_ENABLED".to_string(),
                    "remote".to_string(),
                ),
                ("HATCHDOOR_GIT_HTTPS_TOKEN".to_string(), "token".to_string()),
            ])
            .expect("save initial remote config");
        // No .git directory exists in this vault at all: remote mode was only
        // ever configured, never actually initialized on disk.
        assert!(!tmp.path().join("vault/.git").exists());

        let no_confirmation = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/settings")
                    .method("PATCH")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"updates":{"HATCHDOOR_GIT_SYNC_ENABLED":"local"}}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(no_confirmation.status(), StatusCode::CONFLICT);
        let body = axum::body::to_bytes(no_confirmation.into_body(), usize::MAX)
            .await
            .expect("body");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(json["confirmation_required"], "git_downgrade");

        // Accept the downgrade only: must not silently also accept git_init.
        let downgrade_only = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/settings")
                    .method("PATCH")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"updates":{"HATCHDOOR_GIT_SYNC_ENABLED":"local"},"confirm":["git_downgrade"]}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(downgrade_only.status(), StatusCode::CONFLICT);
        let body = axum::body::to_bytes(downgrade_only.into_body(), usize::MAX)
            .await
            .expect("body");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(
            json["confirmation_required"], "git_init",
            "the second consequence, not a ping-pong back to git_downgrade"
        );

        // The page accumulates: resend with BOTH accepted consequences.
        let both_confirmed = app
            .oneshot(
                Request::builder()
                    .uri("/api/settings")
                    .method("PATCH")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"updates":{"HATCHDOOR_GIT_SYNC_ENABLED":"local"},"confirm":["git_downgrade","git_init"]}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(both_confirmed.status(), StatusCode::OK);
        assert!(tmp.path().join("vault/.git").exists());
        assert_eq!(
            state
                .runtime_snapshot()
                .setting("HATCHDOOR_GIT_SYNC_ENABLED")
                .expect("setting")
                .value,
            "local"
        );
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
    async fn demo_mode_hides_model_selection_endpoints() {
        let (app, _tmp, _state) = app_for_tests_with_web_auth_and_demo_mode(None, true);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/model/retry")
                    .method("POST")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn router_reports_write_capabilities_disabled_for_read_only_vault() {
        let (app, tmp, state) = app_for_tests_with_state();
        let vault_path = state.vault_path().await.expect("ready vault path");
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
    async fn write_api_rejects_move_to_a_noise_path() {
        // Moving an already-indexed note into `.trash/` would make it disappear
        // on the next refresh. Every write target, not just creates, must be
        // checked against the scan config.
        let (app, tmp, _state) = app_for_tests_with_state();
        let hash = crate::cache::parse::content_hash("# Home\n");

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/note/home/move")
                    .method("PATCH")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"target_folder":".trash","expected_content_hash":"{hash}"}}"#
                    )))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(tmp.path().join("vault/Home.md").exists());
        assert!(!tmp.path().join("vault/.trash/Home.md").exists());
    }

    #[tokio::test]
    async fn write_api_current_index_honours_user_excludes() {
        // Write routes rebuild a short-lived index for slug/path work. That
        // rebuild must use HATCHDOOR_EXCLUDE too, or an excluded note becomes
        // writable even though it is absent from every read surface.
        let (_unused_app, _tmp, state) = app_for_tests_with_state();
        let vault_path = state.vault_path().await.expect("ready vault");
        std::fs::write(vault_path.join("Ignored.md"), "# Ignored\n").expect("write note");
        state
            .runtime_config
            .save([("HATCHDOOR_EXCLUDE".to_string(), "Ignored.md".to_string())])
            .expect("save exclude setting");
        let app = build_router(state, None);
        let hash = crate::cache::parse::content_hash("# Ignored\n");

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/note/ignored")
                    .method("PUT")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r##"{{"content":"# Changed\n","expected_content_hash":"{hash}"}}"##
                    )))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn write_api_rejects_archive_to_a_noise_path() {
        let (_unused_app, tmp, state) = app_for_tests_with_state();
        state
            .runtime_config
            .save([("HATCHDOOR_EXCLUDE".to_string(), "90-archive/".to_string())])
            .expect("save exclude setting");
        let app = build_router(state, None);
        let hash = crate::cache::parse::content_hash("# Home\n");

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/note/home/archive")
                    .method("PATCH")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"expected_content_hash":"{hash}"}}"#
                    )))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(tmp.path().join("vault/Home.md").exists());
        assert!(!tmp.path().join("vault/90-archive/Home.md").exists());
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
        let blocked_path = state
            .vault_path()
            .await
            .expect("ready vault path")
            .join("Demo Write.md");

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
    async fn diagnostics_route_serves_ruleset_and_is_disabled_in_demo_mode() {
        // Non-demo: the route returns the active noise ruleset.
        let (app, _tmp) = app_for_tests();
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/diagnostics")
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
        assert!(
            payload["noise_patterns"]
                .as_array()
                .expect("noise_patterns")
                .iter()
                .any(|p| p["source"] == "built-in"),
            "the ruleset dump must list the built-in noise patterns"
        );

        // Demo mode: the surface is disabled entirely (it would reveal demoted paths).
        let (demo_app, _demo_tmp, _state) = app_for_tests_with_web_auth_and_demo_mode(None, true);
        let demo = demo_app
            .oneshot(
                Request::builder()
                    .uri("/api/diagnostics")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(demo.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn demo_mode_404s_demoted_notes_on_fetch_and_download() {
        // In demo mode demotion becomes exclusion: a demoted note is a 404 on the
        // note and download routes, while a default-surface note stays reachable.
        let tmp = TempDir::new().expect("temp dir");
        let vault_root = tmp.path().join("vault");
        std::fs::create_dir_all(vault_root.join("sources")).expect("sources dir");
        std::fs::write(vault_root.join("sources/.hatchdoor-layer"), "sources").expect("marker");
        std::fs::write(vault_root.join("sources/Clip.md"), "# Clip\n").expect("clip");
        std::fs::write(vault_root.join("Home.md"), "# Home\n").expect("home");
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
        let cache = build_cache(&vault_root, embedder.as_ref()).expect("cache");
        let (vault_events, _) = tokio::sync::broadcast::channel(64);
        let (mcp_tools_changed, _) = tokio::sync::broadcast::channel(16);
        let (vault_work, _vault_worker) = crate::vault_work::VaultWorkCoordinator::new();
        let managed_git =
            std::sync::Arc::new(crate::git::ManagedGitScheduler::new(vault_work.clone()));
        let state = AppState {
            cache_db_path: tmp.path().join("cache.sqlite3"),
            vault_registry: VaultRegistryStore::new(tmp.path().join("state/vaults.json")),
            vaults: VaultCollectionRuntime::new(),
            vault_work,
            managed_git,
            legacy_migration_recovery: None,
            startup_sqlite: cache.sqlite.clone(),
            ready_vault: Arc::new(RwLock::new(Some(ReadyVault {
                vault_path: vault_root,
                cache,
            }))),
            vault_revision: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            vault_events,
            mcp_tools_changed,
            embedder,
            runtime_embedder: Arc::new(RuntimeEmbedder::new()),
            model_setup: Arc::new(ModelSetup::new(tmp.path().join("models"))),
            model_setup_started: Arc::new(AtomicBool::new(true)),
            startup_git_config: Arc::new(None),
            web_auth_enabled: false,
            demo_mode: true,
            vault_write_lock: Arc::new(tokio::sync::Mutex::new(())),
            git_sync: Arc::new(RwLock::new(None)),
            scan_config_cache: Arc::new(std::sync::RwLock::new(None)),
            refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
            index_status: crate::app_state::IndexStatusTracker::up_to_date(),
            runtime_config: crate::runtime_config::RuntimeConfig::for_tests(),
            startup: StartupTracker::ready(),
        };
        let app = build_router(state, None);

        let demoted = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/note/clip")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(demoted.status(), StatusCode::NOT_FOUND);

        let demoted_download = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/note/clip/download")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(demoted_download.status(), StatusCode::NOT_FOUND);

        let demoted_links = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/note/clip/links")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(demoted_links.status(), StatusCode::NOT_FOUND);

        let resolved_demoted = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/resolve?target=sources%2FClip")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(resolved_demoted.status(), StatusCode::OK);
        let resolve_body = to_bytes(resolved_demoted.into_body(), usize::MAX)
            .await
            .expect("body");
        let resolved: serde_json::Value = serde_json::from_slice(&resolve_body).expect("json");
        assert_eq!(resolved["slug"], serde_json::Value::Null);

        let rejected_layer_query = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/tree?layers=sources")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(rejected_layer_query.status(), StatusCode::BAD_REQUEST);

        let default = app
            .oneshot(
                Request::builder()
                    .uri("/api/note/home")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(
            default.status(),
            StatusCode::OK,
            "a default-surface note stays reachable in demo mode"
        );
        drop(tmp);
    }

    #[tokio::test]
    async fn vault_assets_are_served_with_private_cache_control() {
        // Assets must be browser-cacheable (they re-render on every note view)
        // but never shared-cacheable: authenticated deployments put
        // ?access_token= in asset URLs.
        let (app, tmp, state) = app_for_tests_with_web_auth(None);
        std::fs::write(
            state
                .vault_path()
                .await
                .expect("ready vault path")
                .join("diagram.png"),
            b"png-bytes",
        )
        .expect("write asset");

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
            "layer",
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
        assert_eq!(archived["layer"], serde_json::Value::Null);

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

    // -----------------------------------------------------------------
    // #98: /api/v1/vaults discovery, management, and events
    // -----------------------------------------------------------------

    fn create_vault_request_body(name: &str, path: &std::path::Path, revision: u64) -> Body {
        Body::from(
            serde_json::json!({
                "expected_registry_revision": revision,
                "name": name,
                "enabled": true,
                "source": {"type": "local", "path": path.to_string_lossy()},
                "exclude_patterns": [],
            })
            .to_string(),
        )
    }

    async fn json_body(response: Response) -> serde_json::Value {
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        serde_json::from_slice(&bytes).expect("json body")
    }

    #[tokio::test]
    async fn vaults_v1_discovery_is_reachable_at_zero_vaults() {
        let (app, _tmp, _state) = app_for_tests_with_web_auth(None);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = json_body(response).await;
        assert_eq!(body["registry_revision"], 0);
        assert_eq!(body["collection_revision"], 0);
        assert_eq!(body["vaults"], serde_json::json!([]));
        assert!(body.get("recovery").is_none());
    }

    #[tokio::test]
    async fn vaults_v1_requires_web_token_when_configured() {
        let (app, _tmp, _state) = app_for_tests_with_web_auth(Some(Arc::from("secret")));

        let unauthorized = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let authorized = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults")
                    .header("authorization", "Bearer secret")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(authorized.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn vaults_v1_absent_in_demo_mode() {
        let (app, _tmp, _state) = app_for_tests_with_web_auth_and_demo_mode(None, true);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn vaults_v1_create_lists_the_new_vault_with_optimistic_concurrency() {
        let (app, tmp, _state) = app_for_tests_with_web_auth(None);
        let vault_path = tmp.path().join("second-vault");
        std::fs::create_dir_all(&vault_path).expect("create vault directory");

        // A stale expected revision is rejected with a structured, retryable
        // conflict rather than silently succeeding.
        let stale = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(create_vault_request_body("Second", &vault_path, 41))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(stale.status(), StatusCode::CONFLICT);
        let stale_body = json_body(stale).await;
        assert_eq!(stale_body["code"], "registry_revision_conflict");
        assert_eq!(stale_body["retryable"], true);

        let created = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(create_vault_request_body("Second", &vault_path, 0))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(created.status(), StatusCode::CREATED);
        let created_body = json_body(created).await;
        assert_eq!(created_body["registry_revision"], 1);
        let vault = &created_body["vault"];
        assert_eq!(vault["name"], "Second");
        assert_eq!(vault["credential_configured"], false);
        assert_eq!(vault["capabilities"]["browse"], true);
        let vault_id = vault["vault_id"].as_str().expect("vault id").to_string();

        let duplicate = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(create_vault_request_body("Second", &vault_path, 1))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(duplicate.status(), StatusCode::CONFLICT);
        assert_eq!(json_body(duplicate).await["code"], "duplicate_vault_name");

        let discovery = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        let discovery_body = json_body(discovery).await;
        assert_eq!(discovery_body["registry_revision"], 1);
        let vaults = discovery_body["vaults"].as_array().expect("vaults array");
        assert_eq!(vaults.len(), 1);
        assert_eq!(vaults[0]["vault_id"], vault_id);
    }

    #[tokio::test]
    async fn vaults_v1_enable_disable_disconnect_lifecycle() {
        let (app, tmp, _state) = app_for_tests_with_web_auth(None);
        let vault_path = tmp.path().join("lifecycle-vault");
        std::fs::create_dir_all(&vault_path).expect("create vault directory");
        let created = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(create_vault_request_body("Lifecycle", &vault_path, 0))
                    .expect("request"),
            )
            .await
            .expect("response");
        let created_body = json_body(created).await;
        let vault_id = created_body["vault"]["vault_id"]
            .as_str()
            .expect("vault id")
            .to_string();

        let disabled = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/v1/vaults/{vault_id}/disable?expected_registry_revision=1"
                    ))
                    .method("POST")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(disabled.status(), StatusCode::OK);
        let disabled_body = json_body(disabled).await;
        assert_eq!(disabled_body["vault"]["enabled"], false);
        assert_eq!(disabled_body["vault"]["capabilities"]["browse"], false);

        let enabled = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/v1/vaults/{vault_id}/enable?expected_registry_revision=2"
                    ))
                    .method("POST")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(enabled.status(), StatusCode::OK);
        assert_eq!(json_body(enabled).await["vault"]["enabled"], true);

        let disconnected = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/v1/vaults/{vault_id}?expected_registry_revision=3"
                    ))
                    .method("DELETE")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(disconnected.status(), StatusCode::OK);
        let disconnected_body = json_body(disconnected).await;
        assert!(disconnected_body.get("vault").is_none() || disconnected_body["vault"].is_null());
        assert_eq!(disconnected_body["registry_revision"], 4);
        assert!(
            vault_path.exists(),
            "disconnect must not delete Vault files"
        );

        let discovery = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(json_body(discovery).await["vaults"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn vaults_v1_edit_identity_change_requires_disabled_then_confirmation() {
        let (app, tmp, _state) = app_for_tests_with_web_auth(None);
        let first_path = tmp.path().join("identity-first");
        let second_path = tmp.path().join("identity-second");
        std::fs::create_dir_all(&first_path).expect("first vault directory");
        std::fs::create_dir_all(&second_path).expect("second vault directory");
        let created = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(create_vault_request_body("Identity", &first_path, 0))
                    .expect("request"),
            )
            .await
            .expect("response");
        let vault_id = json_body(created).await["vault"]["vault_id"]
            .as_str()
            .expect("vault id")
            .to_string();

        let edit_body = |revision: u64, confirm: bool| {
            Body::from(
                serde_json::json!({
                    "expected_registry_revision": revision,
                    "name": "Identity",
                    "source": {"type": "local", "path": second_path.to_string_lossy()},
                    "exclude_patterns": [],
                    "confirm_identity_change": confirm,
                })
                .to_string(),
            )
        };

        // Still enabled: an identity-bearing change is refused outright, even
        // with confirmation, until the Vault is disabled first.
        let while_enabled = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{vault_id}"))
                    .method("PATCH")
                    .header("content-type", "application/json")
                    .body(edit_body(1, true))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(while_enabled.status(), StatusCode::CONFLICT);
        assert_eq!(
            json_body(while_enabled).await["code"],
            "identity_change_requires_disabled"
        );

        let disabled = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/v1/vaults/{vault_id}/disable?expected_registry_revision=1"
                    ))
                    .method("POST")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(disabled.status(), StatusCode::OK);

        let unconfirmed = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{vault_id}"))
                    .method("PATCH")
                    .header("content-type", "application/json")
                    .body(edit_body(2, false))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(unconfirmed.status(), StatusCode::CONFLICT);
        assert_eq!(
            json_body(unconfirmed).await["code"],
            "identity_change_requires_confirmation"
        );

        let confirmed = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{vault_id}"))
                    .method("PATCH")
                    .header("content-type", "application/json")
                    .body(edit_body(2, true))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(confirmed.status(), StatusCode::OK);
        let confirmed_body = json_body(confirmed).await;
        assert_eq!(
            confirmed_body["vault"]["source"]["path"],
            second_path.to_string_lossy().as_ref()
        );
    }

    #[tokio::test]
    async fn vaults_v1_sync_and_retry_require_a_managed_git_source() {
        let (app, tmp, _state) = app_for_tests_with_web_auth(None);
        let vault_path = tmp.path().join("local-only");
        std::fs::create_dir_all(&vault_path).expect("create vault directory");
        let created = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(create_vault_request_body("LocalOnly", &vault_path, 0))
                    .expect("request"),
            )
            .await
            .expect("response");
        let vault_id = json_body(created).await["vault"]["vault_id"]
            .as_str()
            .expect("vault id")
            .to_string();

        let sync = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{vault_id}/sync"))
                    .method("POST")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(sync.status(), StatusCode::CONFLICT);
        assert_eq!(json_body(sync).await["code"], "capability_unavailable");

        let missing = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults/00000000-0000-4000-8000-000000000000/retry")
                    .method("POST")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        assert_eq!(json_body(missing).await["code"], "vault_not_found");
    }

    #[tokio::test]
    async fn vaults_v1_malformed_vault_id_is_a_structured_bad_request() {
        let (app, _tmp, _state) = app_for_tests_with_web_auth(None);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults/not-a-uuid/sync")
                    .method("POST")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(json_body(response).await["code"], "invalid_vault_id");
    }

    #[tokio::test]
    async fn vaults_v1_enable_without_a_revision_query_is_a_structured_bad_request() {
        let (app, tmp, _state) = app_for_tests_with_web_auth(None);
        let vault_path = tmp.path().join("no-query-vault");
        std::fs::create_dir_all(&vault_path).expect("create vault directory");
        let created = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(create_vault_request_body("NoQuery", &vault_path, 0))
                    .expect("request"),
            )
            .await
            .expect("response");
        let vault_id = json_body(created).await["vault"]["vault_id"]
            .as_str()
            .expect("vault id")
            .to_string();

        // No `?expected_registry_revision=...` query string at all: the
        // structured error shape must survive extractor rejection, not just
        // handler-body validation.
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{vault_id}/disable"))
                    .method("POST")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = json_body(response).await;
        assert_eq!(body["code"], "invalid_request_query");
        assert_eq!(body["retryable"], false);
    }

    #[tokio::test]
    async fn vaults_v1_discovery_reports_registry_recovery_without_crashing() {
        let (app, _tmp, state) = app_for_tests_with_web_auth(None);
        std::fs::create_dir_all(state.vault_registry.path().parent().unwrap())
            .expect("registry directory");
        std::fs::write(state.vault_registry.path(), b"not valid json").expect("corrupt registry");

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = json_body(response).await;
        assert!(body["registry_revision"].is_null());
        assert_eq!(body["vaults"], serde_json::json!([]));
        assert_eq!(body["recovery"]["code"], "vault_registry_recovery_required");
        assert_eq!(body["recovery"]["kind"], "corrupt");
    }

    #[tokio::test]
    async fn vaults_v1_events_route_is_not_shadowed_by_the_vault_id_wildcard() {
        let (app, _tmp, _state) = app_for_tests_with_web_auth(None);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults/events")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], "text/event-stream");
    }

    #[tokio::test]
    async fn vaults_v1_events_stream_reports_the_affected_vault_and_category() {
        let (app, tmp, _state) = app_for_tests_with_web_auth(None);
        let vault_path = tmp.path().join("event-vault");
        std::fs::create_dir_all(&vault_path).expect("create vault directory");

        let events_request = Request::builder()
            .uri("/api/v1/vaults/events")
            .body(Body::empty())
            .expect("request");
        let events_response = app.clone().oneshot(events_request).await.expect("response");
        let mut stream = events_response.into_body().into_data_stream();
        // The stream immediately yields the current (empty-collection) value.
        let _initial = tokio::time::timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("initial SSE event")
            .expect("stream item")
            .expect("body chunk");

        let create_response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(create_vault_request_body("EventVault", &vault_path, 0))
                    .expect("request"),
            )
            .await
            .expect("response");
        let vault_id = json_body(create_response).await["vault"]["vault_id"]
            .as_str()
            .expect("vault id")
            .to_string();

        let chunk = tokio::time::timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("definition-change SSE event")
            .expect("stream item")
            .expect("body chunk");
        let event = std::str::from_utf8(&chunk).expect("utf8 event");
        assert!(event.contains("event: vault-collection-revision"));
        assert!(event.contains(r#""category":"definition""#));
        assert!(event.contains(&vault_id));
    }

    // -----------------------------------------------------------------
    // #99: /api/v1/vaults/{vault_id} exact reads and contained resources
    // -----------------------------------------------------------------

    /// Creates a Vault (with `files` already written to disk) via the same
    /// HTTP surface #98 exercises, and returns its `vault_id`. Local-source
    /// activation completes synchronously within the request (proven by
    /// `vaults_v1_create_lists_the_new_vault_with_optimistic_concurrency`
    /// asserting `capabilities.browse == true` immediately after creation),
    /// so exact reads against the returned ID are immediately valid.
    async fn create_vault_with_files(
        app: &Router,
        name: &str,
        root: &std::path::Path,
        files: &[(&str, &str)],
        revision: u64,
    ) -> String {
        std::fs::create_dir_all(root).expect("create vault directory");
        for (relative_path, contents) in files {
            let path = root.join(relative_path);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("create parent dir");
            std::fs::write(path, contents).expect("write note");
        }
        let created = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(create_vault_request_body(name, root, revision))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(created.status(), StatusCode::CREATED);
        json_body(created).await["vault"]["vault_id"]
            .as_str()
            .expect("vault id")
            .to_string()
    }

    #[tokio::test]
    async fn vault_scoped_note_and_links_stay_within_the_requested_vault_for_duplicate_slugs() {
        let (app, tmp, _state) = app_for_tests_with_web_auth(None);
        let first = create_vault_with_files(
            &app,
            "First",
            &tmp.path().join("first"),
            &[
                ("Home.md", "# Home\n\nfirst\n\n[[Shared]]"),
                ("Shared.md", "# Shared"),
            ],
            0,
        )
        .await;
        let second = create_vault_with_files(
            &app,
            "Second",
            &tmp.path().join("second"),
            &[
                ("Home.md", "# Home\n\nsecond\n\n[[Shared]]"),
                ("Shared.md", "# Shared"),
            ],
            1,
        )
        .await;

        let first_note = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{first}/notes/home"))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(first_note.status(), StatusCode::OK);
        let first_body = json_body(first_note).await;
        assert_eq!(first_body["vault_id"], first);
        assert!(
            first_body["note"]["content"]
                .as_str()
                .unwrap()
                .contains("first")
        );

        let second_note = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{second}/notes/home"))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(second_note.status(), StatusCode::OK);
        let second_body = json_body(second_note).await;
        assert_eq!(second_body["vault_id"], second);
        assert!(
            second_body["note"]["content"]
                .as_str()
                .unwrap()
                .contains("second")
        );

        let first_links = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{first}/notes/home/links"))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(first_links.status(), StatusCode::OK);
        let first_links_body = json_body(first_links).await;
        assert_eq!(first_links_body["vault_id"], first);
        assert_eq!(first_links_body["outgoing"][0]["vault_id"], first);
    }

    #[tokio::test]
    async fn vault_scoped_note_reports_structured_errors_for_unknown_disabled_and_malformed_vaults()
    {
        let (app, tmp, _state) = app_for_tests_with_web_auth(None);
        let vault_id = create_vault_with_files(
            &app,
            "Lifecycle",
            &tmp.path().join("lifecycle"),
            &[("Home.md", "# Home")],
            0,
        )
        .await;

        let malformed = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults/not-a-uuid/notes/home")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);
        assert_eq!(json_body(malformed).await["code"], "invalid_vault_id");

        let unknown_id = "00000000-0000-4000-8000-000000000000";
        let unknown = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{unknown_id}/notes/home"))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
        assert_eq!(json_body(unknown).await["code"], "vault_not_found");

        let missing_note = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{vault_id}/notes/does-not-exist"))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(missing_note.status(), StatusCode::NOT_FOUND);
        assert_eq!(json_body(missing_note).await["code"], "note_not_found");

        let disabled = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/v1/vaults/{vault_id}/disable?expected_registry_revision=1"
                    ))
                    .method("POST")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(disabled.status(), StatusCode::OK);

        let disabled_read = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{vault_id}/notes/home"))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(disabled_read.status(), StatusCode::CONFLICT);
        assert_eq!(json_body(disabled_read).await["code"], "vault_disabled");
    }

    #[tokio::test]
    async fn vault_scoped_resolve_and_resolve_batch_scope_to_one_vault() {
        let (app, tmp, _state) = app_for_tests_with_web_auth(None);
        let first = create_vault_with_files(
            &app,
            "First",
            &tmp.path().join("first"),
            &[
                ("Home.md", "# Home\n\n[[Shared]]"),
                ("Shared.md", "# Shared"),
            ],
            0,
        )
        .await;

        let resolved = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{first}/resolve?target=Shared"))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(resolved.status(), StatusCode::OK);
        let resolved_body = json_body(resolved).await;
        assert_eq!(resolved_body["vault_id"], first);
        assert_eq!(resolved_body["slug"], "shared");

        let unresolved = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{first}/resolve?target=Missing"))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(unresolved.status(), StatusCode::OK);
        assert!(json_body(unresolved).await["slug"].is_null());

        let missing_query = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{first}/resolve"))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(missing_query.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            json_body(missing_query).await["code"],
            "invalid_request_query"
        );

        let batch = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{first}/resolve-batch"))
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"targets": ["Shared", "Missing"]}).to_string(),
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(batch.status(), StatusCode::OK);
        let batch_body = json_body(batch).await;
        assert_eq!(batch_body["vault_id"], first);
        assert_eq!(batch_body["results"][0]["slug"], "shared");
        assert!(batch_body["results"][1]["slug"].is_null());
    }

    #[tokio::test]
    async fn vault_scoped_asset_retains_containment_and_vault_scoped_download_serves_export() {
        let (app, tmp, _state) = app_for_tests_with_web_auth(None);
        let first = create_vault_with_files(
            &app,
            "First",
            &tmp.path().join("first"),
            &[
                ("Home.md", "# Home\n\n![[diagram.png]]"),
                ("diagram.png", "png-bytes"),
                ("secret.txt", "not embeddable"),
            ],
            0,
        )
        .await;

        let asset = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{first}/assets/diagram.png"))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(asset.status(), StatusCode::OK);
        assert_eq!(asset.headers()["content-type"], "image/png");
        assert_eq!(asset.headers()["cache-control"], "private, max-age=3600");

        let traversal = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{first}/assets/../../etc/passwd"))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(traversal.status(), StatusCode::BAD_REQUEST);
        assert_eq!(json_body(traversal).await["code"], "invalid_asset_path");

        let disallowed_extension = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{first}/assets/secret.txt"))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(disallowed_extension.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            json_body(disallowed_extension).await["code"],
            "asset_access_denied"
        );

        let download = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{first}/notes/home/download"))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(download.status(), StatusCode::OK);
        assert_eq!(download.headers()["content-type"], "application/zip");
        assert_eq!(download.headers()["cache-control"], "no-store");
        assert!(
            download.headers()["content-disposition"]
                .to_str()
                .unwrap()
                .contains("Home.zip")
        );
    }

    #[tokio::test]
    async fn vault_scoped_asset_reports_a_retryable_unavailable_status_for_a_missing_directory() {
        // A managed-Git Vault can be enabled and accepting operations before
        // its checkout has materialized; simulate that by removing the
        // (local-source) Vault directory after creation. The asset route must
        // report the same retryable `vault_read_unavailable` code an exact
        // note read would for the identical underlying condition, not a
        // non-retryable 500 surfaced by an unchecked filesystem error.
        let (app, tmp, _state) = app_for_tests_with_web_auth(None);
        let vault_root = tmp.path().join("materializing");
        let first = create_vault_with_files(&app, "Materializing", &vault_root, &[], 0).await;
        std::fs::remove_dir_all(&vault_root).expect("remove vault directory");

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{first}/assets/diagram.png"))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = json_body(response).await;
        assert_eq!(body["code"], "vault_read_unavailable");
        assert_eq!(body["retryable"], true);
    }

    #[tokio::test]
    async fn vault_scoped_routes_require_web_token_and_are_absent_in_demo_mode() {
        let (app, tmp, _state) = app_for_tests_with_web_auth(Some(Arc::from("secret")));
        let vault_path = tmp.path().join("token-vault");
        std::fs::create_dir_all(&vault_path).expect("create vault directory");

        let unauthorized = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults/00000000-0000-4000-8000-000000000000/notes/home")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let authorized = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults/00000000-0000-4000-8000-000000000000/notes/home")
                    .header("authorization", "Bearer secret")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        // Authorized but the Vault does not exist: proves the token gate was
        // satisfied rather than short-circuiting before the handler ran.
        assert_eq!(authorized.status(), StatusCode::NOT_FOUND);

        let (demo_app, _demo_tmp, _demo_state) =
            app_for_tests_with_web_auth_and_demo_mode(None, true);
        let demo_response = demo_app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults/00000000-0000-4000-8000-000000000000/notes/home")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(demo_response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn vault_canonical_browser_note_url_reaches_the_spa_shell() {
        let (app, _tmp) = app_for_tests();
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v/00000000-0000-4000-8000-000000000000/n/home")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        // The frontend is not built in this test environment, so
        // `spa_index_handler` reports 503 rather than 200 — the discriminator
        // here is that it is *not* a router 404, proving the canonical Note
        // URL dispatches to the SPA shell rather than falling through.
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        assert!(String::from_utf8_lossy(&bytes).contains("Frontend not built"));
    }
}
