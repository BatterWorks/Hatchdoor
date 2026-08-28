//! HTTP server composition: router construction, startup security-posture
//! checks, and the `serve` run loop. Kept in the library (rather than the binary
//! root) so the HTTP surface is reachable from integration tests.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::Extension;
use axum::extract::DefaultBodyLimit;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post, put};
use axum::{Json, Router};
use tokio::sync::RwLock;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::{DefaultOnResponse, TraceLayer};
use tracing::{debug, error, info, warn};

use crate::app_state::AppState;
use crate::auth::{
    AssetReadTokens, WebOrLiveMcpToken, WebToken, require_web_or_live_mcp_read_token,
    require_web_or_live_mcp_token, require_web_token,
};
use crate::cache::SqliteCache;
use crate::config::AppConfig;
use crate::embed::{Embedder, FastembedEmbedder, RuntimeEmbedder};
use crate::git::{self, GitConfig};
use crate::handlers::{
    MAX_IN_MEMORY_UPLOAD_BYTES, create_vault_handler, demo_read_only_response,
    disable_vault_handler, disconnect_vault_handler, edit_vault_handler, enable_vault_handler,
    generate_mcp_token_handler, get_settings_handler, health_handler, list_vaults_handler,
    patch_settings_handler, refresh_vault_handler, retry_vault_handler, reveal_mcp_token_handler,
    reveal_web_token_handler, spa_index_handler, start_with_no_vaults_handler, sync_vault_handler,
    vault_collection_events_handler, vault_scope_graph_handler, vault_scope_recent_handler,
    vault_scope_search_handler, vault_scope_stats_handler, vault_scope_tree_handler,
    vault_scoped_archive_note_handler, vault_scoped_asset_handler,
    vault_scoped_create_note_handler, vault_scoped_delete_note_handler,
    vault_scoped_move_note_handler, vault_scoped_move_rename_note_handler,
    vault_scoped_note_download_handler, vault_scoped_note_handler, vault_scoped_note_links_handler,
    vault_scoped_rename_note_handler, vault_scoped_resolve_batch_handler,
    vault_scoped_resolve_handler, vault_scoped_stats_detail_handler,
    vault_scoped_update_note_handler, vault_scoped_upload_attachment_handler,
    vault_scoped_write_capabilities_handler,
};
use crate::mcp::{HatchdoorMcpTransport, McpConfig};
use crate::model_setup::{ModelSetup, SelectedModel};
use crate::runtime_config::{
    ConfigSnapshot, RuntimeConfig, live_settings_defaults, settings_file_path,
};
use crate::startup::StartupTracker;
use crate::vault_migration::{LegacyMigrationInput, LegacyMigrationOutcome, migrate_legacy_vault};
use crate::vault_registry::{VaultRegistryState, VaultRegistryStore};
use crate::vault_runtime::{
    VaultCollectionRuntime, VaultRuntime, VaultSource, dispatch_managed_git_turn,
    dispatch_vault_index_turn_with_progress,
};
use crate::vault_work::{VaultWorkCoordinator, VaultWorkError, VaultWorkKind, VaultWorkRequest};

/// Hosts that only accept connections from the local machine. Binding to any
/// other address exposes the port to the network.
fn is_loopback_host(host: &str) -> bool {
    matches!(host.trim(), "127.0.0.1" | "::1" | "[::1]" | "localhost")
}

/// Dependencies captured once for one worker turn. The runtime snapshot is
/// deliberately part of this value so an admitted operation observes one
/// immutable configuration view even if settings change later.
#[derive(Clone)]
struct VaultWorkDispatchContext {
    vaults: VaultCollectionRuntime,
    registry: VaultRegistryStore,
    work: VaultWorkCoordinator,
    managed_git: Arc<crate::git::ManagedGitScheduler>,
    cache: Arc<SqliteCache>,
    embedder: Arc<dyn Embedder>,
    runtime_snapshot: Arc<ConfigSnapshot>,
    author_name: String,
    author_email: String,
    startup: StartupTracker,
}

impl VaultWorkDispatchContext {
    async fn dispatch(self, request: VaultWorkRequest) -> Result<(), VaultWorkError> {
        match request.kind() {
            VaultWorkKind::Git => {
                dispatch_managed_git_turn(
                    &self.vaults,
                    &self.registry,
                    &self.work,
                    &self.managed_git,
                    &self.author_name,
                    &self.author_email,
                    request,
                )
                .await
            }
            VaultWorkKind::Index => {
                let embed_layers = self
                    .runtime_snapshot
                    .setting("HATCHDOOR_EMBED_LAYERS")
                    .map(|setting| crate::runtime_config::is_truthy(&setting.value))
                    .unwrap_or(true);
                let progress_startup = self.startup.clone();
                dispatch_vault_index_turn_with_progress(
                    &self.vaults,
                    self.cache,
                    self.embedder,
                    embed_layers,
                    Some(Arc::new(move |progress| {
                        progress_startup.set_indexing(progress);
                    })),
                    request,
                )
                .await
            }
            VaultWorkKind::Repair => Err(VaultWorkError::new(
                "vault_work_kind_not_yet_implemented",
                format!("{:?} dispatch is not implemented yet", request.kind()),
                false,
            )),
        }
    }
}

/// The instance-wide default commit identity for a Git turn, read from the
/// settings snapshot bound to that turn. A Vault's own configured identity
/// still overrides this (see `crate::git::config::resolve_commit_identity`).
fn git_author_defaults(snapshot: &ConfigSnapshot) -> (String, String) {
    (
        crate::git::config::non_empty_setting(snapshot, "HATCHDOOR_GIT_AUTHOR_NAME")
            .unwrap_or_else(|| "Hatchdoor".to_string()),
        crate::git::config::non_empty_setting(snapshot, "HATCHDOOR_GIT_AUTHOR_EMAIL")
            .unwrap_or_else(|| "hatchdoor@localhost".to_string()),
    )
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

/// Refuse to start while per-Vault settings are still set in the environment.
/// Once the registry owns them, an environment value is silently ignored: the
/// operator's `.env` and Hatchdoor's actual behavior disagree, and every later
/// change made in Settings looks overridden by a file that no longer has any
/// effect. Stopping is the only unambiguous signal.
///
/// `VAULT_PATH` is deliberately absent: Compose sets it on every deployment as
/// the container's vault mount, so refusing on it would refuse every start.
fn check_legacy_environment_posture(ignored_environment_keys: &[String]) -> Result<(), String> {
    use crate::config::{LegacyVaultEnvironmentKeyKind, legacy_vault_environment_key_kind};

    let migrated: Vec<&str> = ignored_environment_keys
        .iter()
        .map(String::as_str)
        .filter(|key| {
            legacy_vault_environment_key_kind(key) == Some(LegacyVaultEnvironmentKeyKind::Migrated)
        })
        .collect();
    let retired = ignored_environment_keys
        .iter()
        .filter(|key| {
            legacy_vault_environment_key_kind(key) == Some(LegacyVaultEnvironmentKeyKind::Retired)
        })
        .map(String::as_str)
        .collect::<Vec<_>>();
    if migrated.is_empty() && retired.is_empty() {
        return Ok(());
    }

    let mut message = String::new();
    if !migrated.is_empty() {
        message.push_str(&format!(
            "Hatchdoor has taken these settings over from your .env and stores them itself now: {}. \
             Remove them from your .env and start Hatchdoor again. To change them from now on, use \
             Settings, or the edit_vault MCP tool.",
            migrated.join(", ")
        ));
    }
    if !retired.is_empty() {
        if !message.is_empty() {
            message.push(' ');
        }
        message.push_str(&format!(
            "{} no longer does anything: Hatchdoor coalesces writes on its own now. Remove it too.",
            retired.join(", ")
        ));
    }
    Err(message)
}

/// `check_demo_mode_posture` above only inspects the legacy single-vault Git
/// sync of `config.vault_source`; it cannot see the durable Vault registry,
/// which is
/// loaded afterward and then activated by
/// `VaultCollectionRuntime::reconcile_and_reconstruct`. A registry Vault
/// configured for two-way Git sync would otherwise pass startup and later
/// have Git writeback scheduled for it while demo mode is enabled, missing
/// the same fail-fast refusal (#109). Read-only acquisition/pull
/// (`LocalHistory`/`PullOnly`) remains permitted, matching the accepted demo
/// boundary.
pub fn check_demo_mode_registry_posture(
    demo_mode: bool,
    snapshot: &crate::vault_registry::VaultRegistrySnapshot,
) -> Result<(), String> {
    if !demo_mode {
        return Ok(());
    }
    for definition in snapshot.definitions() {
        if !definition.enabled() {
            continue;
        }
        let mode = match definition.source() {
            crate::vault_registry::VaultSource::ExistingGit { mode, .. }
            | crate::vault_registry::VaultSource::ManagedGit { mode, .. } => Some(*mode),
            crate::vault_registry::VaultSource::Local { .. } => None,
        };
        if mode == Some(crate::vault_registry::VaultGitMode::TwoWay) {
            return Err(format!(
                "HATCHDOOR_DEMO_MODE=true is incompatible with Git writeback; Vault {} is \
                 registered with two-way Git mode; disable demo mode or change the Vault to \
                 pull-only/local-history for public demos.",
                definition.vault_id()
            ));
        }
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
    // This is merely the largest process-safe MCP body. The handler validates
    // token/origin, binds its live snapshot, and reapplies the method-aware
    // limit before collecting a request.
    let mcp_body_limit = McpConfig::maximum_request_body_limit();

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
    // tool list; `tools::dispatch` blocks content read/write/search tools
    // until the legacy embedding-model setup is ready, but Vault collection
    // discovery/management tools stay reachable throughout (#103), mirroring
    // `vaults_v1`'s own independence from that legacy readiness signal below.
    // The transport itself is rmcp-owned (ADR-17); the sub-router layers the
    // per-request authorization/body-limit middleware in front of it.
    let mcp_transport = HatchdoorMcpTransport::new(state.clone());
    let mcp = mcp_transport
        .router(&state)
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
                "/api/v1/vaults/start-with-no-vaults",
                post(start_with_no_vaults_handler).layer(demo_guard.clone()),
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
                "/api/v1/vaults/{vault_id}/refresh",
                post(refresh_vault_handler).layer(demo_guard.clone()),
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
            // Grouped with mutation-related routes (not #109's exposed safe-read
            // list) since it is write-capability discovery, not content
            // browsing; gated the same as the mutations it describes.
            .route(
                "/api/v1/vaults/{vault_id}/write-capabilities",
                get(vault_scoped_write_capabilities_handler).layer(demo_guard.clone()),
            )
            // #137: the rich, exact single-Vault statistics report the lean
            // `{vault_id}/stats` collection projection below cannot back. A
            // distinct nested path, not a query flag, keeping the exact/
            // collection route split this group already uses everywhere else.
            .route(
                "/api/v1/vaults/{vault_id}/stats/detail",
                get(vault_scoped_stats_detail_handler),
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
        // #109: demo mode publishes every enabled Vault's reads
        // unauthenticated even when this instance also has
        // `HATCHDOOR_WEB_BEARER_TOKEN` configured. Gating the whole group
        // behind `require_web_token` here would intercept an unauthenticated
        // demo request with a bare `401` before the mutation-specific
        // `demo_guard` (`reject_demo_mutation`) already layered on individual
        // routes above ever runs, which must be the one to answer mutations
        // with the stable `403 demo_read_only` refusal.
        match web_bearer_token.clone() {
            Some(token) if !state.demo_mode => vaults_v1.layer(
                axum::middleware::from_fn_with_state(WebToken(token), require_web_token),
            ),
            Some(_) | None => vaults_v1,
        }
    };

    // Vault-scoped asset reads sit outside `vaults_v1`'s web-token-only auth for
    // the same reason the upload route below does, in the opposite direction:
    // `get_attachment`'s default response is a `download_url` pointing here
    // (#176), and an MCP client holding only the MCP bearer token could not
    // fetch it while this route accepted the web token alone. Read-only MCP
    // already exposes the same bytes through `encoding: "base64"`, so accepting
    // the MCP credential here widens no capability. The web token still works,
    // including as an `access_token` query parameter, which is how the browser's
    // `<img>` tags and download navigations reach it.
    let vault_assets = Router::new().route(
        "/api/v1/vaults/{vault_id}/assets/{*path}",
        get(vault_scoped_asset_handler),
    );
    // Gated on exactly the condition `vaults_v1` uses above, and for the same
    // two reasons: a deployment with no web token configured serves this route
    // openly (enabling MCP must not suddenly demand a credential the browser
    // has never had), and #109's demo mode publishes reads unauthenticated,
    // with asset visibility decided by the handler's own layer rules rather
    // than by this token check.
    let vault_assets = match web_bearer_token.clone() {
        Some(token) if !state.demo_mode => {
            vault_assets.layer(axum::middleware::from_fn_with_state(
                AssetReadTokens {
                    web: token,
                    runtime_config: state.runtime_config.clone(),
                    limiter: mcp_transport.limiter(),
                },
                require_web_or_live_mcp_read_token,
            ))
        }
        Some(_) | None => vault_assets,
    };

    // Vault-scoped attachment upload sits outside `vaults_v1`'s web-token-only
    // auth, mirroring the legacy `/api/attachment` route it replaces: an MCP
    // agent that already holds the MCP bearer token can use it directly,
    // without provisioning the separate web token just for this one route. It
    // still accepts the web token too, since the web UI's paste-to-upload flow
    // hits the same endpoint. #109: mounted in demo mode too, like the rest of
    // this router, but gated by `reject_demo_mutation` since it is a content
    // mutation.
    let vault_attachment = Router::new().route(
        "/api/v1/vaults/{vault_id}/attachments",
        post(vault_scoped_upload_attachment_handler)
            .layer(DefaultBodyLimit::max(attachment_body_limit))
            .layer(demo_guard.clone()),
    );
    // #109: same reasoning as `vaults_v1` above. This route is a content
    // mutation, so demo mode must answer with `403 demo_read_only` from the
    // route's own `demo_guard` layer rather than a bare `401` from the token
    // gate that would otherwise wrap the whole subrouter first.
    let vault_attachment = if state.demo_mode {
        vault_attachment
    } else {
        vault_attachment.layer(axum::middleware::from_fn_with_state(
            WebOrLiveMcpToken {
                web: web_bearer_token.clone(),
                runtime_config: state.runtime_config.clone(),
            },
            require_web_or_live_mcp_token,
        ))
    };

    Router::new()
        .route("/health", get(health_handler))
        .route("/ready", get(readiness_handler))
        .route("/api/startup-status", get(startup_status_handler))
        .merge(model_setup)
        .merge(settings)
        .merge(vaults_v1)
        .merge(vault_assets)
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
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            reject_startup_recovery_mutation,
        ))
        .with_state(state)
}

/// Environment-cleanup recovery keeps liveness and read-only explanation
/// surfaces reachable, but it is not an alternate operating mode. Refuse all
/// state-changing HTTP/MCP requests until the operator removes the named keys
/// and restarts, regardless of which inner router would otherwise own them.
async fn reject_startup_recovery_mutation(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let is_safe_method = matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    );
    let recovery = state
        .legacy_migration_recovery
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    if !is_safe_method
        && let Some(recovery) = recovery
        && !recovery.can_start_with_no_vaults()
    {
        return crate::handlers::vaults::VaultApiError::new(
            "legacy_environment_cleanup_required",
            recovery.message(),
            None,
            false,
        )
        .respond(StatusCode::SERVICE_UNAVAILABLE);
    }
    next.run(request).await
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

async fn accept_gemma_handler(State(state): State<AppState>) -> Response {
    match select_model_and_start(state, SelectedModel::Gemma) {
        Ok(()) => StatusCode::ACCEPTED.into_response(),
        Err(ModelChoiceError::AlreadyActive) => model_setup_conflict(),
        Err(ModelChoiceError::Persist(error)) => model_setup_error(error),
    }
}

async fn decline_gemma_handler(State(state): State<AppState>) -> Response {
    match select_model_and_start(state, SelectedModel::Nomic) {
        Ok(()) => StatusCode::ACCEPTED.into_response(),
        Err(ModelChoiceError::AlreadyActive) => model_setup_conflict(),
        Err(ModelChoiceError::Persist(error)) => model_setup_error(error),
    }
}

async fn retry_model_setup_handler(State(state): State<AppState>) -> Response {
    match state.model_setup.selected() {
        Ok(selected @ (SelectedModel::Gemma | SelectedModel::Nomic)) => {
            if spawn_model_startup(state, selected) {
                StatusCode::ACCEPTED.into_response()
            } else {
                model_setup_conflict()
            }
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

fn model_setup_conflict() -> Response {
    (
        StatusCode::CONFLICT,
        Json(serde_json::json!({
            "error": "The running search model cannot be changed until Hatchdoor restarts."
        })),
    )
        .into_response()
}

/// The only failures that can occur before the background model lifecycle is
/// spawned. Kept crate-visible so the MCP setup tools can report the same
/// persisted-choice/activation outcome as the HTTP setup endpoints.
pub(crate) enum ModelChoiceError {
    AlreadyActive,
    Persist(String),
}

/// Persist a first-run model choice only after claiming the one model setup
/// lifecycle. Both HTTP and MCP choices must use this function: a loser must
/// leave `selection.json` untouched because the winning lifecycle owns the
/// runtime embedder and ensuing index rebuild.
pub(crate) fn select_model_and_start(
    state: AppState,
    selected: SelectedModel,
) -> Result<(), ModelChoiceError> {
    if !claim_model_setup(&state) {
        return Err(ModelChoiceError::AlreadyActive);
    }

    let persist = match selected {
        SelectedModel::Gemma => state.model_setup.accept_gemma(),
        SelectedModel::Nomic => state.model_setup.decline_gemma(),
        SelectedModel::TermsRequired => Err("model terms have not been accepted".to_string()),
    };
    if let Err(error) = persist {
        state.model_setup_started.store(false, Ordering::Release);
        return Err(ModelChoiceError::Persist(error));
    }

    spawn_claimed_model_startup(state, selected);
    Ok(())
}

/// Claim the sole model-setup lifecycle before a choice is persisted. This
/// makes choice persistence and runtime activation one serialized operation.
fn claim_model_setup(state: &AppState) -> bool {
    state
        .model_setup_started
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

pub(crate) fn spawn_model_startup(state: AppState, selected: SelectedModel) -> bool {
    if !claim_model_setup(&state) {
        return false;
    }
    spawn_claimed_model_startup(state, selected);
    true
}

fn spawn_claimed_model_startup(state: AppState, selected: SelectedModel) {
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
                // Lifecycle reconstruction may have queued Index work before
                // the shared embedder finished loading. Re-request every
                // active Vault through the same coalescing FIFO now that its
                // candidate snapshots can be built.
                for vault_id in state.vaults.active_vault_ids() {
                    state.vault_work.request(vault_id, VaultWorkKind::Index);
                }
                // The Vault collection owns indexing now. Its Index turn
                // publishes a structure-only generation before embedding, so
                // browsing becomes available without a second, legacy
                // single-Vault build contending for the same SQLite writer.
                tracker.set_scanning();
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
    }
    .unwrap_or_else(|e| {
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

    // Migration may persist the registry and discard a recognized legacy
    // cache, so run it only after startup security/configuration refusals and
    // before opening SQLite.
    let vault_registry = VaultRegistryStore::at_default_path();
    let legacy_vault_path = match &config.vault_source {
        VaultSource::Local { vault_path } => vault_path.clone(),
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
            let recovery = check_legacy_environment_posture(&ignored_environment_keys)
                .err()
                .map(crate::vault_migration::LegacyMigrationRecovery::environment_cleanup);
            if let Some(recovery) = &recovery {
                warn!(
                    code = recovery.code(),
                    message = recovery.message(),
                    "Legacy environment cleanup requires an operator restart"
                );
            }
            (state, recovery)
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
            let recovery = check_legacy_environment_posture(&ignored_environment_keys)
                .err()
                .map(|message| {
                    crate::vault_migration::LegacyMigrationRecovery::environment_cleanup(format!(
                        "Your Vault was imported successfully. {message}"
                    ))
                });
            if let Some(recovery) = &recovery {
                warn!(
                    code = recovery.code(),
                    message = recovery.message(),
                    "Imported Vault is waiting for legacy environment cleanup and restart"
                );
            }
            (VaultRegistryState::Ready(snapshot), recovery)
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

    if let VaultRegistryState::Ready(snapshot) = &registry_state
        && let Err(message) = check_demo_mode_registry_posture(config.demo_mode, snapshot)
    {
        error!("{message}");
        std::process::exit(1);
    }

    let startup_recovery_active = legacy_migration_recovery.is_some();

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
    let vaults = VaultCollectionRuntime::with_watching_and_cache(
        config.cache_db_path.clone(),
        sqlite.clone(),
    );
    // Subscribe before reconciliation starts per-Vault watchers, so an early
    // filesystem event is retained in the bounded intent channel until the
    // forwarding task starts below.
    let watcher_changes = vaults.subscribe_changes();
    // #90 establishes durable reconstruction and lifecycle admission. The
    // worker loop below is the one global dispatcher for all admitted turns.
    let (vault_work, vault_worker) = VaultWorkCoordinator::new();
    let managed_git = Arc::new(crate::git::ManagedGitScheduler::new(vault_work.clone()));
    match (&registry_state, legacy_migration_recovery.as_ref()) {
        (_, Some(recovery)) => warn!(
            code = recovery.code(),
            "Startup recovery is active; no Vault runtimes were activated"
        ),
        (VaultRegistryState::Ready(snapshot), None) => {
            vaults
                .reconcile_and_reconstruct(&vault_registry, snapshot, &vault_work, &managed_git)
                .await
        }
        (VaultRegistryState::Recovery(recovery), None) => warn!(
            message = recovery.message(),
            "Vault registry requires operator recovery; no Vault runtimes were activated"
        ),
    }
    let runtime = VaultRuntime::new(config.vault_source.clone());
    let startup = StartupTracker::new(runtime);
    if startup_recovery_active {
        startup.runtime().set_unavailable(
            "startup_recovery_required",
            "Startup recovery is required before Vaults can be activated",
        );
    } else if selected_model == SelectedModel::TermsRequired {
        startup.set_terms_required();
    } else {
        startup.set_scanning();
    }
    let shutdown_vaults = vaults.clone();
    let state = AppState {
        cache_db_path: config.cache_db_path.clone(),
        vault_registry,
        vaults,
        vault_work: vault_work.clone(),
        managed_git: managed_git.clone(),
        legacy_migration_recovery: Arc::new(std::sync::RwLock::new(legacy_migration_recovery)),
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
    // Git turns through the managed-Git scheduler and Index turns through the
    // Vault-qualified disposable snapshot builder. Repair remains owned by
    // its later packet.
    // Exits on its own once `vault_work.shutdown()` drains to quiescence.
    let dispatch_task = tokio::spawn({
        let mut vault_worker = vault_worker;
        let dispatch_vaults = state.vaults.clone();
        let dispatch_registry = state.vault_registry.clone();
        let dispatch_work = vault_work.clone();
        let dispatch_managed_git = managed_git.clone();
        let dispatch_cache = state.startup_sqlite.clone();
        let dispatch_embedder = state.embedder.clone();
        let dispatch_runtime_config = state.runtime_config.clone();
        let dispatch_startup = state.startup.clone();
        let dispatch_model_setup_started = state.model_setup_started.clone();
        async move {
            while let Some(outcome) = vault_worker
                .run_next(|request| {
                    let vaults = dispatch_vaults.clone();
                    let registry = dispatch_registry.clone();
                    let work = dispatch_work.clone();
                    let managed_git = dispatch_managed_git.clone();
                    let cache = dispatch_cache.clone();
                    let embedder = dispatch_embedder.clone();
                    let runtime_snapshot = dispatch_runtime_config.snapshot();
                    // Read per turn, not once at startup, so saving a new
                    // author name or email applies to the next Git turn of
                    // every Vault without its own commit identity — no
                    // restart. Bound to the same snapshot the rest of this
                    // turn observes.
                    let (author_name, author_email) = git_author_defaults(&runtime_snapshot);
                    let startup = dispatch_startup.clone();
                    VaultWorkDispatchContext {
                        vaults,
                        registry,
                        work,
                        managed_git,
                        cache,
                        embedder,
                        runtime_snapshot,
                        author_name,
                        author_email,
                        startup,
                    }
                    .dispatch(request)
                })
                .await
            {
                if outcome.request.kind() == VaultWorkKind::Index {
                    match &outcome.result {
                        Ok(()) if collection_indexes_ready(&dispatch_vaults) => {
                            dispatch_startup.set_ready();
                            dispatch_model_setup_started.store(false, Ordering::Release);
                            info!("Vault collection indexing complete");
                        }
                        Err(error) if error.code() != "embedder_not_ready" => {
                            dispatch_startup.set_failed();
                            dispatch_model_setup_started.store(false, Ordering::Release);
                        }
                        _ => {}
                    }
                }
                if let Err(error) = outcome.result {
                    // Repair remains expected until its dedicated packet;
                    // Index and Git failures are actionable per-Vault status.
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
    let watcher_index_task = watcher_changes.map(|mut changes| {
        let watcher_work = vault_work.clone();
        let watcher_vaults = state.vaults.clone();
        tokio::spawn(async move {
            while forward_vault_change_intent(changes.recv().await, &watcher_work, &watcher_vaults)
            {
            }
        })
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

    // Filesystem watching belongs to each Vault's own runtime
    // (`vault_runtime.rs` spawns one cancellable `VaultWatcherHandle` per
    // active Vault, forwarded through `VaultWorkCoordinator`, which coalesces).
    // Startup deliberately spawns no watcher of its own: audit findings
    // C01-F02/C01-F03 were detached replacement watchers accumulating from
    // exactly that, with no handle to cancel.
    if !startup_recovery_active && selected_model != SelectedModel::TermsRequired {
        spawn_model_startup(state.clone(), selected_model);
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
    if let Some(task) = watcher_index_task {
        task.abort();
    }
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

/// Forward a per-Vault watcher invalidation to the one shared Index queue.
/// A lagged broadcast intentionally falls back to every active Vault because
/// the channel is a coalescible hint rather than a durable event log.
fn forward_vault_change_intent(
    change: Result<crate::vault_registry::VaultId, tokio::sync::broadcast::error::RecvError>,
    coordinator: &VaultWorkCoordinator,
    vaults: &VaultCollectionRuntime,
) -> bool {
    match change {
        Ok(vault_id) => {
            coordinator.request(vault_id, VaultWorkKind::Index);
            true
        }
        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
            for vault_id in vaults.active_vault_ids() {
                coordinator.request(vault_id, VaultWorkKind::Index);
            }
            true
        }
        Err(tokio::sync::broadcast::error::RecvError::Closed) => false,
    }
}

fn collection_indexes_ready(vaults: &VaultCollectionRuntime) -> bool {
    let active = vaults.active_vault_ids();
    !active.is_empty()
        && active.into_iter().all(|vault_id| {
            vaults.runtime(vault_id).is_some_and(|runtime| {
                runtime.snapshot().search == crate::vault_runtime::VaultSearchStatus::Ready
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::ReadyVault;

    #[test]
    fn compose_vault_path_alone_never_refuses_a_start() {
        // Every Compose deployment sets VAULT_PATH; refusing on it would
        // refuse every one of them.
        check_legacy_environment_posture(&["VAULT_PATH".to_string()])
            .expect("VAULT_PATH is not a per-Vault setting");
    }

    /// The dispatch loop reads the author defaults from the snapshot bound to
    /// each turn rather than from a value captured once at startup, so saving
    /// a new name or email applies to the next Git turn of every Vault without
    /// its own commit identity — with no restart.
    #[test]
    fn git_author_defaults_follow_a_saved_settings_change_without_a_restart() {
        let runtime_config = crate::runtime_config::RuntimeConfig::for_tests();

        assert_eq!(
            git_author_defaults(&runtime_config.snapshot()),
            ("Hatchdoor".to_string(), "hatchdoor@localhost".to_string()),
            "an unconfigured instance falls back to the documented defaults"
        );

        runtime_config
            .save([
                (
                    "HATCHDOOR_GIT_AUTHOR_NAME".to_string(),
                    "Second Author".to_string(),
                ),
                (
                    "HATCHDOOR_GIT_AUTHOR_EMAIL".to_string(),
                    "second@example.test".to_string(),
                ),
            ])
            .expect("save author defaults");

        // A turn dispatched after the save binds a fresh snapshot, exactly as
        // the dispatch closure does.
        assert_eq!(
            git_author_defaults(&runtime_config.snapshot()),
            (
                "Second Author".to_string(),
                "second@example.test".to_string()
            )
        );
    }

    #[test]
    fn a_settled_registry_refuses_to_start_beside_stale_per_vault_variables() {
        let message = check_legacy_environment_posture(&[
            "VAULT_PATH".to_string(),
            "HATCHDOOR_GIT_AUTHOR_EMAIL".to_string(),
            "HATCHDOOR_EXCLUDE".to_string(),
        ])
        .expect_err("stale per-Vault variables refuse the start");
        assert!(message.contains("HATCHDOOR_GIT_AUTHOR_EMAIL"), "{message}");
        assert!(message.contains("HATCHDOOR_EXCLUDE"), "{message}");
        assert!(!message.contains("VAULT_PATH"), "{message}");
        assert!(message.contains("Settings"), "{message}");
    }

    #[test]
    fn a_retired_debounce_variable_is_named_as_obsolete_rather_than_migrated() {
        let message =
            check_legacy_environment_posture(&["HATCHDOOR_GIT_DEBOUNCE_SECONDS".to_string()])
                .expect_err("an obsolete variable still has to go");
        assert!(
            message.contains("no longer does anything"),
            "a retired setting must not be described as taken over: {message}"
        );
        assert!(!message.contains("taken these settings over"), "{message}");
    }

    #[tokio::test]
    async fn watcher_index_intents_use_the_shared_fifo_and_lag_requeues_active_vaults() {
        let directory = tempfile::tempdir().expect("temporary state directory");
        let vault_path = directory.path().join("vault");
        std::fs::create_dir_all(&vault_path).expect("create Vault directory");
        let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
        let snapshot = registry
            .add(
                0,
                crate::vault_registry::NewVaultDefinition {
                    name: "Watcher Vault".to_string(),
                    enabled: true,
                    source: crate::vault_registry::VaultSource::Local { path: vault_path },
                    exclude_patterns: Vec::new(),
                    https_credentials: None,
                    archive_folder: None,
                    commit_identity: None,
                },
            )
            .expect("add Vault");
        let vault_id = snapshot
            .definitions()
            .next()
            .expect("Vault definition")
            .vault_id();
        let vaults = VaultCollectionRuntime::new();
        vaults.reconcile(&registry, &snapshot);
        let (coordinator, mut worker) = VaultWorkCoordinator::new();

        assert!(forward_vault_change_intent(
            Ok(vault_id),
            &coordinator,
            &vaults
        ));
        let direct = worker
            .run_next(|request| async move {
                assert_eq!(request.vault_id(), vault_id);
                assert_eq!(request.kind(), VaultWorkKind::Index);
                Ok::<(), VaultWorkError>(())
            })
            .await
            .expect("watcher intent queues Index");
        direct.result.expect("watcher Index turn succeeds");

        assert!(forward_vault_change_intent(
            Err(tokio::sync::broadcast::error::RecvError::Lagged(1)),
            &coordinator,
            &vaults,
        ));
        let after_lag = worker
            .run_next(|request| async move {
                assert_eq!(request.vault_id(), vault_id);
                assert_eq!(request.kind(), VaultWorkKind::Index);
                Ok::<(), VaultWorkError>(())
            })
            .await
            .expect("lag requeues active Vault Index");
        after_lag.result.expect("lag recovery turn succeeds");
        assert!(!forward_vault_change_intent(
            Err(tokio::sync::broadcast::error::RecvError::Closed),
            &coordinator,
            &vaults,
        ));
    }

    #[test]
    fn startup_readiness_follows_collection_index_completion() {
        let directory = tempfile::tempdir().expect("temporary state directory");
        let vault_path = directory.path().join("vault");
        std::fs::create_dir_all(&vault_path).expect("create Vault directory");
        let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
        let snapshot = registry
            .add(
                0,
                crate::vault_registry::NewVaultDefinition {
                    name: "Startup Vault".to_string(),
                    enabled: true,
                    source: crate::vault_registry::VaultSource::Local { path: vault_path },
                    exclude_patterns: Vec::new(),
                    https_credentials: None,
                    archive_folder: None,
                    commit_identity: None,
                },
            )
            .expect("add Vault");
        let vault_id = snapshot
            .definitions()
            .next()
            .expect("Vault definition")
            .vault_id();
        let vaults = VaultCollectionRuntime::new();
        vaults.reconcile(&registry, &snapshot);

        assert!(!collection_indexes_ready(&vaults));
        vaults
            .runtime(vault_id)
            .expect("active Vault")
            .set_search_status(crate::vault_runtime::VaultSearchStatus::Ready, None)
            .expect("publish ready search status");
        assert!(collection_indexes_ready(&vaults));
    }

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
        // Every loopback spelling accepted without a token has a corresponding
        // structurally valid listener address (covered in config tests).
        for host in ["127.0.0.1", "localhost", "::1", "[::1]"] {
            assert!(check_web_auth_posture(host, false, false).is_ok(), "{host}");
        }
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
    fn demo_mode_registry_posture_rejects_an_enabled_two_way_vault() {
        let directory = tempfile::tempdir().expect("temporary state directory");
        let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
        let snapshot = registry
            .add(
                0,
                crate::vault_registry::NewVaultDefinition {
                    name: "Two-Way Vault".to_string(),
                    enabled: true,
                    source: crate::vault_registry::VaultSource::ManagedGit {
                        repository_url: "https://example.com/vault.git".to_string(),
                        branch: None,
                        vault_subdirectory: None,
                        mode: crate::vault_registry::VaultGitMode::TwoWay,
                        poll_interval_secs: 3600,
                    },
                    exclude_patterns: Vec::new(),
                    https_credentials: None,
                    archive_folder: None,
                    commit_identity: None,
                },
            )
            .expect("add two-way Vault");

        // Outside demo mode, a two-way registry Vault is unremarkable.
        assert!(check_demo_mode_registry_posture(false, &snapshot).is_ok());

        let error = check_demo_mode_registry_posture(true, &snapshot)
            .expect_err("enabled registry TwoWay Vault rejected in demo mode");
        assert!(error.contains("two-way Git mode"));
    }

    #[test]
    fn demo_mode_registry_posture_rejects_an_enabled_existing_git_two_way_vault() {
        // `ExistingGit` and `ManagedGit` share one match arm in
        // `check_demo_mode_registry_posture`; cover `ExistingGit` separately
        // so a future divergence between the two arms is caught. Unlike
        // `ManagedGit`, the registry requires `repository_path` to already be
        // a real Git working checkout (see
        // `vault_scoped_pull_only_vault_disables_mutation` for the same
        // fixture pattern).
        let directory = tempfile::tempdir().expect("temporary state directory");
        let repository_path = directory.path().join("existing-two-way-repo");
        std::fs::create_dir_all(&repository_path).expect("create repo directory");
        git2::Repository::init(&repository_path).expect("init git repo");
        let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
        let snapshot = registry
            .add(
                0,
                crate::vault_registry::NewVaultDefinition {
                    name: "Existing Git Two-Way Vault".to_string(),
                    enabled: true,
                    source: crate::vault_registry::VaultSource::ExistingGit {
                        repository_path,
                        repository_url: Some(
                            "https://example.com/existing-two-way.git".to_string(),
                        ),
                        branch: None,
                        vault_subdirectory: None,
                        mode: crate::vault_registry::VaultGitMode::TwoWay,
                        poll_interval_secs:
                            crate::vault_registry::DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
                    },
                    exclude_patterns: Vec::new(),
                    https_credentials: None,
                    archive_folder: None,
                    commit_identity: None,
                },
            )
            .expect("add existing-git two-way Vault");

        let error = check_demo_mode_registry_posture(true, &snapshot)
            .expect_err("enabled registry ExistingGit TwoWay Vault rejected in demo mode");
        assert!(error.contains("two-way Git mode"));
    }

    #[test]
    fn demo_mode_registry_posture_allows_pull_only_local_history_and_disabled_two_way() {
        let directory = tempfile::tempdir().expect("temporary state directory");
        // `ManagedGit` requires `pull_only` or `two_way` mode (a purely local
        // history makes no sense for a Vault whose only source is a remote);
        // `LocalHistory` is only reachable through `ExistingGit`, which needs
        // a real Git working checkout to pass registry validation.
        let local_history_repository_path = directory.path().join("local-history-repo");
        std::fs::create_dir_all(&local_history_repository_path)
            .expect("create local-history repo directory");
        git2::Repository::init(&local_history_repository_path).expect("init git repo");
        let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
        let snapshot = registry
            .add(
                0,
                crate::vault_registry::NewVaultDefinition {
                    name: "Pull-Only Vault".to_string(),
                    enabled: true,
                    source: crate::vault_registry::VaultSource::ManagedGit {
                        repository_url: "https://example.com/pull-only.git".to_string(),
                        branch: None,
                        vault_subdirectory: None,
                        mode: crate::vault_registry::VaultGitMode::PullOnly,
                        poll_interval_secs: 3600,
                    },
                    exclude_patterns: Vec::new(),
                    https_credentials: None,
                    archive_folder: None,
                    commit_identity: None,
                },
            )
            .expect("add pull-only Vault");
        let snapshot = registry
            .add(
                snapshot.revision(),
                crate::vault_registry::NewVaultDefinition {
                    name: "Local History Vault".to_string(),
                    enabled: true,
                    source: crate::vault_registry::VaultSource::ExistingGit {
                        repository_path: local_history_repository_path,
                        repository_url: None,
                        branch: None,
                        vault_subdirectory: None,
                        mode: crate::vault_registry::VaultGitMode::LocalHistory,
                        poll_interval_secs:
                            crate::vault_registry::DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
                    },
                    exclude_patterns: Vec::new(),
                    https_credentials: None,
                    archive_folder: None,
                    commit_identity: None,
                },
            )
            .expect("add local-history Vault");
        let snapshot = registry
            .add(
                snapshot.revision(),
                crate::vault_registry::NewVaultDefinition {
                    name: "Disabled Two-Way Vault".to_string(),
                    enabled: false,
                    source: crate::vault_registry::VaultSource::ManagedGit {
                        repository_url: "https://example.com/disabled-two-way.git".to_string(),
                        branch: None,
                        vault_subdirectory: None,
                        mode: crate::vault_registry::VaultGitMode::TwoWay,
                        poll_interval_secs: 3600,
                    },
                    exclude_patterns: Vec::new(),
                    https_credentials: None,
                    archive_folder: None,
                    commit_identity: None,
                },
            )
            .expect("add disabled two-way Vault");

        assert!(check_demo_mode_registry_posture(true, &snapshot).is_ok());
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
    use crate::cache::SqliteCache;
    use crate::embed::{Embedder, StubEmbedder};
    use crate::search::vault_scoped::{VaultSearchCore, VaultSearchRequest};
    use crate::search::{LayerSelection, NoteFilters, SearchMode};
    use crate::vault_read::VaultScope;

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
            legacy_migration_recovery: Arc::new(std::sync::RwLock::new(None)),
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

    #[tokio::test]
    async fn queued_index_turn_uses_the_runtime_snapshot_captured_by_server_dispatch() {
        let directory = TempDir::new_in("/tmp").expect("temporary state directory");
        let vault_path = directory.path().join("vault");
        std::fs::create_dir_all(vault_path.join("sources")).expect("create Vault directory");
        std::fs::write(vault_path.join("sources/.hatchdoor-layer"), "sources")
            .expect("write layer marker");
        std::fs::write(
            vault_path.join("sources/Clip.md"),
            "# Clip\n\nmelatonin regulates the circadian rhythm",
        )
        .expect("write demoted note");

        let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
        let snapshot = registry
            .add(
                0,
                crate::vault_registry::NewVaultDefinition {
                    name: "Only".to_string(),
                    enabled: true,
                    source: crate::vault_registry::VaultSource::Local { path: vault_path },
                    exclude_patterns: Vec::new(),
                    https_credentials: None,
                    archive_folder: None,
                    commit_identity: None,
                },
            )
            .expect("add Vault");
        let vault_id = snapshot
            .definitions()
            .next()
            .expect("Vault definition")
            .vault_id();
        let vaults = VaultCollectionRuntime::new();
        let (work, mut worker) = VaultWorkCoordinator::new();
        let managed_git = Arc::new(crate::git::ManagedGitScheduler::new(work.clone()));
        vaults
            .reconcile_and_reconstruct(&registry, &snapshot, &work, &managed_git)
            .await;
        let cache = Arc::new(SqliteCache::in_memory(384).expect("open shared cache"));
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
        let runtime_config = RuntimeConfig::for_tests();
        runtime_config
            .save([("HATCHDOOR_EMBED_LAYERS".to_string(), "false".to_string())])
            .expect("save disabled setting");
        let captured = runtime_config.snapshot();
        runtime_config
            .save([("HATCHDOOR_EMBED_LAYERS".to_string(), "true".to_string())])
            .expect("save later setting");

        let outcome = worker
            .run_next({
                let vaults = vaults.clone();
                let registry = registry.clone();
                let work = work.clone();
                let cache = cache.clone();
                let embedder = embedder.clone();
                move |request| {
                    VaultWorkDispatchContext {
                        vaults,
                        registry,
                        work,
                        managed_git,
                        cache,
                        embedder,
                        runtime_snapshot: captured,
                        author_name: "Hatchdoor".to_string(),
                        author_email: "hatchdoor@example.test".to_string(),
                        startup: StartupTracker::scanning(),
                    }
                    .dispatch(request)
                }
            })
            .await
            .expect("queued Index turn");
        outcome.result.expect("Index publication succeeds");

        let (layers, _) = LayerSelection::parse(&["sources".to_string()], &["sources".to_string()]);
        let search = VaultSearchCore::new(&cache, &vaults, embedder.as_ref());
        let keyword = search
            .search(VaultSearchRequest {
                scope: VaultScope::One(vault_id),
                query: "melatonin".to_string(),
                mode: SearchMode::Keyword,
                limit: 10,
                per_note_cap: 1,
                filters: NoteFilters::default(),
                include_properties: Vec::new(),
                layers: layers.clone(),
            })
            .expect("keyword search");
        assert!(
            keyword
                .data
                .results
                .iter()
                .any(|hit| hit.note_slug == "clip")
        );
        assert!(
            search
                .search(VaultSearchRequest {
                    scope: VaultScope::One(vault_id),
                    query: "melatonin circadian".to_string(),
                    mode: SearchMode::Semantic,
                    limit: 10,
                    per_note_cap: 1,
                    filters: NoteFilters::default(),
                    include_properties: Vec::new(),
                    layers,
                })
                .expect("semantic search")
                .data
                .results
                .is_empty(),
            "the turn must retain the false setting captured before the later save"
        );
    }

    fn app_for_tests_with_web_and_mcp_auth(
        web_bearer_token: Option<Arc<str>>,
        mcp_bearer_token: Option<String>,
    ) -> (Router, TempDir) {
        let (app, tmp, _state) = app_for_tests_with_web_and_mcp_auth_and_write_mode(
            web_bearer_token,
            mcp_bearer_token,
            true,
        );
        (app, tmp)
    }

    fn app_for_tests_with_web_and_mcp_auth_and_write_mode(
        web_bearer_token: Option<Arc<str>>,
        mcp_bearer_token: Option<String>,
        mcp_write_enabled: bool,
    ) -> (Router, TempDir, AppState) {
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
            legacy_migration_recovery: Arc::new(std::sync::RwLock::new(None)),
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

        (build_router(state.clone(), web_bearer_token), tmp, state)
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
    async fn attachment_route_forbids_read_only_mcp_token_without_creating_file() {
        // Supersedes the earlier #60 expectation that a read-only MCP token
        // keeps working here: audit finding X-F01 made an MCP disable (or a
        // write-mode disable) an immediate revocation of that credential's
        // upload capability. The gate lives in `auth.rs`.
        let (app, tmp, _state) = app_for_tests_with_web_and_mcp_auth_and_write_mode(
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
            .clone()
            .oneshot(attachment_upload_request(
                &vault_id,
                "Attachments/via-mcp-token.png",
                Some("mcp-secret"),
            ))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(
            !tmp.path()
                .join("attachments/Attachments/via-mcp-token.png")
                .exists()
        );

        let web_response = app
            .oneshot(attachment_upload_request(
                &vault_id,
                "Attachments/via-web-token.png",
                Some("web-secret"),
            ))
            .await
            .expect("response");
        assert_eq!(web_response.status(), StatusCode::OK);
        assert!(
            tmp.path()
                .join("attachments/Attachments/via-web-token.png")
                .exists()
        );
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
        let vault_root = tmp.path().join("mcp-attachments");
        let vault_id = create_vault_with_files(&app, "Mcp", &vault_root, &[], 0).await;

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
                    .header("host", "localhost")
                    .header("accept", "application/json, text/event-stream")
                    .header("authorization", "Bearer first-token")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#,
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
                    .header("host", "localhost")
                    .header("authorization", "Bearer first-token")
                    .header("origin", "https://evil.example")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#,
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

        // The same live MCP token loses its attachment-write capability on the
        // first request after write mode is disabled.
        let read_only_attachment = app
            .clone()
            .oneshot(attachment_upload_request(
                &vault_id,
                "Attachments/read-only.png",
                Some("first-token"),
            ))
            .await
            .expect("response");
        assert_eq!(read_only_attachment.status(), StatusCode::FORBIDDEN);
        assert!(!vault_root.join("Attachments/read-only.png").exists());

        let writes_enabled = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/settings")
                    .method("PATCH")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"updates":{"HATCHDOOR_MCP_WRITE_ENABLED":"true"}}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(writes_enabled.status(), StatusCode::OK);

        let enabled_attachment = app
            .clone()
            .oneshot(attachment_upload_request(
                &vault_id,
                "Attachments/writes-enabled.png",
                Some("first-token"),
            ))
            .await
            .expect("response");
        assert_eq!(enabled_attachment.status(), StatusCode::OK);
        assert!(vault_root.join("Attachments/writes-enabled.png").exists());

        let limits = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/settings")
                    .method("PATCH")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"updates":{"HATCHDOOR_MAX_ATTACHMENT_BYTES":"1","HATCHDOOR_MCP_MAX_BASE64_BYTES":"1"}}"#,
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

        let limited_session = initialize_mcp_session(&app, "first-token").await;
        let limited_base64 = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/mcp")
                    .method("POST")
                    .header("host", "localhost")
                    .header("mcp-session-id", limited_session)
                    .header("accept", "application/json, text/event-stream")
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

    /// Dedicated regression for the v2.5.0 security invariant across the
    /// ADR-17 boundary swap (#172): the multipart attachment endpoint accepts
    /// an MCP bearer token **only** while MCP *and* MCP write mode are both
    /// live-enabled in the current runtime snapshot — and it re-checks the
    /// snapshot on every request, not once at startup.
    #[tokio::test]
    async fn attachment_route_accepts_the_mcp_token_only_while_mcp_and_writes_are_live() {
        let (app, tmp, state) = app_for_tests_with_web_and_mcp_auth_and_write_mode(
            Some(Arc::from("web-secret")),
            Some("mcp-secret".to_string()),
            true,
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
        let upload = |name: &'static str| {
            let app = app.clone();
            let request = attachment_upload_request(
                &vault_id,
                &format!("Attachments/{name}"),
                Some("mcp-secret"),
            );
            async move { app.oneshot(request).await.expect("response") }
        };
        let save = |updates: &[(&str, &str)]| {
            state
                .runtime_config
                .save(updates.iter().map(|(k, v)| (k.to_string(), v.to_string())))
        };

        // MCP fully disabled: the MCP credential matches nothing and is 401.
        save(&[
            ("HATCHDOOR_MCP_ENABLED", "false"),
            ("HATCHDOOR_MCP_WRITE_ENABLED", "false"),
        ])
        .expect("save disabled");
        assert_eq!(
            upload("mcp-disabled.png").await.status(),
            StatusCode::UNAUTHORIZED
        );

        // MCP enabled but write mode off: recognized, but revoked → 403.
        save(&[
            ("HATCHDOOR_MCP_ENABLED", "true"),
            ("HATCHDOOR_MCP_WRITE_ENABLED", "false"),
        ])
        .expect("save read-only");
        assert_eq!(
            upload("read-only.png").await.status(),
            StatusCode::FORBIDDEN
        );
        assert!(
            !tmp.path()
                .join("attachments/Attachments/read-only.png")
                .exists()
        );

        // Both live: accepted on the very next request against the same router.
        save(&[("HATCHDOOR_MCP_WRITE_ENABLED", "true")]).expect("save writes");
        assert_eq!(upload("both-live.png").await.status(), StatusCode::OK);
        assert!(
            tmp.path()
                .join("attachments/Attachments/both-live.png")
                .exists()
        );

        // And revocation is immediate again — no restart, no re-mount.
        save(&[("HATCHDOOR_MCP_ENABLED", "false")]).expect("save re-disable");
        assert_eq!(
            upload("re-disabled.png").await.status(),
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn asset_route_accepts_a_read_only_mcp_token_and_the_web_token_in_either_position() {
        // #176: `get_attachment`'s default response is a download_url pointing
        // at this route, so the MCP credential must be able to fetch it —
        // including in read-only mode, since the same bytes are already
        // reachable through that tool's base64 encoding. The web token keeps
        // both positions because the browser reaches assets through `<img>`
        // tags that cannot set headers.
        let (app, tmp, _state) = app_for_tests_with_web_and_mcp_auth_and_write_mode(
            Some(Arc::from("web-secret")),
            Some("mcp-secret".to_string()),
            false,
        );
        let vault_root = tmp.path().join("assets-auth");
        let vault_id = create_vault_with_files_using_token(
            &app,
            "Assets",
            &vault_root,
            &[("Home.md", "# Home\n\n![[diagram.png]]\n")],
            0,
            Some("web-secret"),
        )
        .await;
        std::fs::write(vault_root.join("diagram.png"), b"not-really-a-png")
            .expect("write attachment");

        let fetch = |header: Option<&'static str>, query: &'static str| {
            let app = app.clone();
            let uri = format!("/api/v1/vaults/{vault_id}/assets/diagram.png{query}");
            async move {
                let mut request = Request::builder().uri(uri);
                if let Some(token) = header {
                    request = request.header("authorization", format!("Bearer {token}"));
                }
                app.oneshot(request.body(Body::empty()).expect("request"))
                    .await
                    .expect("response")
            }
        };

        assert_eq!(fetch(None, "").await.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            fetch(Some("not-a-real-token"), "").await.status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(fetch(Some("web-secret"), "").await.status(), StatusCode::OK);
        assert_eq!(
            fetch(None, "?access_token=web-secret").await.status(),
            StatusCode::OK,
            "the web token must still work in the query position for <img> tags"
        );

        // The point of the change: write mode is off, and the MCP token still
        // reads.
        let with_mcp_token = fetch(Some("mcp-secret"), "").await;
        assert_eq!(with_mcp_token.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(with_mcp_token.into_body(), usize::MAX)
            .await
            .expect("body");
        assert_eq!(bytes.as_ref(), b"not-really-a-png");

        // The MCP token is header-only here, so it never lands in a trace span.
        assert_eq!(
            fetch(None, "?access_token=mcp-secret").await.status(),
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn asset_route_holds_the_mcp_token_to_its_own_base64_byte_ceiling() {
        // The MCP credential's ceiling for attachment bytes is
        // HATCHDOOR_MCP_MAX_BASE64_BYTES, enforced by get_attachment's base64
        // encoding. The download_url that same tool advertises points here, at a
        // route whose own bound is far larger, so without this clamp the URL
        // would be a way around the setting. The web token is unaffected.
        let (app, tmp, state) = app_for_tests_with_web_and_mcp_auth_and_write_mode(
            Some(Arc::from("web-secret")),
            Some("mcp-secret".to_string()),
            false,
        );
        state
            .runtime_config
            .save([(
                "HATCHDOOR_MCP_MAX_BASE64_BYTES".to_string(),
                "8".to_string(),
            )])
            .expect("save a small base64 ceiling");

        let vault_root = tmp.path().join("assets-ceiling");
        let vault_id = create_vault_with_files_using_token(
            &app,
            "Assets",
            &vault_root,
            &[("Home.md", "# Home\n")],
            0,
            Some("web-secret"),
        )
        .await;
        std::fs::write(vault_root.join("small.png"), b"12345678").expect("write small");
        std::fs::write(vault_root.join("big.png"), b"0123456789abcdef").expect("write big");

        let fetch = |name: &'static str, token: &'static str| {
            let app = app.clone();
            let uri = format!("/api/v1/vaults/{vault_id}/assets/{name}");
            async move {
                app.oneshot(
                    Request::builder()
                        .uri(uri)
                        .header("authorization", format!("Bearer {token}"))
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response")
            }
        };

        assert_eq!(
            fetch("small.png", "mcp-secret").await.status(),
            StatusCode::OK
        );

        let refused = fetch("big.png", "mcp-secret").await;
        assert_eq!(
            refused.status(),
            StatusCode::PAYLOAD_TOO_LARGE,
            "an attachment over the MCP ceiling must not be served to the MCP token"
        );
        assert_eq!(json_body(refused).await["code"], "asset_too_large");

        assert_eq!(
            fetch("big.png", "web-secret").await.status(),
            StatusCode::OK,
            "the web token keeps the route's own larger bound"
        );
    }

    #[tokio::test]
    async fn asset_route_spends_the_mcp_tool_quota_and_answers_429_when_it_runs_out() {
        // Without this the download_url would be an unmetered second channel:
        // a credential rate-limited on /mcp could pull attachment bytes here
        // without limit. The limiter instance is shared with the transport.
        let (app, tmp, _state) = app_for_tests_with_web_and_mcp_auth_and_write_mode(
            Some(Arc::from("web-secret")),
            Some("mcp-secret".to_string()),
            false,
        );
        let vault_root = tmp.path().join("assets-quota");
        let vault_id = create_vault_with_files_using_token(
            &app,
            "Assets",
            &vault_root,
            &[("Home.md", "# Home\n")],
            0,
            Some("web-secret"),
        )
        .await;
        std::fs::write(vault_root.join("diagram.png"), b"bytes").expect("write attachment");

        let fetch = |token: &'static str| {
            let app = app.clone();
            let uri = format!("/api/v1/vaults/{vault_id}/assets/diagram.png");
            async move {
                app.oneshot(
                    Request::builder()
                        .uri(uri)
                        .header("authorization", format!("Bearer {token}"))
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response")
            }
        };

        for _ in 0..crate::mcp::limits::TOOL_CALLS_PER_MINUTE {
            assert_eq!(fetch("mcp-secret").await.status(), StatusCode::OK);
        }

        let throttled = fetch("mcp-secret").await;
        assert_eq!(throttled.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(
            throttled.headers().contains_key("retry-after"),
            "a throttled asset read must say when to come back"
        );

        assert_eq!(
            fetch("web-secret").await.status(),
            StatusCode::OK,
            "the web token is not on the MCP quota"
        );
    }

    #[tokio::test]
    async fn asset_route_stays_open_with_no_web_token_even_once_mcp_is_enabled() {
        // Enabling MCP in Settings must not start demanding a credential the
        // browser has never had: a deployment with no web token configured
        // served assets openly before, and still does.
        let (app, tmp, _state) = app_for_tests_with_web_and_mcp_auth_and_write_mode(
            None,
            Some("mcp-secret".to_string()),
            false,
        );
        let vault_root = tmp.path().join("assets-open");
        let vault_id = create_vault_with_files_using_token(
            &app,
            "Assets",
            &vault_root,
            &[("Home.md", "# Home\n")],
            0,
            None,
        )
        .await;
        std::fs::write(vault_root.join("diagram.png"), b"bytes").expect("write attachment");

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{vault_id}/assets/diagram.png"))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn asset_route_stops_accepting_the_mcp_token_the_moment_mcp_is_disabled() {
        let (app, tmp, state) = app_for_tests_with_web_and_mcp_auth_and_write_mode(
            Some(Arc::from("web-secret")),
            Some("mcp-secret".to_string()),
            false,
        );
        let vault_root = tmp.path().join("assets-revocation");
        let vault_id = create_vault_with_files_using_token(
            &app,
            "Assets",
            &vault_root,
            &[("Home.md", "# Home\n")],
            0,
            Some("web-secret"),
        )
        .await;
        std::fs::write(vault_root.join("diagram.png"), b"bytes").expect("write attachment");

        let fetch = || {
            let app = app.clone();
            let uri = format!("/api/v1/vaults/{vault_id}/assets/diagram.png");
            async move {
                app.oneshot(
                    Request::builder()
                        .uri(uri)
                        .header("authorization", "Bearer mcp-secret")
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response")
            }
        };

        assert_eq!(fetch().await.status(), StatusCode::OK);

        state
            .runtime_config
            .save([("HATCHDOOR_MCP_ENABLED".to_string(), "false".to_string())])
            .expect("save disabled");
        assert_eq!(
            fetch().await.status(),
            StatusCode::UNAUTHORIZED,
            "disabling MCP must revoke the credential on the very next request"
        );
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

    /// Perform the legacy initialize handshake against the real `/mcp`
    /// transport and return the issued session ID.
    async fn initialize_mcp_session(app: &Router, token: &str) -> String {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/mcp")
                    .method("POST")
                    .header("host", "localhost")
                    .header("accept", "application/json, text/event-stream")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(
                        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        response
            .headers()
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
            .expect("initialize issues Mcp-Session-Id")
    }

    #[tokio::test]
    async fn mcp_route_accepts_an_authenticated_write_request_above_axums_default_limit() {
        // Axum's default request-body limit is 2 MiB. A valid write-enabled MCP
        // session may need more than that for base64 attachment data, so this
        // must reach normal JSON-RPC dispatch rather than only an auth failure.
        let (_unused_app, _tmp, state) = app_for_tests_with_state();
        state
            .runtime_config
            .save([
                ("HATCHDOOR_MCP_ENABLED".to_string(), "true".to_string()),
                (
                    "HATCHDOOR_MCP_WRITE_ENABLED".to_string(),
                    "true".to_string(),
                ),
                (
                    "HATCHDOOR_MCP_BEARER_TOKEN".to_string(),
                    "mcp-secret".to_string(),
                ),
                (
                    "HATCHDOOR_MCP_MAX_BASE64_BYTES".to_string(),
                    (2 * 1024 * 1024).to_string(),
                ),
            ])
            .expect("configure write-enabled MCP");
        let app = build_router(state, None);
        let session_id = initialize_mcp_session(&app, "mcp-secret").await;
        let body = format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"ping","params":{{"padding":"{}"}}}}"#,
            "x".repeat(2 * 1024 * 1024 + 1)
        );
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/mcp")
                    .method("POST")
                    .header("host", "localhost")
                    .header("mcp-session-id", session_id)
                    .header("accept", "application/json, text/event-stream")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer mcp-secret")
                    .body(Body::from(body))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn mcp_route_rejects_an_authenticated_read_only_body_over_its_live_limit() {
        let (_unused_app, _tmp, state) = app_for_tests_with_state();
        state
            .runtime_config
            .save([
                ("HATCHDOOR_MCP_ENABLED".to_string(), "true".to_string()),
                (
                    "HATCHDOOR_MCP_WRITE_ENABLED".to_string(),
                    "false".to_string(),
                ),
                (
                    "HATCHDOOR_MCP_BEARER_TOKEN".to_string(),
                    "mcp-secret".to_string(),
                ),
            ])
            .expect("configure read-only MCP");
        let app = build_router(state, None);
        let body = vec![b' '; crate::mcp::config::MAX_ORDINARY_MCP_REQUEST_BYTES as usize + 1];
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/mcp")
                    .method("POST")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer mcp-secret")
                    .body(Body::from(body))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let payload = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        assert!(String::from_utf8_lossy(&payload).contains("MCP request exceeds"));
    }

    #[tokio::test]
    async fn mcp_route_rejects_unauthorized_requests_before_polling_the_body() {
        use std::convert::Infallible;
        use tokio_stream::StreamExt;

        let (app, _tmp) = app_for_tests();
        let panic_if_polled = tokio_stream::once(Ok::<_, Infallible>(axum::body::Bytes::new()))
            .then(|_| async {
                panic!("unauthorized MCP request body must not be collected");
                #[allow(unreachable_code)]
                Ok::<_, Infallible>(axum::body::Bytes::new())
            });
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/mcp")
                    .method("POST")
                    .body(Body::from_stream(panic_if_polled))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
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
    async fn unavailable_vault_keeps_liveness_status_and_spa_available() {
        let (_app, _tmp, mut state) = app_for_tests_with_state();
        *state.ready_vault.write().await = None;
        state.startup = StartupTracker::new(VaultRuntime::new(VaultSource::Local {
            vault_path: "/data/vault".into(),
        }));
        state
            .startup
            .runtime()
            .set_unavailable("vault_unreadable", "Vault could not be read.");
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

        // `/api/vault-status` was retired in #183; `/api/startup-status` is
        // the remaining unauthenticated probe, and it still reports the
        // unavailable Vault as a failed start without disclosing its
        // filesystem path.
        let status = app
            .oneshot(
                Request::builder()
                    .uri("/api/startup-status")
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
        assert_eq!(payload["state"], "failed");
        assert!(!payload.to_string().contains("/data/vault"));
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
    async fn vault_scoped_resolve_batch_resolves_obsidian_style_asset_targets() {
        let (app, tmp, _state) = app_for_tests_with_web_auth(None);
        let vault_root = tmp.path().join("obsidian");
        let vault_id = create_vault_with_files(
            &app,
            "Obsidian",
            &vault_root,
            &[
                ("97_Notes/Some note.md", "![[Some document.pdf]]\n"),
                ("98_Attachments/Some document.pdf", "%PDF-1.4\n"),
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
                    .body(Body::from(
                        r#"{"targets":[],"asset_targets":["Some document.pdf","Absent.png"],"note_path":"97_Notes/Some note"}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let payload = json_body(response).await;
        let results = payload["asset_results"]
            .as_array()
            .expect("asset results array");

        let found = results
            .iter()
            .find(|result| result["target"] == "Some document.pdf")
            .expect("pdf result");
        assert_eq!(found["path"], "98_Attachments/Some document.pdf");

        let absent = results
            .iter()
            .find(|result| result["target"] == "Absent.png")
            .expect("absent result");
        assert!(
            absent["path"].is_null(),
            "an unresolvable asset stays null so the client can render it missing"
        );
    }

    #[tokio::test]
    async fn vault_scoped_resolve_batch_oversized_json_body_reports_413_not_400() {
        // Finding 3 (#101): `vault_scoped_resolve_batch_handler` imports the
        // shared `json_rejection_response` from `vaults.rs`, which used to
        // flatten every JSON rejection to 400, losing a length-limit
        // rejection's real 413.
        let (app, tmp, _state) = app_for_tests_with_web_auth(None);
        let vault_id =
            create_vault_with_files(&app, "Notes", &tmp.path().join("notes"), &[], 0).await;
        let big_targets: Vec<String> = std::iter::repeat_n("x".repeat(64), 60_000).collect();
        let body = serde_json::json!({ "targets": big_targets }).to_string();
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{vault_id}/resolve-batch"))
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
    async fn post_start_model_choice_is_rejected_without_persisting_a_new_selection() {
        let (app, _tmp, state) = app_for_tests_with_state();
        assert_eq!(
            state.model_setup.selected().expect("initial selection"),
            SelectedModel::TermsRequired
        );

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/model/accept-gemma")
                    .method("POST")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(
            state
                .model_setup
                .selected()
                .expect("selection after refusal"),
            SelectedModel::TermsRequired,
            "a rejected post-start request must not persist a model the runtime did not adopt"
        );
    }

    #[tokio::test]
    async fn settings_routes_require_web_auth_and_are_absent_in_demo_mode() {
        let (protected, _tmp, _state) = app_for_tests_with_web_auth(Some(Arc::from("web-secret")));
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

        // #183 retired the two instance-wide consoles and the three routes
        // that fed them. They are gone from the router, so a credential that
        // opens `/api/settings` still finds nothing at any of them, and the
        // unauthenticated probe surface no longer includes `/api/vault-status`.
        for path in ["/api/index-status", "/api/git-status", "/api/vault-status"] {
            let retired = protected
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .header("authorization", "Bearer web-secret")
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_eq!(retired.status(), StatusCode::NOT_FOUND, "{path}");
        }
        // `/api/startup-status` is the probe that remains, and it is still
        // reachable without a credential.
        let startup_status = protected
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/startup-status")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(startup_status.status(), StatusCode::OK);
        assert_eq!(startup_status.headers()["cache-control"], "no-store");

        let (demo, _tmp, _) = app_for_tests_with_web_auth_and_demo_mode(None, true);
        for path in [
            "/api/settings",
            "/api/index-status",
            "/api/git-status",
            "/api/vault-status",
        ] {
            let absent = demo
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_eq!(absent.status(), StatusCode::NOT_FOUND, "{path}");
        }
    }

    /// The reindex consequence is still the server's to demand, and the new
    /// values are still durable before any rebuilding starts. What changed
    /// (#181) is what a confirmed save then does: it requests one Index turn
    /// per active Vault through the shared work coordinator instead of the
    /// legacy instance-wide rebuild, so `index_status` — which only ever
    /// described that legacy rebuild — stays untouched by a settings save.
    /// The per-Vault queueing itself is proven in
    /// `handlers::settings::tests`, which can observe the coordinator's queue.
    #[tokio::test]
    async fn reindex_settings_require_confirmation_and_persist_without_the_legacy_rebuild() {
        let (app, _tmp, state) = app_for_tests_with_state();

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
        assert_eq!(
            state.index_status.status().state,
            "up_to_date",
            "a settings save must no longer start the legacy instance-wide rebuild"
        );
        assert!(!state.index_status.status().stale);

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
        assert_eq!(state.index_status.status().state, "up_to_date");
        assert!(!state.index_status.status().stale);
    }

    #[tokio::test]
    async fn enabling_local_versioning_no_longer_asks_for_confirmation() {
        // #183 retired the `git_init` consequence along with the Versioning
        // console that explained it. Enabling local versioning on a vault that
        // is not yet a git repository now applies in one call and still
        // creates the repository.
        let (app, tmp, state) = app_for_tests_with_state();
        assert!(!tmp.path().join("vault/.git").exists());

        let response = app
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
        assert_eq!(response.status(), StatusCode::OK);
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
    async fn downgrade_onto_a_non_repo_vault_no_longer_asks_for_confirmation() {
        // The case that used to need both `git_downgrade` and `git_init`:
        // switching remote -> local when the vault is not a git repository at
        // all. Both consequences were retired in #183, so the save applies in
        // one call rather than across two 409 round-trips.
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

        let response = app
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
        assert_eq!(response.status(), StatusCode::OK);
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
            .clone()
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
    async fn vault_attachment_route_enforces_a_lowered_live_limit_while_reading_the_field() {
        // Regression for the R06 upload bound: the limit must be bound from the
        // live snapshot *before* the field is consumed, so lowering it takes
        // effect immediately rather than after the bytes are already buffered.
        // `attachment_upload_request` sends a 9-byte file.
        let (app, tmp, state) = app_for_tests_with_state();
        let vault_root = tmp.path().join("attachments");
        let vault_id = create_vault_with_files(&app, "Attachments", &vault_root, &[], 0).await;
        state
            .runtime_config
            .save([(
                "HATCHDOOR_MAX_ATTACHMENT_BYTES".to_string(),
                "8".to_string(),
            )])
            .expect("lower live attachment limit");

        let response = app
            .oneshot(attachment_upload_request(
                &vault_id,
                "Attachments/limited.png",
                None,
            ))
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(!vault_root.join("Attachments/limited.png").exists());
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
    async fn create_vault_oversized_json_body_reports_413_not_400() {
        // Finding 3 (#101): `json_rejection_response` (vaults.rs), shared by
        // create/edit-Vault and resolve-batch, used to flatten every JSON
        // rejection to 400, losing a length-limit rejection's real 413 —
        // unlike note-mutation routes' own `write_payload`, proven above.
        let (app, _tmp, _state) = app_for_tests_with_web_auth(None);
        let big = "x".repeat(3 * 1024 * 1024);
        let body = serde_json::json!({
            "expected_registry_revision": 0,
            "name": big,
            "enabled": true,
            "source": {"type": "local", "path": "/tmp/oversized-vault-body-test"},
        })
        .to_string();
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults")
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
        // The refusal itself is asserted at the mutation core's interface
        // (`vault_mutation.rs`). This stays as the route's mapping proof:
        // that `noise_excluded_write` reaches the client as a `400` with its
        // code intact, and that nothing was written.
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
        assert_eq!(body["demo_mode"], false);
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
            .clone()
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

        let refresh = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults/00000000-0000-4000-8000-000000000000/refresh")
                    .method("POST")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(refresh.status(), StatusCode::UNAUTHORIZED);
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
        assert_eq!(body["demo_mode"], true);

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
            ("POST", format!("/api/v1/vaults/{vault_id}/refresh")),
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
                    archive_folder: None,
                    commit_identity: None,
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
        assert_eq!(discovery_body["demo_mode"], true);

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

    /// Register `vaults` directly and reconcile them, for demo-mode tests:
    /// `POST /api/v1/vaults` is refused in demo mode, so the collection cannot
    /// be built through the API the way `create_vault_with_files` does.
    async fn register_vaults_directly(
        state: &AppState,
        vaults: &[(&str, &std::path::Path, bool)],
    ) -> Vec<crate::vault_registry::VaultId> {
        let mut revision = 0;
        let mut ids = Vec::new();
        for (name, path, enabled) in vaults {
            let snapshot = state
                .vault_registry
                .add(
                    revision,
                    crate::vault_registry::NewVaultDefinition {
                        name: (*name).to_string(),
                        enabled: *enabled,
                        source: crate::vault_registry::VaultSource::Local {
                            path: (*path).to_path_buf(),
                        },
                        exclude_patterns: Vec::new(),
                        https_credentials: None,
                        archive_folder: None,
                        commit_identity: None,
                    },
                )
                .expect("add vault to registry");
            revision = snapshot.revision();
            let id = snapshot
                .definitions()
                .find(|definition| definition.name() == *name)
                .expect("added definition")
                .vault_id();
            ids.push(id);
            state
                .vaults
                .reconcile_and_reconstruct(
                    &state.vault_registry,
                    &snapshot,
                    &state.vault_work,
                    &state.managed_git,
                )
                .await;
        }
        ids
    }

    #[tokio::test]
    async fn demo_mode_discovery_withholds_deployment_detail_and_disabled_vaults() {
        // #109's second criterion. Discovery is reachable unauthenticated in
        // demo mode, so the operator's deployment must not travel with it: no
        // absolute path, no exclusion list, no archive folder, no commit
        // author, no runtime error text (whose messages embed paths), and no
        // disabled definition. Everything a visitor browses with stays.
        let (app, tmp, state) = app_for_tests_with_web_auth_and_demo_mode(None, true);
        let published_root = tmp.path().join("published-vault");
        let withheld_root = tmp.path().join("withheld-vault");
        for root in [&published_root, &withheld_root] {
            std::fs::create_dir_all(root).expect("create vault directory");
            std::fs::write(root.join("Home.md"), "# Home\n").expect("write note");
        }
        let ids = register_vaults_directly(
            &state,
            &[
                ("Published", published_root.as_path(), true),
                ("Withheld", withheld_root.as_path(), false),
            ],
        )
        .await;

        let response = app
            .clone()
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

        let vaults = body["vaults"].as_array().expect("vaults array");
        assert_eq!(vaults.len(), 1, "only the enabled Vault is published");
        assert_eq!(vaults[0]["vault_id"], ids[0].to_string());
        assert_eq!(vaults[0]["name"], "Published");

        // Withheld: everything that describes the deployment.
        assert!(vaults[0].get("source").is_none(), "source must be absent");
        assert_eq!(
            vaults[0]["exclude_patterns"]
                .as_array()
                .expect("exclude_patterns array")
                .len(),
            0
        );
        for absent in [
            "archive_folder",
            "commit_identity",
            "activation_error",
            "search_error",
            "git_error",
            "watcher_error",
        ] {
            assert!(
                vaults[0].get(absent).is_none(),
                "{absent} must be absent in demo mode"
            );
        }

        // Kept: identity, the four independent statuses, and capabilities, so
        // honest participation reporting still works.
        for present in [
            "activation",
            "local_content",
            "search",
            "git",
            "watcher",
            "capabilities",
            "credential_configured",
        ] {
            assert!(
                vaults[0].get(present).is_some(),
                "{present} must survive the public projection"
            );
        }

        // No path from either Vault appears anywhere in the payload, including
        // the disabled one's name.
        let serialized = body.to_string();
        for leaked in [
            published_root.to_string_lossy().to_string(),
            withheld_root.to_string_lossy().to_string(),
            "Withheld".to_string(),
        ] {
            assert!(
                !serialized.contains(&leaked),
                "demo discovery leaked {leaked}: {serialized}"
            );
        }

        // The authenticated projection is unchanged: an ordinary instance still
        // states the source its Settings page edits, and still lists a disabled
        // definition.
        let (ordinary_app, ordinary_tmp, ordinary_state) =
            app_for_tests_with_web_auth_and_demo_mode(None, false);
        let ordinary_root = ordinary_tmp.path().join("ordinary-vault");
        let disabled_root = ordinary_tmp.path().join("disabled-vault");
        for root in [&ordinary_root, &disabled_root] {
            std::fs::create_dir_all(root).expect("create vault directory");
            std::fs::write(root.join("Home.md"), "# Home\n").expect("write note");
        }
        register_vaults_directly(
            &ordinary_state,
            &[
                ("Ordinary", ordinary_root.as_path(), true),
                ("Disabled", disabled_root.as_path(), false),
            ],
        )
        .await;
        let ordinary = ordinary_app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        let ordinary_body = json_body(ordinary).await;
        let ordinary_vaults = ordinary_body["vaults"].as_array().expect("vaults array");
        assert_eq!(
            ordinary_vaults.len(),
            2,
            "disabled definitions still listed"
        );
        // Discovery orders by immutable Vault ID, not insertion, so find the
        // Vault by name rather than assuming a position.
        let ordinary_entry = ordinary_vaults
            .iter()
            .find(|vault| vault["name"] == "Ordinary")
            .expect("the enabled Vault is listed");
        assert_eq!(ordinary_entry["source"]["type"], "local");
        assert_eq!(
            ordinary_entry["source"]["path"],
            ordinary_root.to_string_lossy().to_string()
        );
    }

    #[tokio::test]
    async fn demo_mode_withholds_demoted_notes_from_every_read_surface() {
        // #109's seventh criterion. A demo has no operator and no layer
        // toggle, so a demoted Note is not reachable by fetch, links, resolve,
        // download, or any layer selector, and its existence cannot be
        // inferred from the tree, graph, recent list, or statistics.
        let (app, tmp, state) = app_for_tests_with_web_auth_and_demo_mode(None, true);
        let vault_root = tmp.path().join("layered-vault");
        std::fs::create_dir_all(vault_root.join("sources")).expect("create layer directory");
        std::fs::write(
            vault_root.join("Home.md"),
            "# Home\n\nbasalt weathering\n\n[[Clipping]]\n",
        )
        .expect("write default note");
        std::fs::write(vault_root.join("sources/.hatchdoor-layer"), "sources")
            .expect("write layer marker");
        std::fs::write(
            vault_root.join("sources/Clipping.md"),
            "# Clipping\n\nbasalt weathering clipped article\n",
        )
        .expect("write demoted note");

        let vault_id = register_vaults_directly(&state, &[("Layered", vault_root.as_path(), true)])
            .await[0]
            .to_string();
        publish_vault_snapshot(&state, &vault_id, &vault_root);

        let get = async |uri: String| {
            app.clone()
                .oneshot(
                    Request::builder()
                        .uri(uri)
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response")
        };

        // Fetched, linked, downloaded: not found, exactly as a Note that does
        // not exist, so nothing distinguishes withheld from absent.
        for uri in [
            format!("/api/v1/vaults/{vault_id}/notes/clipping"),
            format!("/api/v1/vaults/{vault_id}/notes/clipping/links"),
            format!("/api/v1/vaults/{vault_id}/notes/clipping/download"),
        ] {
            let response = get(uri.clone()).await;
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{uri}");
        }

        // Resolved: the wikilink from the default Note does not resolve.
        let resolved = get(format!("/api/v1/vaults/{vault_id}/resolve?target=Clipping")).await;
        assert_eq!(resolved.status(), StatusCode::OK);
        assert!(json_body(resolved).await["slug"].is_null());

        // The default Note is still fully readable, and its outgoing link to
        // the demoted Note is dropped rather than dangling.
        let home = get(format!("/api/v1/vaults/{vault_id}/notes/home")).await;
        assert_eq!(home.status(), StatusCode::OK);
        let links = get(format!("/api/v1/vaults/{vault_id}/notes/home/links")).await;
        assert_eq!(links.status(), StatusCode::OK);
        let links_body = json_body(links).await;
        assert!(
            !links_body.to_string().contains("clipping"),
            "a link to a demoted Note names it: {links_body}"
        );

        // Searched, including through the layer selector.
        for query in ["", "&layers=all", "&layers=sources", "&layers=default"] {
            let uri = format!(
                "/api/v1/vaults/{vault_id}/search?q=basalt%20weathering&mode=keyword{query}"
            );
            let response = get(uri.clone()).await;
            assert_eq!(response.status(), StatusCode::OK, "{uri}");
            let body = json_body(response).await;
            assert!(
                !body.to_string().contains("clipping"),
                "search leaked the demoted Note for '{query}': {body}"
            );
        }

        // Inferred from a collection projection.
        for surface in ["tree", "graph", "recent", "stats"] {
            let uri = format!("/api/v1/vaults/{vault_id}/{surface}");
            let response = get(uri.clone()).await;
            assert_eq!(response.status(), StatusCode::OK, "{uri}");
            let body = json_body(response).await;
            assert!(
                !body.to_string().to_lowercase().contains("clipping"),
                "{surface} leaked the demoted Note: {body}"
            );
        }
        let stats = get(format!("/api/v1/vaults/{vault_id}/stats")).await;
        let stats_body = json_body(stats).await;
        assert_eq!(
            stats_body["data"][0]["note_count"], 1,
            "a demoted Note is not counted on a demo"
        );
    }

    #[tokio::test]
    async fn demo_mode_serves_only_assets_on_its_readable_surface() {
        let (demo, tmp, state) = app_for_tests_with_web_auth_and_demo_mode(None, true);
        let vault_root = tmp.path().join("layered-assets");
        std::fs::create_dir_all(vault_root.join("sources")).expect("create layer directory");
        std::fs::write(vault_root.join("Home.md"), "# Home\n\n![[visible.png]]\n")
            .expect("write visible note");
        std::fs::write(vault_root.join("visible.png"), b"visible").expect("write visible asset");
        std::fs::write(vault_root.join("sources/.hatchdoor-layer"), "sources")
            .expect("write layer marker");
        std::fs::write(
            vault_root.join("sources/Clipping.md"),
            "# Clipping\n\n![[hidden.png]]\n",
        )
        .expect("write demoted note");
        std::fs::write(vault_root.join("sources/hidden.png"), b"hidden")
            .expect("write hidden asset");

        let vault_id = register_vaults_directly(&state, &[("Layered", vault_root.as_path(), true)])
            .await[0]
            .to_string();

        for (path, expected) in [
            ("visible.png", StatusCode::OK),
            ("sources/hidden.png", StatusCode::NOT_FOUND),
        ] {
            let response = demo
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(format!("/api/v1/vaults/{vault_id}/assets/{path}"))
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_eq!(response.status(), expected, "{path}");
        }
    }

    #[tokio::test]
    async fn demo_mode_asset_surface_excludes_noise_and_honours_nested_layer_markers() {
        let (demo, tmp, state) = app_for_tests_with_web_auth_and_demo_mode(None, true);
        let vault_root = tmp.path().join("complete-asset-surface");
        for directory in ["sources/nested", ".trash", "private"] {
            std::fs::create_dir_all(vault_root.join(directory)).expect("create asset directory");
        }
        std::fs::write(vault_root.join("Home.md"), "# Home\n").expect("write visible note");
        std::fs::write(vault_root.join("asset-only.png"), b"public asset")
            .expect("write public asset without note");
        std::fs::write(vault_root.join("sources/.hatchdoor-layer"), "sources")
            .expect("write demoting marker");
        std::fs::write(
            vault_root.join("sources/asset-only.png"),
            b"hidden layer asset",
        )
        .expect("write hidden asset without note");
        std::fs::write(
            vault_root.join("sources/nested/.hatchdoor-layer"),
            "default",
        )
        .expect("write nested default reinclude marker");
        std::fs::write(
            vault_root.join("sources/nested/reincluded.png"),
            b"reinclude asset",
        )
        .expect("write reincluded asset without note");
        std::fs::write(vault_root.join(".trash/noise.png"), b"noise asset")
            .expect("write built-in noise asset");
        std::fs::write(vault_root.join("private/excluded.png"), b"excluded asset")
            .expect("write configured excluded asset");

        let snapshot = state
            .vault_registry
            .add(
                0,
                crate::vault_registry::NewVaultDefinition {
                    name: "Complete surface".to_string(),
                    enabled: true,
                    source: crate::vault_registry::VaultSource::Local {
                        path: vault_root.clone(),
                    },
                    exclude_patterns: vec!["private/".to_string()],
                    https_credentials: None,
                    archive_folder: None,
                    commit_identity: None,
                },
            )
            .expect("add vault to registry");
        let vault_id = snapshot
            .definitions()
            .find(|definition| definition.name() == "Complete surface")
            .expect("added vault")
            .vault_id()
            .to_string();
        state
            .vaults
            .reconcile_and_reconstruct(
                &state.vault_registry,
                &snapshot,
                &state.vault_work,
                &state.managed_git,
            )
            .await;

        for (path, expected) in [
            ("asset-only.png", StatusCode::OK),
            ("sources/asset-only.png", StatusCode::NOT_FOUND),
            ("sources/nested/reincluded.png", StatusCode::OK),
            (".trash/noise.png", StatusCode::NOT_FOUND),
            ("private/excluded.png", StatusCode::NOT_FOUND),
        ] {
            let response = demo
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(format!("/api/v1/vaults/{vault_id}/assets/{path}"))
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_eq!(response.status(), expected, "{path}");
        }
    }

    #[tokio::test]
    async fn ordinary_mode_retains_access_to_assets_under_demoted_layers() {
        let (app, tmp, state) = app_for_tests_with_web_auth_and_demo_mode(None, false);
        let vault_root = tmp.path().join("ordinary-layered-assets");
        std::fs::create_dir_all(vault_root.join("sources")).expect("create layer directory");
        std::fs::create_dir_all(vault_root.join(".trash")).expect("create noise directory");
        std::fs::write(vault_root.join("Home.md"), "# Home\n").expect("write visible note");
        std::fs::write(vault_root.join("sources/.hatchdoor-layer"), "sources")
            .expect("write layer marker");
        std::fs::write(vault_root.join("sources/hidden.png"), b"operator asset")
            .expect("write demoted asset");
        std::fs::write(vault_root.join(".trash/noise.png"), b"operator noise asset")
            .expect("write noise asset");

        let vault_id = register_vaults_directly(&state, &[("Layered", vault_root.as_path(), true)])
            .await[0]
            .to_string();
        for path in ["sources/hidden.png", ".trash/noise.png"] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(format!("/api/v1/vaults/{vault_id}/assets/{path}"))
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_eq!(response.status(), StatusCode::OK, "{path}");
        }
    }

    #[tokio::test]
    async fn an_ordinary_instance_still_reaches_demoted_notes() {
        // The other half of the #109 clamp: an authenticated operator's
        // instance keeps the established layer semantics, where demoting
        // removes a Note from the default *search* surface only.
        let (app, tmp, state) = app_for_tests_with_web_auth_and_demo_mode(None, false);
        let vault_root = tmp.path().join("layered-vault");
        std::fs::create_dir_all(vault_root.join("sources")).expect("create layer directory");
        std::fs::write(vault_root.join("Home.md"), "# Home\n\nbasalt weathering\n")
            .expect("write default note");
        std::fs::write(vault_root.join("sources/.hatchdoor-layer"), "sources")
            .expect("write layer marker");
        std::fs::write(
            vault_root.join("sources/Clipping.md"),
            "# Clipping\n\nbasalt weathering clipped article\n",
        )
        .expect("write demoted note");

        let vault_id = register_vaults_directly(&state, &[("Layered", vault_root.as_path(), true)])
            .await[0]
            .to_string();
        publish_vault_snapshot(&state, &vault_id, &vault_root);

        let note = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{vault_id}/notes/clipping"))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(note.status(), StatusCode::OK);

        let tree = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{vault_id}/tree"))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        let tree_body = json_body(tree).await;
        assert!(
            tree_body.to_string().contains("clipping"),
            "an ordinary instance still browses a demoted Note: {tree_body}"
        );
    }

    #[tokio::test]
    async fn demo_mode_with_web_token_configured_reads_stay_open_and_mutations_stay_403_not_401() {
        // #109 finding 2: a `HATCHDOOR_WEB_BEARER_TOKEN` configured alongside
        // demo mode must not turn public demo reads into `401`, and must not
        // let the token gate intercept a mutation before
        // `reject_demo_mutation` can answer with the required `403
        // demo_read_only` — regardless of whether a request presents no
        // token, the wrong token, or the configured one.
        let (app, tmp, state) =
            app_for_tests_with_web_auth_and_demo_mode(Some(Arc::from("secret")), true);
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
                    archive_folder: None,
                    commit_identity: None,
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

        // Reads: discovery, one-or-all, and exact content, all reachable with
        // no token presented at all.
        for uri in [
            "/api/v1/vaults".to_string(),
            "/api/v1/vaults/all/tree".to_string(),
            format!("/api/v1/vaults/{vault_id}/notes/home"),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(uri.clone())
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_eq!(response.status(), StatusCode::OK, "{uri}");
        }

        // Content mutations: unauthenticated, wrong-token, and
        // correctly-authenticated requests must all still get the stable
        // `403 demo_read_only` refusal, never a `401` from the token gate
        // running ahead of it.
        for token in [None, Some("wrong-token"), Some("secret")] {
            let mut builder = Request::builder()
                .uri(format!("/api/v1/vaults/{vault_id}/notes/home"))
                .method("PUT")
                .header("content-type", "application/json");
            if let Some(token) = token {
                builder = builder.header("authorization", format!("Bearer {token}"));
            }
            let response = app
                .clone()
                .oneshot(
                    builder
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
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "token={token:?}");
            let body = json_body(response).await;
            assert_eq!(body["code"], "demo_read_only", "token={token:?}");
        }

        // The Vault-control collection mutation (unrelated to any one Vault)
        // must refuse the same way.
        let collection_mutation = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(create_vault_request_body(
                        "Second",
                        &tmp.path().join("second"),
                        snapshot.revision(),
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(collection_mutation.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            json_body(collection_mutation).await["code"],
            "demo_read_only"
        );

        // The attachment upload route follows the same rule as the other
        // content mutations above, including when it presents a wrong token.
        let vault_id = vault_id.to_string();
        for token in [None, Some("wrong-token"), Some("secret")] {
            let response = app
                .clone()
                .oneshot(attachment_upload_request(
                    &vault_id,
                    "Attachments/demo.png",
                    token,
                ))
                .await
                .expect("response");
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "token={token:?}");
            assert_eq!(
                json_body(response).await["code"],
                "demo_read_only",
                "token={token:?}"
            );
        }
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
        assert_eq!(discovery_body["demo_mode"], false);
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
    async fn vaults_v1_disable_waits_for_an_active_foreground_mutation() {
        let (app, tmp, state) = app_for_tests_with_web_auth(None);
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
        let vault_id = json_body(created).await["vault"]["vault_id"]
            .as_str()
            .expect("vault id")
            .to_string();
        let parsed_vault_id = vault_id.parse().expect("canonical vault id");
        let mutation = state
            .vaults
            .runtime(parsed_vault_id)
            .expect("enabled runtime")
            .acquire_mutation()
            .await
            .expect("foreground mutation acquires its Vault lock");

        let disabled = app.clone().oneshot(
            Request::builder()
                .uri(format!(
                    "/api/v1/vaults/{vault_id}/disable?expected_registry_revision=1"
                ))
                .method("POST")
                .body(Body::empty())
                .expect("request"),
        );
        tokio::pin!(disabled);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(25), &mut disabled)
                .await
                .is_err(),
            "the disable response waits for the foreground mutation"
        );

        drop(mutation);
        assert_eq!(disabled.await.expect("response").status(), StatusCode::OK);
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
    async fn vaults_v1_refresh_enqueues_one_enabled_vault_without_inline_indexing() {
        let (app, tmp, state) = app_for_tests_with_web_auth(None);
        let vault_path = tmp.path().join("refreshable");
        std::fs::create_dir_all(&vault_path).expect("create vault directory");
        std::fs::write(vault_path.join("Home.md"), "# Home\n").expect("write note");
        let created = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(create_vault_request_body("Refreshable", &vault_path, 0))
                    .expect("request"),
            )
            .await
            .expect("response");
        let vault_id = json_body(created).await["vault"]["vault_id"]
            .as_str()
            .expect("vault id")
            .to_string();

        let refresh = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{vault_id}/refresh"))
                    .method("POST")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(refresh.status(), StatusCode::ACCEPTED);
        let refresh_body = json_body(refresh).await;
        assert_eq!(refresh_body["vault_id"], vault_id);
        assert_eq!(refresh_body["schedule"], "coalesced");
        assert_eq!(
            state.vaults.snapshot().vaults[&vault_id.parse().expect("vault id")].search,
            crate::vault_runtime::VaultSearchStatus::Unavailable,
            "the response returns before the queued Index turn builds a snapshot"
        );

        let all = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults/all/refresh")
                    .method("POST")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(all.status(), StatusCode::BAD_REQUEST);
        assert_eq!(json_body(all).await["code"], "invalid_vault_id");
    }

    #[tokio::test]
    async fn vaults_v1_refresh_reports_capability_and_coordinator_unavailability() {
        let (app, tmp, state) = app_for_tests_with_web_auth(None);
        let unavailable_path = tmp.path().join("unavailable");
        std::fs::create_dir_all(&unavailable_path).expect("create vault directory");
        let unavailable_snapshot = state
            .vault_registry
            .add(
                0,
                crate::vault_registry::NewVaultDefinition {
                    name: "Unavailable".to_string(),
                    enabled: true,
                    source: crate::vault_registry::VaultSource::Local {
                        path: unavailable_path,
                    },
                    exclude_patterns: Vec::new(),
                    https_credentials: None,
                    archive_folder: None,
                    commit_identity: None,
                },
            )
            .expect("add unavailable vault");
        state
            .vaults
            .reconcile_and_reconstruct(
                &state.vault_registry,
                &unavailable_snapshot,
                &state.vault_work,
                &state.managed_git,
            )
            .await;
        let unavailable_id = unavailable_snapshot.vault_ids().next().expect("vault id");
        state
            .vaults
            .runtime(unavailable_id)
            .expect("active runtime")
            .set_local_content_status(
                crate::vault_runtime::LocalContentStatus::Unavailable,
                Some(crate::vault_runtime::VaultRuntimeError {
                    code: "vault_read_unavailable".to_string(),
                    message: "Vault directory became unavailable".to_string(),
                    retryable: true,
                    detail: None,
                }),
            )
            .expect("publish unavailable local content");

        let unavailable = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{unavailable_id}/refresh"))
                    .method("POST")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(unavailable.status(), StatusCode::CONFLICT);
        assert_eq!(
            json_body(unavailable).await["code"],
            "capability_unavailable"
        );

        let disabled_snapshot = state
            .vault_registry
            .disable(1, unavailable_id)
            .expect("disable unavailable vault");
        state
            .vaults
            .reconcile_and_reconstruct(
                &state.vault_registry,
                &disabled_snapshot,
                &state.vault_work,
                &state.managed_git,
            )
            .await;
        let disabled = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{unavailable_id}/refresh"))
                    .method("POST")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(disabled.status(), StatusCode::CONFLICT);
        assert_eq!(json_body(disabled).await["code"], "vault_disabled");

        let available_path = tmp.path().join("available");
        std::fs::create_dir_all(&available_path).expect("create vault directory");
        std::fs::write(available_path.join("Home.md"), "# Home\n").expect("write note");
        let available_snapshot = state
            .vault_registry
            .add(
                2,
                crate::vault_registry::NewVaultDefinition {
                    name: "Available".to_string(),
                    enabled: true,
                    source: crate::vault_registry::VaultSource::Local {
                        path: available_path,
                    },
                    exclude_patterns: Vec::new(),
                    https_credentials: None,
                    archive_folder: None,
                    commit_identity: None,
                },
            )
            .expect("add available vault");
        state
            .vaults
            .reconcile_and_reconstruct(
                &state.vault_registry,
                &available_snapshot,
                &state.vault_work,
                &state.managed_git,
            )
            .await;
        let available_id = available_snapshot
            .definitions()
            .find(|definition| definition.name() == "Available")
            .expect("available definition")
            .vault_id();
        state.vault_work.drain_vault(available_id);

        let rejected = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{available_id}/refresh"))
                    .method("POST")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(rejected.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(json_body(rejected).await["code"], "vault_unavailable");
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
        assert_eq!(body["demo_mode"], false);
    }

    #[tokio::test]
    async fn vaults_v1_discovery_reports_registry_recovery_and_demo_mode_together() {
        // #133: #122's registry-recovery screens are reached on a demo too,
        // so the recovery arm must carry the demo posture rather than
        // sending the browser back to guessing at exactly the moment it has
        // least to go on.
        let (app, _tmp, state) = app_for_tests_with_web_auth_and_demo_mode(None, true);
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
        assert_eq!(body["recovery"]["code"], "vault_registry_recovery_required");
        assert_eq!(body["demo_mode"], true);
    }

    #[tokio::test]
    async fn vaults_v1_discovery_reports_legacy_migration_recovery_distinctly_from_registry_recovery()
     {
        // #150: an unreadable registry file and a failed legacy import look
        // different to the browser, even though the registry itself loads
        // fine (empty, revision 0) in the legacy-migration-recovery case.
        let (app, _tmp, state) = app_for_tests_with_web_auth(None);
        *state
            .legacy_migration_recovery
            .write()
            .expect("recovery lock") =
            Some(crate::vault_migration::LegacyMigrationRecovery::for_test(
                "legacy Vault path is not a readable directory",
            ));

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
        assert_eq!(body["vaults"], serde_json::json!([]));
        assert!(body["recovery"].is_null());
        assert_eq!(
            body["legacy_migration_recovery"]["code"],
            "legacy_migration_required"
        );
        assert_eq!(
            body["legacy_migration_recovery"]["message"],
            "legacy Vault path is not a readable directory"
        );
    }

    #[tokio::test]
    async fn vaults_v1_discovery_omits_legacy_migration_recovery_when_absent() {
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
        let body = json_body(response).await;
        assert!(body["legacy_migration_recovery"].is_null());
    }

    #[tokio::test]
    async fn start_with_no_vaults_requires_a_pending_recovery() {
        let (app, _tmp, _state) = app_for_tests_with_web_auth(None);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults/start-with-no-vaults")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"confirm":true}"#))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = json_body(response).await;
        assert_eq!(body["code"], "legacy_migration_recovery_not_pending");
    }

    #[tokio::test]
    async fn environment_cleanup_recovery_hides_vaults_and_refuses_mutations() {
        let (app, _tmp, state) = app_for_tests_with_web_auth(None);
        *state
            .legacy_migration_recovery
            .write()
            .expect("recovery lock") = Some(
            crate::vault_migration::LegacyMigrationRecovery::environment_cleanup(
                "Remove HATCHDOOR_EXCLUDE and restart.",
            ),
        );

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
        let body = json_body(discovery).await;
        assert_eq!(body["vaults"], serde_json::json!([]));
        assert_eq!(
            body["legacy_migration_recovery"]["code"],
            "legacy_environment_cleanup_required"
        );

        let mutation = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults/start-with-no-vaults")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"confirm":true}"#))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(mutation.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            json_body(mutation).await["code"],
            "legacy_environment_cleanup_required"
        );
    }

    #[tokio::test]
    async fn start_with_no_vaults_requires_explicit_confirmation() {
        let (app, _tmp, state) = app_for_tests_with_web_auth(None);
        *state
            .legacy_migration_recovery
            .write()
            .expect("recovery lock") =
            Some(crate::vault_migration::LegacyMigrationRecovery::for_test(
                "legacy Vault path is not a readable directory",
            ));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults/start-with-no-vaults")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"confirm":false}"#))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = json_body(response).await;
        assert_eq!(body["code"], "confirmation_required");
        // Refused, not silently discarded: the recovery flag survives.
        assert!(
            state
                .legacy_migration_recovery
                .read()
                .expect("recovery lock")
                .is_some()
        );
    }

    #[tokio::test]
    async fn start_with_no_vaults_confirmed_writes_an_empty_registry_and_clears_recovery() {
        let (app, _tmp, state) = app_for_tests_with_web_auth(None);
        *state
            .legacy_migration_recovery
            .write()
            .expect("recovery lock") =
            Some(crate::vault_migration::LegacyMigrationRecovery::for_test(
                "legacy Vault path is not a readable directory",
            ));

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults/start-with-no-vaults")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"confirm":true}"#))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = json_body(response).await;
        assert_eq!(body["registry_revision"], 1);
        assert!(body["vault"].is_null());
        assert!(
            state
                .legacy_migration_recovery
                .read()
                .expect("recovery lock")
                .is_none()
        );

        // Discovery now reads back the ordinary, no-longer-pending state.
        let discovery = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        let body = json_body(discovery).await;
        assert!(body["recovery"].is_null());
        assert!(body["legacy_migration_recovery"].is_null());
        assert_eq!(body["vaults"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn start_with_no_vaults_is_refused_in_demo_mode() {
        let (app, _tmp, state) = app_for_tests_with_web_auth_and_demo_mode(None, true);
        *state
            .legacy_migration_recovery
            .write()
            .expect("recovery lock") =
            Some(crate::vault_migration::LegacyMigrationRecovery::for_test(
                "legacy Vault path is not a readable directory",
            ));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults/start-with-no-vaults")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"confirm":true}"#))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = json_body(response).await;
        assert_eq!(body["code"], "demo_read_only");
    }

    #[tokio::test]
    async fn vaults_v1_discovery_reports_demo_mode_posture_at_many_enabled_vaults() {
        // #133: the demo indicator is instance-wide and constant regardless
        // of how many Vaults the collection holds, on both an ordinary
        // instance and a demo one.
        for demo_mode in [false, true] {
            let (app, tmp, state) = app_for_tests_with_web_auth_and_demo_mode(None, demo_mode);
            let mut expected_revision = 0;
            for name in ["Alpha", "Beta"] {
                let vault_path = tmp.path().join(name);
                std::fs::create_dir_all(&vault_path).expect("create vault directory");
                let snapshot = state
                    .vault_registry
                    .add(
                        expected_revision,
                        crate::vault_registry::NewVaultDefinition {
                            name: name.to_string(),
                            enabled: true,
                            source: crate::vault_registry::VaultSource::Local {
                                path: vault_path.clone(),
                            },
                            exclude_patterns: Vec::new(),
                            https_credentials: None,
                            archive_folder: None,
                            commit_identity: None,
                        },
                    )
                    .expect("add vault to registry");
                expected_revision = snapshot.revision();
                state
                    .vaults
                    .reconcile_and_reconstruct(
                        &state.vault_registry,
                        &snapshot,
                        &state.vault_work,
                        &state.managed_git,
                    )
                    .await;
            }

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
            assert_eq!(
                body["vaults"].as_array().expect("vaults array").len(),
                2,
                "demo_mode={demo_mode}"
            );
            assert_eq!(body["demo_mode"], demo_mode, "demo_mode={demo_mode}");
        }
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
    /// cache's already-published Vault snapshot. The process-level worker that
    /// dispatches `VaultWorkKind::Index` is not started by this router fixture,
    /// so publish fixture snapshots directly through the same shared cache
    /// `VaultReadCore`/`VaultSearchCore` read from.
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
    async fn vault_scoped_stats_detail_returns_the_rich_report_for_exactly_one_vault() {
        let (app, tmp, state) = app_for_tests_with_web_auth(None);
        let root = tmp.path().join("first");
        let vault_id = create_vault_with_files(
            &app,
            "First",
            &root,
            &[
                (
                    "Home.md",
                    "---\ntags: [alpha]\n---\n# Home\n\none two three\n\n[[Shared]]",
                ),
                ("Shared.md", "# Shared\n\nfour five"),
            ],
            0,
        )
        .await;
        publish_vault_snapshot(&state, &vault_id, &root);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/vaults/{vault_id}/stats/detail"))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = json_body(response).await;
        assert_eq!(body["vault_id"], vault_id);
        assert_eq!(body["stats"]["note_count"], 2);
        assert_eq!(body["stats"]["tag_count"], 1);
        assert_eq!(body["stats"]["link_count"], 1);
        assert_eq!(body["stats"]["most_linked"][0]["slug"], "shared", "{body}");

        let not_found = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/vaults/00000000-0000-4000-8000-000000000000/stats/detail")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(not_found.status(), StatusCode::NOT_FOUND);
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
        // The discriminator is that this is *not* a router 404: the canonical
        // Note URL dispatches to the SPA shell rather than falling through.
        // Which success-side status the shell reports depends on whether
        // `frontend/dist` happens to exist — absent in CI and in a fresh
        // checkout (503), present for anyone who has run `just prod-check` or
        // `npm run build` (200). Asserting one of them makes the test pass or
        // fail on an untracked build artifact rather than on the routing this
        // covers, so accept both and pin the reason only in the 503 case.
        let status = response.status();
        assert!(
            status == StatusCode::OK || status == StatusCode::SERVICE_UNAVAILABLE,
            "canonical Note URL must reach the SPA shell, got {status}"
        );
        if status == StatusCode::SERVICE_UNAVAILABLE {
            let bytes = to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("body");
            assert!(String::from_utf8_lossy(&bytes).contains("Frontend not built"));
        }
    }
}
