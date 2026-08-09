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
use axum::routing::{get, patch, post, put};
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
    MAX_IN_MEMORY_UPLOAD_BYTES, create_vault_handler, demo_read_only_response,
    disable_vault_handler, disconnect_vault_handler, edit_vault_handler, enable_vault_handler,
    generate_mcp_token_handler, get_git_status_handler, get_index_status_handler,
    get_settings_handler, health_handler, list_vaults_handler, patch_settings_handler,
    retry_vault_handler, reveal_mcp_token_handler, reveal_web_token_handler, spa_index_handler,
    sync_vault_handler, vault_collection_events_handler, vault_scope_graph_handler,
    vault_scope_recent_handler, vault_scope_search_handler, vault_scope_stats_handler,
    vault_scope_tree_handler, vault_scoped_archive_note_handler, vault_scoped_asset_handler,
    vault_scoped_create_note_handler, vault_scoped_delete_note_handler,
    vault_scoped_move_note_handler, vault_scoped_move_rename_note_handler,
    vault_scoped_note_download_handler, vault_scoped_note_handler, vault_scoped_note_links_handler,
    vault_scoped_rename_note_handler, vault_scoped_resolve_batch_handler,
    vault_scoped_resolve_handler, vault_scoped_update_note_handler,
    vault_scoped_upload_attachment_handler, vault_scoped_write_capabilities_handler,
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

    // Vault-collection discovery, management, events, exact content reads, and
    // #101's Vault-scoped mutations are deliberately not gated by any
    // `require_vault_ready`-style middleware: connecting the first Vault and
    // recovering a corrupt registry must stay reachable at zero enabled
    // Vaults. Every operation gates per-request on its own targeted Vault
    // instead (`vault_not_found`/`vault_disabled`/`vault_unavailable`/
    // `capability_unavailable` from `VaultReadCore`/`VaultControlBlock`),
    // which is the per-Vault equivalent this surface needs.
    //
    // #109: this whole group is now mounted in demo mode too — a demo
    // deployment publishes every enabled Vault as a public read-only
    // collection instead of having no working content surface at all
    // (#101's documented gap). Discovery, events, exact reads, contained
    // assets/downloads, resolve/resolve-batch, and one-or-all
    // tree/recent/stats/graph/search stay reachable unauthenticated. Every
    // mutation and Vault-control route (collection management, manual Git
    // sync/retry, Markdown mutations, write-capabilities discovery) is
    // wrapped individually below in `reject_demo_mutation`, which refuses
    // with the shared `403 demo_read_only` error before running — mounted
    // per mutating route/method rather than over this whole group, so a GET
    // read sharing a path with a mutating verb is unaffected.
    let demo_guard = axum::middleware::from_fn_with_state(state.clone(), reject_demo_mutation);
    let vaults_v1 = {
        let vaults_v1 = Router::new()
            .route(
                "/api/v1/vaults",
                get(list_vaults_handler)
                    .merge(post(create_vault_handler).layer(demo_guard.clone())),
            )
            .route(
                "/api/v1/vaults/events",
                get(vault_collection_events_handler),
            )
            .route(
                "/api/v1/vaults/{vault_id}",
                patch(edit_vault_handler)
                    .delete(disconnect_vault_handler)
                    .layer(demo_guard.clone()),
            )
            .route(
                "/api/v1/vaults/{vault_id}/enable",
                post(enable_vault_handler).layer(demo_guard.clone()),
            )
            .route(
                "/api/v1/vaults/{vault_id}/disable",
                post(disable_vault_handler).layer(demo_guard.clone()),
            )
            .route(
                "/api/v1/vaults/{vault_id}/sync",
                post(sync_vault_handler).layer(demo_guard.clone()),
            )
            .route(
                "/api/v1/vaults/{vault_id}/retry",
                post(retry_vault_handler).layer(demo_guard.clone()),
            )
            .route(
                "/api/v1/vaults/{vault_id}/notes",
                post(vault_scoped_create_note_handler).layer(demo_guard.clone()),
            )
            .route(
                "/api/v1/vaults/{vault_id}/notes/{slug}",
                get(vault_scoped_note_handler).merge(
                    put(vault_scoped_update_note_handler)
                        .delete(vault_scoped_delete_note_handler)
                        .layer(demo_guard.clone()),
                ),
            )
            .route(
                "/api/v1/vaults/{vault_id}/notes/{slug}/rename",
                patch(vault_scoped_rename_note_handler).layer(demo_guard.clone()),
            )
            .route(
                "/api/v1/vaults/{vault_id}/notes/{slug}/move",
                patch(vault_scoped_move_note_handler).layer(demo_guard.clone()),
            )
            .route(
                "/api/v1/vaults/{vault_id}/notes/{slug}/move-rename",
                patch(vault_scoped_move_rename_note_handler).layer(demo_guard.clone()),
            )
            .route(
                "/api/v1/vaults/{vault_id}/notes/{slug}/archive",
                patch(vault_scoped_archive_note_handler).layer(demo_guard.clone()),
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
            )
            // Grouped with mutation-related routes (not #109's exposed safe-read
            // list) since it is write-capability discovery, not content
            // browsing; gated the same as the mutations it describes.
            .route(
                "/api/v1/vaults/{vault_id}/write-capabilities",
                get(vault_scoped_write_capabilities_handler).layer(demo_guard.clone()),
            )
            // #100: one-or-all collection reads and search. `{vault_id}` here
            // is a Vault-or-`all` scope (parsed by `parse_vault_scope`); the
            // path parameter keeps the name `vault_id` for every route in
            // this group because axum's router requires one consistent
            // parameter name per path position — the segment is a Vault ID in
            // every sibling route above, and `all` is simply the one
            // additional value this group's handlers accept for it.
            .route(
                "/api/v1/vaults/{vault_id}/tree",
                get(vault_scope_tree_handler),
            )
            .route(
                "/api/v1/vaults/{vault_id}/recent",
                get(vault_scope_recent_handler),
            )
            .route(
                "/api/v1/vaults/{vault_id}/stats",
                get(vault_scope_stats_handler),
            )
            .route(
                "/api/v1/vaults/{vault_id}/graph",
                get(vault_scope_graph_handler),
            )
            .route(
                "/api/v1/vaults/{vault_id}/search",
                get(vault_scope_search_handler),
            );
        match web_bearer_token.clone() {
            Some(token) => vaults_v1.layer(axum::middleware::from_fn_with_state(
                WebToken(token),
                require_web_token,
            )),
            None => vaults_v1,
        }
    };

    // Vault-scoped attachment upload sits outside `vaults_v1`'s web-token-only
    // auth, mirroring the legacy `/api/attachment` route it replaces: an MCP
    // agent that already holds the MCP bearer token can use it directly,
    // without provisioning the separate web token just for this one route. It
    // still accepts the web token too, since the web UI's paste-to-upload flow
    // hits the same endpoint. #109: mounted in demo mode too, like the rest of
    // this router, but gated by `reject_demo_mutation` since it is a content
    // mutation.
    let vault_attachment = Router::new()
        .route(
            "/api/v1/vaults/{vault_id}/attachments",
            post(vault_scoped_upload_attachment_handler)
                .layer(DefaultBodyLimit::max(attachment_body_limit))
                .layer(demo_guard.clone()),
        )
        .layer(axum::middleware::from_fn_with_state(
            WebOrLiveMcpToken {
                web: web_bearer_token.clone(),
                runtime_config: state.runtime_config.clone(),
            },
            require_web_or_live_mcp_token,
        ));

    Router::new()
        .route("/health", get(health_handler))
        .route("/ready", get(readiness_handler))
        .route("/api/startup-status", get(startup_status_handler))
        .route("/api/vault-status", get(vault_status_handler))
        .merge(model_setup)
        .merge(settings)
        .merge(vaults_v1)
        .merge(vault_attachment)
        .merge(mcp)
        .route("/", get(spa_index_handler))
        // Canonical Vault-qualified browser Note URL (issue #62): unambiguous
        // when multiple Vaults contain the same slug. The legacy slug-only
        // `/n/{slug}` route is retired in #101 along with the rest of the
        // unscoped API — frontend consumption of this route is #67.
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

/// Demo mode (#109) publishes the whole enabled Vault collection as public
/// read-only: every content mutation and Vault-control route stays mounted
/// and reachable — unlike settings/setup, which stay absent (`404`) as
/// operator-only surfaces — but refuses with the structured `403
/// demo_read_only` error before any state changes. Applied per mutating
/// route/method in `build_router` so a GET read sharing a path with a
/// mutating verb (e.g. `POST /api/v1/vaults`, `PUT
/// .../notes/{slug}`) is unaffected.
async fn reject_demo_mutation(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    if state.demo_mode {
        return demo_read_only_response();
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

    fn attachment_upload_request(
        vault_id: &str,
        target_relative_path: &str,
        token: Option<&str>,
    ) -> Request<Body> {
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
            .uri(format!("/api/v1/vaults/{vault_id}/attachments"))
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
        let (app, tmp) = app_for_tests_with_web_and_mcp_auth(
            Some(Arc::from("web-secret")),
            Some("mcp-secret".to_string()),
        );
        let vault_id = create_vault_with_files_using_token(
            &app,
            "Attachments",
            &tmp.path().join("attachments"),
            &[],
            0,
            Some("web-secret"),
        )
        .await;

        let no_token = app
            .clone()
            .oneshot(attachment_upload_request(
                &vault_id,
                "Attachments/no-token.png",
                None,
            ))
            .await
            .expect("response");
        assert_eq!(no_token.status(), StatusCode::UNAUTHORIZED);

        let wrong_token = app
            .clone()
            .oneshot(attachment_upload_request(
                &vault_id,
                "Attachments/wrong-token.png",
                Some("not-a-real-token"),
            ))
            .await
            .expect("response");
        assert_eq!(wrong_token.status(), StatusCode::UNAUTHORIZED);

        let with_web_token = app
            .clone()
            .oneshot(attachment_upload_request(
                &vault_id,
                "Attachments/via-web-token.png",
                Some("web-secret"),
            ))
            .await
            .expect("response");
        assert_eq!(with_web_token.status(), StatusCode::OK);

        let with_mcp_token = app
            .oneshot(attachment_upload_request(
                &vault_id,
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
        // worked for the attachment route while MCP write mode is off must
        // keep working.
        let (app, tmp) = app_for_tests_with_web_and_mcp_auth_and_write_mode(
            Some(Arc::from("web-secret")),
            Some("mcp-secret".to_string()),
            false,
        );
        let vault_id = create_vault_with_files_using_token(
            &app,
            "Attachments",
            &tmp.path().join("attachments"),
            &[],
            0,
            Some("web-secret"),
        )
        .await;

        let response = app
            .oneshot(attachment_upload_request(
                &vault_id,
                "Attachments/via-mcp-token.png",
                Some("mcp-secret"),
            ))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn attachment_route_open_when_no_token_configured() {
        let (app, tmp) = app_for_tests_with_web_and_mcp_auth(None, None);
        let vault_id =
            create_vault_with_files(&app, "Attachments", &tmp.path().join("attachments"), &[], 0)
                .await;

        let response = app
            .oneshot(attachment_upload_request(
                &vault_id,
                "Attachments/open.png",
                None,
            ))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn mcp_settings_apply_atomically_and_rotate_attachment_authorization() {
        let (app, tmp, state) = app_for_tests_with_state();
        let vault_id =
            create_vault_with_files(&app, "Mcp", &tmp.path().join("mcp-attachments"), &[], 0).await;

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
            .oneshot(attachment_upload_request(
                &vault_id,
                "Attachments/rejected.png",
                None,
            ))
            .await
            .expect("response");
        assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);

        let first_token = app
            .clone()
            .oneshot(attachment_upload_request(
                &vault_id,
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
                &vault_id,
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
                &vault_id,
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
                    .body(Body::from(format!(
                        r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"import_attachment","arguments":{{"vault_id":"{vault_id}","target_relative_path":"Attachments/limited.png","content":"cG5nLWJ5dGVz"}}}}}}"#,
                    )))
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
                &vault_id,
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
                &vault_id,
                "Attachments/old-token.png",
                Some("first-token"),
            ))
            .await
            .expect("response");
        assert_eq!(old_token.status(), StatusCode::UNAUTHORIZED);

        let new_token = app
            .oneshot(attachment_upload_request(
                &vault_id,
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
    }

    #[tokio::test]
    async fn vault_scoped_pull_only_vault_disables_mutation() {
        // A Pull-only Vault's `capabilities.mutate` is false regardless of local
        // content (issue #62): no real Git remote traffic is needed to prove
        // the adapter's `ensure_mutable` gate, but the registry still requires
        // `repository_path` to be a real Git working checkout to accept an
        // `existing_git` source at all.
        let (app, tmp, _state) = app_for_tests_with_web_auth(None);
        let repository_path = tmp.path().join("pull-only-repo");
        std::fs::create_dir_all(&repository_path).expect("create repo directory");
        git2::Repository::init(&repository_path).expect("init git repo");
        std::fs::write(repository_path.join("Home.md"), "# Home\n").expect("write note");
        let created = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "expected_registry_revision": 0,
                            "name": "PullOnly",
                            "enabled": true,
                            "source": {
                                "type": "existing_git",
                                "repository_path": repository_path.to_string_lossy(),
                                "repository_url": "https://example.test/vault.git",
                                "branch": null,
                                "vault_subdirectory": null,
                                "mode": "pull_only",
                            },
                            "exclude_patterns": [],
                        })
                        .to_string(),
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(created.status(), StatusCode::CREATED);
        let vault_id = json_body(created).await["vault"]["vault_id"]
            .as_str()
            .expect("vault id")
            .to_string();

        let capabilities = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{vault_id}/write-capabilities"))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(capabilities.status(), StatusCode::OK);
        let payload = json_body(capabilities).await;
        assert_eq!(payload["enabled"], false);
        assert!(
            payload["warnings"]
                .as_array()
                .expect("warnings")
                .iter()
                .any(|warning| warning
                    .as_str()
                    .unwrap_or("")
                    .contains("do not allow mutation"))
        );

        let mutation = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{vault_id}/notes"))
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"relative_path":"Blocked.md","content":"no"}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(mutation.status(), StatusCode::CONFLICT);
        assert_eq!(json_body(mutation).await["code"], "capability_unavailable");
        assert!(!repository_path.join("Blocked.md").exists());
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
            .oneshot(
                Request::builder()
                    .uri("/ready")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(readiness.status(), StatusCode::SERVICE_UNAVAILABLE);
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
    async fn vault_scoped_resolve_batch_marks_archived_notes() {
        let (app, tmp, _state) = app_for_tests_with_web_auth(None);
        let vault_root = tmp.path().join("archiving");
        let vault_id = create_vault_with_files(
            &app,
            "Archiving",
            &vault_root,
            &[
                ("Home.md", "# Home\n"),
                ("90-archive/Old Setup.md", "# Old Setup\n"),
            ],
            0,
        )
        .await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{vault_id}/resolve-batch"))
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"targets":["Home","90-archive/Old Setup"]}"#))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let payload = json_body(response).await;
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
        let guarded_route = "/api/v1/vaults/00000000-0000-4000-8000-000000000000/notes/home";

        let no_token = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(guarded_route)
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

        // Authorized but the Vault does not exist: proves the token gate was
        // satisfied rather than short-circuiting before the handler ran.
        let with_header = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(guarded_route)
                    .method("GET")
                    .header("authorization", "Bearer secret-token")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(with_header.status(), StatusCode::NOT_FOUND);

        let with_query = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("{guarded_route}?access_token=secret-token"))
                    .method("GET")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(with_query.status(), StatusCode::NOT_FOUND);

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
    async fn vault_scoped_write_capabilities_route_reports_enabled() {
        let (app, tmp, _state) = app_for_tests_with_web_auth(None);
        let vault_id =
            create_vault_with_files(&app, "Writable", &tmp.path().join("writable"), &[], 0).await;
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{vault_id}/write-capabilities"))
                    .method("GET")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let payload = json_body(response).await;
        assert_eq!(payload["vault_id"], vault_id);
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
    async fn vault_scoped_write_capabilities_requires_web_token() {
        let (app, tmp, _state) = app_for_tests_with_web_auth(Some(Arc::from("secret-token")));
        let vault_id = create_vault_with_files_using_token(
            &app,
            "Writable",
            &tmp.path().join("writable"),
            &[],
            0,
            Some("secret-token"),
        )
        .await;

        let unauthorized = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{vault_id}/write-capabilities"))
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
                    .uri(format!("/api/v1/vaults/{vault_id}/write-capabilities"))
                    .method("GET")
                    .header("authorization", "Bearer secret-token")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(authorized.status(), StatusCode::OK);
        let payload = json_body(authorized).await;
        assert_eq!(payload["enabled"], true);
        assert!(payload["warnings"].as_array().expect("warnings").is_empty());
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
    async fn vault_scoped_write_capabilities_reports_disabled_for_read_only_vault() {
        let (app, tmp, _state) = app_for_tests_with_web_auth(None);
        let vault_root = tmp.path().join("read-only");
        let vault_id = create_vault_with_files(&app, "ReadOnly", &vault_root, &[], 0).await;
        let original_permissions = std::fs::metadata(&vault_root)
            .expect("vault metadata")
            .permissions();
        let mut read_only_permissions = original_permissions.clone();
        read_only_permissions.set_readonly(true);
        std::fs::set_permissions(&vault_root, read_only_permissions).expect("make vault read-only");

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{vault_id}/write-capabilities"))
                    .method("GET")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        std::fs::set_permissions(&vault_root, original_permissions)
            .expect("restore vault permissions");

        assert_eq!(response.status(), StatusCode::OK);
        let payload = json_body(response).await;
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
    async fn vault_scoped_uploads_attachment_into_vault() {
        let (app, tmp, _state) = app_for_tests_with_web_auth(None);
        let vault_root = tmp.path().join("attachments");
        let vault_id = create_vault_with_files(&app, "Attachments", &vault_root, &[], 0).await;
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
                    .uri(format!("/api/v1/vaults/{vault_id}/attachments"))
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
        let json = json_body(response).await;
        assert_eq!(json["vault_id"], vault_id);
        assert_eq!(
            json["attachment"]["relative_path"],
            "Attachments/pasted.png"
        );
        assert_eq!(json["attachment"]["size_bytes"], 9);
        assert_eq!(
            std::fs::read(vault_root.join("Attachments/pasted.png")).expect("file"),
            b"png-bytes"
        );
    }

    #[tokio::test]
    async fn vault_scoped_attachment_accepts_upload_between_2mb_and_configured_max() {
        // The default McpConfig caps attachments at 10 MB. A 3 MB upload is well
        // within that, but exceeds axum's built-in 2 MB body limit — without an
        // explicit DefaultBodyLimit the framework rejects it before the handler
        // (and its real size check) ever runs.
        let (app, tmp, _state) = app_for_tests_with_web_auth(None);
        let vault_root = tmp.path().join("big-attachments");
        let vault_id = create_vault_with_files(&app, "BigAttachments", &vault_root, &[], 0).await;
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
                    .uri(format!("/api/v1/vaults/{vault_id}/attachments"))
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
            std::fs::read(vault_root.join("Attachments/big.png"))
                .expect("file")
                .len(),
            3 * 1024 * 1024
        );
    }

    #[tokio::test]
    async fn vault_scoped_update_note_rejects_stale_hash() {
        let (app, tmp, _state) = app_for_tests_with_web_auth(None);
        let vault_id = create_vault_with_files(
            &app,
            "Notes",
            &tmp.path().join("notes"),
            &[("Home.md", "# Home\n")],
            0,
        )
        .await;

        let note_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{vault_id}/notes/home"))
                    .method("GET")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        let hash = json_body(note_response).await["note"]["content_hash"]
            .as_str()
            .expect("hash")
            .to_string();

        let update = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{vault_id}/notes/home"))
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
        assert_eq!(json_body(update).await["vault_id"], vault_id);

        let stale = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{vault_id}/notes/home"))
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
        assert_eq!(json_body(stale).await["code"], "write_conflict");
    }

    #[tokio::test]
    async fn vault_scoped_update_note_rejects_payload_missing_expected_hash() {
        let (app, tmp, _state) = app_for_tests_with_web_auth(None);
        let vault_id = create_vault_with_files(
            &app,
            "Notes",
            &tmp.path().join("notes"),
            &[("Home.md", "# Home\n")],
            0,
        )
        .await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{vault_id}/notes/home"))
                    .method("PUT")
                    .header("content-type", "application/json")
                    .body(Body::from(r##"{"content":"# Home\nupdated\n"}"##))
                    .expect("request"),
            )
            .await
            .expect("response");

        // Well-formed JSON missing a required field is a 422 (Unprocessable
        // Entity) — the real status axum's Json extractor reports. write_payload
        // preserves it instead of masking every rejection as 400.
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn vault_scoped_create_note_oversized_json_body_reports_413_not_400() {
        // A JSON write body over axum's 2 MB limit is a length-limit rejection
        // (413), not a malformed-body one (400). write_payload must preserve the
        // rejection's real status for clients/proxies that key off status codes.
        let (app, tmp, _state) = app_for_tests_with_web_auth(None);
        let vault_id =
            create_vault_with_files(&app, "Notes", &tmp.path().join("notes"), &[], 0).await;
        let big = "x".repeat(3 * 1024 * 1024);
        let body = format!(r#"{{"relative_path":"Big.md","content":"{big}"}}"#);
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{vault_id}/notes"))
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
    async fn vault_scoped_create_note_rejects_a_noise_path() {
        // A note written to this Vault's own noise-exclusion pattern would be
        // indexed away; the create route must refuse it, matching the MCP write
        // path and the legacy single-Vault write API.
        let (app, tmp, _state) = app_for_tests_with_web_auth(None);
        let vault_root = tmp.path().join("noise");
        let vault_id = create_vault_with_files(&app, "Noise", &vault_root, &[], 0).await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{vault_id}/notes"))
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
        assert_eq!(json_body(response).await["code"], "noise_excluded_write");
        assert!(!vault_root.join("Notes/scratch.tmp").exists());
    }

    #[tokio::test]
    async fn vault_scoped_move_note_rejects_a_vault_owned_noise_path() {
        // Moving an already-indexed note into a Vault-configured exclude pattern
        // would make it disappear on the next read. Every write target, not just
        // creates, must be checked against that Vault's own exclude patterns —
        // not the legacy instance-wide HATCHDOOR_EXCLUDE setting.
        let (app, tmp, _state) = app_for_tests_with_web_auth(None);
        let vault_root = tmp.path().join("noise-move");
        std::fs::create_dir_all(&vault_root).expect("create vault directory");
        std::fs::write(vault_root.join("Home.md"), "# Home\n").expect("write note");
        let created = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "expected_registry_revision": 0,
                            "name": "NoiseMove",
                            "enabled": true,
                            "source": {"type": "local", "path": vault_root.to_string_lossy()},
                            "exclude_patterns": [".trash/"],
                        })
                        .to_string(),
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        let vault_id = json_body(created).await["vault"]["vault_id"]
            .as_str()
            .expect("vault id")
            .to_string();
        let hash = crate::cache::parse::content_hash("# Home\n");

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{vault_id}/notes/home/move"))
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
        assert_eq!(json_body(response).await["code"], "noise_excluded_write");
        assert!(vault_root.join("Home.md").exists());
        assert!(!vault_root.join(".trash/Home.md").exists());
    }

    #[tokio::test]
    async fn vault_scoped_move_note_rebuilds_the_index_from_this_vaults_own_content() {
        // Write routes rebuild a short-lived index for slug/path work, scoped to
        // this Vault's own directory — unrelated Vaults or the legacy instance
        // config must not affect it. A note absent from this Vault's directory
        // is simply not found.
        let (app, tmp, _state) = app_for_tests_with_web_auth(None);
        let vault_id =
            create_vault_with_files(&app, "Notes", &tmp.path().join("notes"), &[], 0).await;
        let hash = crate::cache::parse::content_hash("# Ignored\n");

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{vault_id}/notes/ignored"))
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
    async fn vault_scoped_archive_note_rejects_a_vault_owned_noise_path() {
        let (app, tmp, _state) = app_for_tests_with_web_auth(None);
        let vault_root = tmp.path().join("noise-archive");
        std::fs::create_dir_all(&vault_root).expect("create vault directory");
        std::fs::write(vault_root.join("Home.md"), "# Home\n").expect("write note");
        let created = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "expected_registry_revision": 0,
                            "name": "NoiseArchive",
                            "enabled": true,
                            "source": {"type": "local", "path": vault_root.to_string_lossy()},
                            "exclude_patterns": ["90-archive/"],
                        })
                        .to_string(),
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        let vault_id = json_body(created).await["vault"]["vault_id"]
            .as_str()
            .expect("vault id")
            .to_string();
        let hash = crate::cache::parse::content_hash("# Home\n");

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{vault_id}/notes/home/archive"))
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
        assert_eq!(json_body(response).await["code"], "noise_excluded_write");
        assert!(vault_root.join("Home.md").exists());
        assert!(!vault_root.join("90-archive/Home.md").exists());
    }

    #[tokio::test]
    async fn vault_scoped_create_note_rejects_path_traversal() {
        let (app, tmp, _state) = app_for_tests_with_web_auth(None);
        let vault_id =
            create_vault_with_files(&app, "Notes", &tmp.path().join("notes"), &[], 0).await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{vault_id}/notes"))
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
    async fn vault_scoped_delete_note_rejects_stale_hash() {
        let (app, tmp, _state) = app_for_tests_with_web_auth(None);
        let vault_id = create_vault_with_files(
            &app,
            "Notes",
            &tmp.path().join("notes"),
            &[("Home.md", "# Home\n")],
            0,
        )
        .await;

        let note_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{vault_id}/notes/home"))
                    .method("GET")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        let original_hash = json_body(note_response).await["note"]["content_hash"]
            .as_str()
            .expect("hash")
            .to_string();

        let update = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{vault_id}/notes/home"))
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
                    .uri(format!("/api/v1/vaults/{vault_id}/notes/home"))
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
        assert_eq!(json_body(stale_delete).await["code"], "write_conflict");
    }

    #[tokio::test]
    async fn vault_scoped_creates_renames_moves_archives_and_deletes_note() {
        let (app, tmp, _state) = app_for_tests_with_web_auth(None);
        let vault_id =
            create_vault_with_files(&app, "Lifecycle", &tmp.path().join("lifecycle"), &[], 0).await;

        let create = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{vault_id}/notes"))
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
        let created = json_body(create).await;
        let created_object = created.as_object().expect("object");
        for field in [
            "vault_id",
            "ok",
            "slug",
            "relative_path",
            "content_hash",
            "quality_warnings",
            "rewritten_notes",
            "moved_assets",
            "trashed_path",
            "layer",
        ] {
            assert!(created_object.contains_key(field), "missing field {field}");
        }
        assert_eq!(created["ok"], true);
        assert_eq!(created["vault_id"], vault_id);
        let slug = created["slug"].as_str().expect("slug").to_string();
        let hash = created["content_hash"].as_str().expect("hash").to_string();

        let duplicate_create = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{vault_id}/notes"))
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
                    .uri(format!("/api/v1/vaults/{vault_id}/notes/{slug}/rename"))
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
        let renamed = json_body(rename).await;
        let renamed_slug = renamed["slug"].as_str().expect("renamed slug").to_string();
        let renamed_hash = renamed["content_hash"]
            .as_str()
            .expect("renamed hash")
            .to_string();

        let move_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/v1/vaults/{vault_id}/notes/{renamed_slug}/move"
                    ))
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
        let moved = json_body(move_response).await;
        let moved_slug = moved["slug"].as_str().expect("moved slug").to_string();
        let moved_hash = moved["content_hash"]
            .as_str()
            .expect("moved hash")
            .to_string();

        let archive = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/v1/vaults/{vault_id}/notes/{moved_slug}/archive"
                    ))
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
        let archived = json_body(archive).await;
        let archived_slug = archived["slug"]
            .as_str()
            .expect("archived slug")
            .to_string();
        let archived_hash = archived["content_hash"]
            .as_str()
            .expect("archived hash")
            .to_string();
        assert_eq!(archived["relative_path"], "90-archive/Renamed Note");
        assert_eq!(archived["layer"], serde_json::Value::Null);

        let delete = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{vault_id}/notes/{archived_slug}"))
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
    // Legacy unscoped application API: every route is retired in #101 with
    // no compatibility shim (issue #62). `refresh`/`diagnostics` are retired
    // with no Vault-scoped replacement (see module docs on
    // `handlers/diagnostics.rs` and this file's `vaults_v1` construction);
    // every other legacy route has a Vault-scoped equivalent proven above or
    // in #98/#99/#100's own tests.
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn legacy_unscoped_api_routes_are_absent() {
        let (app, _tmp, _state) = app_for_tests_with_web_auth(None);
        let get_routes = [
            "/api/tree",
            "/api/vault-events",
            "/api/recently-modified",
            "/api/note/home",
            "/api/note/home/download",
            "/api/note/home/links",
            "/api/resolve?target=Home",
            "/api/search?q=home",
            "/api/stats",
            "/api/diagnostics",
            "/api/graph",
            "/api/write-capabilities",
            "/vault-assets/diagram.png",
            "/n/home",
        ];
        for path in get_routes {
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
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "GET {path}");
        }

        // A non-GET/HEAD request to any unmatched path falls through to the
        // SPA fallback `ServeDir`, which itself answers `405` (it only serves
        // static files over GET/HEAD) rather than the router's own `404` —
        // still proof no route intercepts these methods to run the retired
        // handler.
        let post_routes = [
            "/api/note",
            "/api/resolve-batch",
            "/api/refresh",
            "/api/attachment",
        ];
        for path in post_routes {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .method("POST")
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_eq!(
                response.status(),
                StatusCode::METHOD_NOT_ALLOWED,
                "POST {path}"
            );
        }

        for method in ["PUT", "PATCH", "DELETE"] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/api/note/home")
                        .method(method)
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_eq!(
                response.status(),
                StatusCode::METHOD_NOT_ALLOWED,
                "{method} /api/note/home"
            );
        }

        let patch_routes = [
            "/api/note/home/rename",
            "/api/note/home/move",
            "/api/note/home/archive",
            "/api/note/home/move-rename",
        ];
        for path in patch_routes {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .method("PATCH")
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_eq!(
                response.status(),
                StatusCode::METHOD_NOT_ALLOWED,
                "PATCH {path}"
            );
        }
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
    async fn vaults_v1_discovery_and_events_reachable_unauthenticated_in_demo_mode() {
        // #109: demo mode publishes the whole enabled Vault collection as
        // public read-only, so discovery (and, by the same posture, the
        // collection event stream) must stay reachable with no token at all
        // — unlike the old #101-era posture where this entire group was
        // absent (404) in demo mode.
        let (app, _tmp, _state) = app_for_tests_with_web_auth_and_demo_mode(None, true);

        let discovery = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(discovery.status(), StatusCode::OK);
        let body = json_body(discovery).await;
        assert_eq!(body["vaults"], serde_json::json!([]));

        let events = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults/events")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(events.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn vaults_v1_collection_mutations_refuse_with_demo_read_only_in_demo_mode() {
        // Every collection-management and manual-Git-control route (#109) —
        // Vault-control operations, not content mutations, but still gated
        // the same way — must refuse before touching the registry rather
        // than being absent, so a demo client gets a clear structured reason
        // rather than the ambiguity of a 404.
        let (app, _tmp, _state) = app_for_tests_with_web_auth_and_demo_mode(None, true);
        let vault_id = "00000000-0000-4000-8000-000000000000";

        let create = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(create_vault_request_body(
                        "Demo",
                        std::path::Path::new("/does-not-matter"),
                        0,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(create.status(), StatusCode::FORBIDDEN);
        let create_body = json_body(create).await;
        assert_eq!(create_body["code"], "demo_read_only");

        for (method, uri) in [
            ("PATCH", format!("/api/v1/vaults/{vault_id}")),
            ("DELETE", format!("/api/v1/vaults/{vault_id}")),
            ("POST", format!("/api/v1/vaults/{vault_id}/enable")),
            ("POST", format!("/api/v1/vaults/{vault_id}/disable")),
            ("POST", format!("/api/v1/vaults/{vault_id}/sync")),
            ("POST", format!("/api/v1/vaults/{vault_id}/retry")),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(&uri)
                        .method(method)
                        .header("content-type", "application/json")
                        .body(Body::from("{}"))
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_eq!(
                response.status(),
                StatusCode::FORBIDDEN,
                "{method} {uri} should refuse in demo mode"
            );
            let body = json_body(response).await;
            assert_eq!(body["code"], "demo_read_only", "{method} {uri}");
        }
    }

    #[tokio::test]
    async fn demo_mode_browses_an_enabled_vault_but_still_refuses_its_mutations() {
        // #109's core scenario: an enabled Vault (added directly to the
        // registry here, since demo mode itself refuses `POST
        // /api/v1/vaults`) is publicly browsable end-to-end — listed by
        // discovery, exact-readable by content — while its content mutations
        // still refuse with `demo_read_only`.
        let (app, tmp, state) = app_for_tests_with_web_auth_and_demo_mode(None, true);
        let vault_path = tmp.path().join("demo-vault");
        std::fs::create_dir_all(&vault_path).expect("create vault directory");
        std::fs::write(vault_path.join("Home.md"), "# Home\n\ndemo content\n").expect("write note");

        let snapshot = state
            .vault_registry
            .add(
                0,
                crate::vault_registry::NewVaultDefinition {
                    name: "Demo Vault".to_string(),
                    enabled: true,
                    source: crate::vault_registry::VaultSource::Local {
                        path: vault_path.clone(),
                    },
                    exclude_patterns: Vec::new(),
                    https_credentials: None,
                },
            )
            .expect("add vault to registry");
        state
            .vaults
            .reconcile_and_reconstruct(
                &state.vault_registry,
                &snapshot,
                &state.vault_work,
                &state.managed_git,
            )
            .await;
        let vault_id = snapshot.vault_ids().next().expect("one vault id");

        let discovery = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(discovery.status(), StatusCode::OK);
        let discovery_body = json_body(discovery).await;
        let vaults = discovery_body["vaults"].as_array().expect("vaults array");
        assert_eq!(vaults.len(), 1);
        assert_eq!(vaults[0]["vault_id"], vault_id.to_string());

        let note = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{vault_id}/notes/home"))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(note.status(), StatusCode::OK);
        let note_body = json_body(note).await;
        assert!(
            note_body["note"]["content"]
                .as_str()
                .unwrap()
                .contains("demo content")
        );

        let mutation = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{vault_id}/notes/home"))
                    .method("PUT")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "content": "# Home\n\ntampered\n",
                            "expected_content_hash": "does-not-matter",
                        })
                        .to_string(),
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(mutation.status(), StatusCode::FORBIDDEN);
        let mutation_body = json_body(mutation).await;
        assert_eq!(mutation_body["code"], "demo_read_only");

        // The mutation truly never ran: the note on disk is unchanged.
        let on_disk = std::fs::read_to_string(vault_path.join("Home.md")).expect("read note");
        assert!(on_disk.contains("demo content"));
        assert!(!on_disk.contains("tampered"));
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

    /// Like `create_vault_with_files`, for fixtures where `/api/v1/vaults`
    /// itself requires a web bearer token.
    async fn create_vault_with_files_using_token(
        app: &Router,
        name: &str,
        root: &std::path::Path,
        files: &[(&str, &str)],
        revision: u64,
        token: Option<&str>,
    ) -> String {
        std::fs::create_dir_all(root).expect("create vault directory");
        for (relative_path, contents) in files {
            let path = root.join(relative_path);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("create parent dir");
            std::fs::write(path, contents).expect("write note");
        }
        let mut builder = Request::builder()
            .uri("/api/v1/vaults")
            .method("POST")
            .header("content-type", "application/json");
        if let Some(token) = token {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        let created = app
            .clone()
            .oneshot(
                builder
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
    async fn vault_scoped_routes_require_web_token_when_configured() {
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
    }

    #[tokio::test]
    async fn vault_scoped_content_reads_reachable_unauthenticated_in_demo_mode() {
        // #109: exact reads and their contained resources stay reachable in
        // demo mode with no token — reaching the real handler (proven by the
        // structured `vault_not_found` body, not a generic router/static-file
        // 404) rather than the old #101-era posture where this whole group
        // was absent.
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
        let demo_body = json_body(demo_response).await;
        assert_eq!(demo_body["code"], "vault_not_found");
    }

    #[tokio::test]
    async fn vault_scoped_content_mutations_refuse_with_demo_read_only_in_demo_mode() {
        // Every content-mutation, attachment-upload, and write-capability
        // route in `vault_write.rs` (#109) must refuse before touching the
        // Vault's Markdown, rather than being absent.
        let (app, _tmp, _state) = app_for_tests_with_web_auth_and_demo_mode(None, true);
        let vault_id = "00000000-0000-4000-8000-000000000000";

        for (method, uri) in [
            ("POST", format!("/api/v1/vaults/{vault_id}/notes")),
            ("PUT", format!("/api/v1/vaults/{vault_id}/notes/home")),
            ("DELETE", format!("/api/v1/vaults/{vault_id}/notes/home")),
            (
                "PATCH",
                format!("/api/v1/vaults/{vault_id}/notes/home/rename"),
            ),
            (
                "PATCH",
                format!("/api/v1/vaults/{vault_id}/notes/home/move"),
            ),
            (
                "PATCH",
                format!("/api/v1/vaults/{vault_id}/notes/home/move-rename"),
            ),
            (
                "PATCH",
                format!("/api/v1/vaults/{vault_id}/notes/home/archive"),
            ),
            (
                "GET",
                format!("/api/v1/vaults/{vault_id}/write-capabilities"),
            ),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(&uri)
                        .method(method)
                        .header("content-type", "application/json")
                        .body(Body::from("{}"))
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_eq!(
                response.status(),
                StatusCode::FORBIDDEN,
                "{method} {uri} should refuse in demo mode"
            );
            let body = json_body(response).await;
            assert_eq!(body["code"], "demo_read_only", "{method} {uri}");
        }

        let attachment_response = app
            .oneshot(attachment_upload_request(
                vault_id,
                "Attachments/demo.png",
                None,
            ))
            .await
            .expect("response");
        assert_eq!(attachment_response.status(), StatusCode::FORBIDDEN);
        let attachment_body = json_body(attachment_response).await;
        assert_eq!(attachment_body["code"], "demo_read_only");
    }

    // -----------------------------------------------------------------
    // #100: /api/v1/vaults/{scope}/{tree,recent,stats,graph,search} —
    // one-or-all collection reads and search
    // -----------------------------------------------------------------

    /// Collection reads (unlike exact reads) serve only the shared disposable
    /// cache's already-published Vault snapshot. `VaultWorkKind::Index`
    /// dispatch — the background turn that would build and publish that
    /// snapshot after a real Vault creation — is explicitly not yet
    /// implemented (`src/server.rs`'s dispatch loop returns
    /// `vault_work_kind_not_yet_implemented` for it, and the test harness's
    /// `_vault_worker` is never driven regardless), so a Vault created only
    /// through the HTTP surface never becomes a fresh collection-read
    /// participant in this test process. Publish its snapshot directly
    /// through the same shared cache `VaultReadCore`/`VaultSearchCore` read
    /// from, mirroring what `vault_read.rs`'s and `search/vault_scoped.rs`'s
    /// own unit tests already do, and what the eventual indexing dispatch
    /// will do in production.
    fn publish_vault_snapshot(state: &AppState, vault_id: &str, vault_root: &std::path::Path) {
        use std::str::FromStr;
        let vault_id = crate::vault_registry::VaultId::from_str(vault_id).expect("parse vault id");
        let index = crate::vault::VaultIndex::build(vault_root).expect("build index");
        state
            .startup_sqlite
            .replace_vault_snapshot(vault_id, &index, state.embedder.as_ref())
            .expect("publish snapshot");
    }

    #[tokio::test]
    async fn vault_scope_tree_stats_graph_group_data_per_vault_for_all_scope() {
        let (app, tmp, _state) = app_for_tests_with_web_auth(None);
        let first_root = tmp.path().join("first");
        let first = create_vault_with_files(
            &app,
            "First",
            &first_root,
            &[
                ("Home.md", "# Home\n\nfirst\n\n[[Shared]]"),
                ("Shared.md", "# Shared"),
            ],
            0,
        )
        .await;
        let second_root = tmp.path().join("second");
        let second = create_vault_with_files(
            &app,
            "Second",
            &second_root,
            &[
                ("Home.md", "# Home\n\nsecond\n\n[[Shared]]"),
                ("Shared.md", "# Shared"),
            ],
            1,
        )
        .await;
        publish_vault_snapshot(&_state, &first, &first_root);
        publish_vault_snapshot(&_state, &second, &second_root);

        let tree = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults/all/tree")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(tree.status(), StatusCode::OK);
        let tree_body = json_body(tree).await;
        assert_eq!(tree_body["scope"], "all");
        assert_eq!(tree_body["partial"], false);
        assert_eq!(tree_body["data"].as_array().expect("tree data").len(), 2);
        assert_eq!(
            tree_body["participants"]
                .as_array()
                .expect("participants")
                .len(),
            2
        );
        for vault_tree in tree_body["data"].as_array().unwrap() {
            let vault_id = vault_tree["vault_id"].as_str().expect("vault_id");
            for note in vault_tree["tree"]["notes"].as_array().unwrap() {
                assert_eq!(note["vault_id"], vault_id);
            }
        }

        let stats = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults/all/stats")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(stats.status(), StatusCode::OK);
        let stats_body = json_body(stats).await;
        assert_eq!(stats_body["data"].as_array().expect("stats data").len(), 2);
        assert!(
            stats_body["data"]
                .as_array()
                .unwrap()
                .iter()
                .all(|entry| entry["note_count"] == 2)
        );

        let graph = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults/all/graph")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(graph.status(), StatusCode::OK);
        let graph_body = json_body(graph).await;
        let graph_data = graph_body["data"].as_array().expect("graph data");
        assert_eq!(graph_data.len(), 2);
        for vault_graph in graph_data {
            let vault_id = vault_graph["vault_id"].as_str().expect("vault_id");
            for edge in vault_graph["edges"].as_array().unwrap() {
                // No cross-Vault graph edges.
                assert_eq!(edge["vault_id"], vault_id);
            }
        }
        let vault_ids: std::collections::BTreeSet<_> = graph_data
            .iter()
            .map(|entry| entry["vault_id"].as_str().unwrap().to_string())
            .collect();
        assert!(vault_ids.contains(&first));
        assert!(vault_ids.contains(&second));
    }

    #[tokio::test]
    async fn vault_scope_recent_and_search_flatten_across_vaults_and_honour_one_scope() {
        let (app, tmp, _state) = app_for_tests_with_web_auth(None);
        let first_root = tmp.path().join("first");
        let first = create_vault_with_files(
            &app,
            "First",
            &first_root,
            &[("Home.md", "# Home\n\nshared term")],
            0,
        )
        .await;
        let second_root = tmp.path().join("second");
        let second = create_vault_with_files(
            &app,
            "Second",
            &second_root,
            &[("Home.md", "# Home\n\nshared term")],
            1,
        )
        .await;
        publish_vault_snapshot(&_state, &first, &first_root);
        publish_vault_snapshot(&_state, &second, &second_root);

        let recent_all = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults/all/recent")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(recent_all.status(), StatusCode::OK);
        let recent_all_body = json_body(recent_all).await;
        assert_eq!(
            recent_all_body["data"]
                .as_array()
                .expect("recent data")
                .len(),
            2
        );

        let recent_one = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{first}/recent"))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(recent_one.status(), StatusCode::OK);
        let recent_one_body = json_body(recent_one).await;
        let recent_one_data = recent_one_body["data"].as_array().expect("recent one data");
        assert_eq!(recent_one_data.len(), 1);
        assert_eq!(recent_one_data[0]["vault_id"], first);

        let search_all = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults/all/search?q=shared&mode=keyword")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(search_all.status(), StatusCode::OK);
        let search_all_body = json_body(search_all).await;
        assert_eq!(search_all_body["scope"], "all");
        let results = search_all_body["data"]["results"]
            .as_array()
            .expect("search results");
        assert_eq!(results.len(), 2);
        let mut result_vault_ids: Vec<String> = results
            .iter()
            .map(|result| result["vault_id"].as_str().unwrap().to_string())
            .collect();
        result_vault_ids.sort();
        let mut expected = vec![first.clone(), second.clone()];
        expected.sort();
        assert_eq!(result_vault_ids, expected);

        let search_one = app
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/v1/vaults/{second}/search?q=shared&mode=keyword"
                    ))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(search_one.status(), StatusCode::OK);
        let search_one_body = json_body(search_one).await;
        let one_results = search_one_body["data"]["results"]
            .as_array()
            .expect("search one results");
        assert_eq!(one_results.len(), 1);
        assert_eq!(one_results[0]["vault_id"], second);
    }

    #[tokio::test]
    async fn vault_scope_zero_enabled_vaults_returns_a_complete_empty_envelope() {
        let (app, _tmp, _state) = app_for_tests_with_web_auth(None);

        for uri in [
            "/api/v1/vaults/all/tree",
            "/api/v1/vaults/all/recent",
            "/api/v1/vaults/all/stats",
            "/api/v1/vaults/all/graph",
            "/api/v1/vaults/all/search?q=anything",
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(uri)
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_eq!(response.status(), StatusCode::OK, "uri: {uri}");
            let body = json_body(response).await;
            assert_eq!(body["scope"], "all", "uri: {uri}");
            assert_eq!(body["partial"], false, "uri: {uri}");
            assert_eq!(
                body["participants"].as_array().expect("participants").len(),
                0,
                "uri: {uri}"
            );
            let data = &body["data"];
            let is_empty = data
                .as_array()
                .map(|array| array.is_empty())
                .unwrap_or_else(|| {
                    data["results"]
                        .as_array()
                        .expect("search results array")
                        .is_empty()
                });
            assert!(is_empty, "uri: {uri} data: {data}");
        }
    }

    #[tokio::test]
    async fn vault_scope_reports_structured_errors_for_invalid_scope_unknown_and_disabled_vault() {
        let (app, tmp, _state) = app_for_tests_with_web_auth(None);
        let vault_id = create_vault_with_files(
            &app,
            "Lifecycle",
            &tmp.path().join("lifecycle"),
            &[("Home.md", "# Home")],
            0,
        )
        .await;

        let invalid_scope = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults/not-a-scope/tree")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(invalid_scope.status(), StatusCode::BAD_REQUEST);
        assert_eq!(json_body(invalid_scope).await["code"], "invalid_scope");

        let unknown_id = "00000000-0000-4000-8000-000000000000";
        let unknown = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{unknown_id}/stats"))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
        assert_eq!(json_body(unknown).await["code"], "vault_not_found");

        let disable = app
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
        assert_eq!(disable.status(), StatusCode::OK);

        let disabled_read = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{vault_id}/graph"))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(disabled_read.status(), StatusCode::CONFLICT);
        assert_eq!(json_body(disabled_read).await["code"], "vault_disabled");
    }

    #[tokio::test]
    async fn vault_scope_search_validates_query_and_applies_layer_selection_independently() {
        let (app, tmp, _state) = app_for_tests_with_web_auth(None);
        let layered_root = tmp.path().join("layered");
        let layered = create_vault_with_files(
            &app,
            "Layered",
            &layered_root,
            &[
                ("sources/.hatchdoor-layer", "sources"),
                ("sources/Clipping.md", "# Clipping\n\nneedle"),
            ],
            0,
        )
        .await;
        let plain_root = tmp.path().join("plain");
        let plain = create_vault_with_files(
            &app,
            "Plain",
            &plain_root,
            &[("Home.md", "# Home\n\nneedle")],
            1,
        )
        .await;
        publish_vault_snapshot(&_state, &layered, &layered_root);
        publish_vault_snapshot(&_state, &plain, &plain_root);

        let missing_query = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults/all/search")
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

        let empty_query = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults/all/search?q=")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(empty_query.status(), StatusCode::BAD_REQUEST);
        assert_eq!(json_body(empty_query).await["code"], "invalid_search_query");

        let absent_layer = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults/all/search?q=needle&mode=keyword&layers=ghost")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(absent_layer.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            json_body(absent_layer).await["code"],
            "invalid_layer_selection"
        );

        let selected_layer = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults/all/search?q=needle&mode=keyword&layers=sources")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(selected_layer.status(), StatusCode::OK);
        let selected_layer_body = json_body(selected_layer).await;
        let results = selected_layer_body["data"]["results"]
            .as_array()
            .expect("results");
        assert_eq!(results.len(), 1);
        assert_ne!(results[0]["vault_id"], plain);
    }

    #[tokio::test]
    async fn vault_scope_routes_require_web_token_when_configured_and_reachable_unauthenticated_in_demo_mode()
     {
        let (app, _tmp, _state) = app_for_tests_with_web_auth(Some(Arc::from("secret")));

        let unauthorized = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults/all/tree")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let authorized = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults/all/tree")
                    .header("authorization", "Bearer secret")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(authorized.status(), StatusCode::OK);

        // #109: one-or-all collection reads are pure reads, so they stay
        // reachable with no token at all in demo mode, unlike the old
        // #101-era posture where this whole group was absent (404).
        let (demo_app, _demo_tmp, _demo_state) =
            app_for_tests_with_web_auth_and_demo_mode(None, true);
        let demo_response = demo_app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults/all/tree")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(demo_response.status(), StatusCode::OK);
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
