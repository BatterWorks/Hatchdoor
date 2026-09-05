//! The `/mcp` transport boundary. rmcp owns Streamable HTTP serving and
//! JSON-RPC framing (ADR-17); this module mounts rmcp's
//! [`StreamableHttpService`] behind Hatchdoor's per-request security gate so
//! the ordering enabled check → token configured → Origin allowlist →
//! constant-time bearer compare → protocol-version header is unchanged, and
//! write-enabled requests keep their larger live-configured body allowance.

use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::any_service;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::tower::{
    StreamableHttpServerConfig, StreamableHttpService,
};
use serde_json::Value;

use crate::app_state::AppState;

use super::adapter::HatchdoorMcpHandler;
use super::auth::{reject_unsupported_protocol_version, validate_mcp_request};
use super::config::McpConfig;
use super::limits::{self, RateLimiter, RequestClass};
use super::protocol::jsonrpc_error_response;
use super::subscriptions::{McpBearerToken, SubscriptionRegistry};

/// The process-wide MCP transport: one rmcp service instance whose session
/// manager outlives individual requests (legacy clients hold a session across
/// POSTs). Handler state is captured once from the composition root.
#[derive(Clone)]
pub struct HatchdoorMcpTransport {
    service: StreamableHttpService<HatchdoorMcpHandler, LocalSessionManager>,
    /// Layered resource protection (#171): one rolling quota window per token
    /// plus the process-wide concurrency pools, shared by every handler this
    /// service constructs.
    limiter: Arc<RateLimiter>,
}

impl HatchdoorMcpTransport {
    pub fn new(state: AppState) -> Self {
        let mut config = StreamableHttpServerConfig::default();
        // Legacy 2025-11-25 clients keep today's initialize/session flow;
        // modern requests are always served statelessly by rmcp itself.
        config.legacy_session_mode = true;
        // Host validation is Hatchdoor's own concern: the operator binds a
        // configured host behind a bearer token, and DNS-rebinding is blocked
        // by our per-request Origin allowlist. rmcp's default loopback-only
        // Host list would break non-loopback deployments.
        config.allowed_hosts = Vec::new();
        // Origin allowlisting stays in our middleware for exact legacy
        // semantics; leaving rmcp's list empty disables its duplicate check.
        config.allowed_origins = Vec::new();
        // Same static outer guard as before: the middleware re-applies the
        // live per-capability limit precisely.
        config.max_request_body_bytes = McpConfig::maximum_request_body_limit();
        // Modern `2026-07-28` requests are always routed statelessly, so this
        // gate applies to exactly them: each request must carry the
        // `MCP-Protocol-Version` header plus per-request `_meta` whose declared
        // version and required capability metadata match. Legacy session-routed
        // traffic keeps today's rules unchanged.
        config.stateless_protocol_metadata_required = true;
        // Long-lived `subscriptions/listen` streams stay alive across idle
        // periods with rmcp's SSE keep-alive comments (#170); 15s is also the
        // library default but is pinned here so the behavior is explicit and
        // survives an upstream default change.
        config.sse_keep_alive = Some(std::time::Duration::from_secs(15));
        // The per-token live-subscription budget (#170) shared by every
        // handler instance this service constructs.
        let subscriptions = Arc::new(SubscriptionRegistry::new());
        Self {
            limiter: Arc::new(RateLimiter::new()),
            service: StreamableHttpService::new(
                move || {
                    Ok(HatchdoorMcpHandler::new(
                        state.clone(),
                        subscriptions.clone(),
                    ))
                },
                Arc::new(LocalSessionManager::default()),
                config,
            ),
        }
    }

    /// This transport's rate limiter, so the Vault asset route can spend the
    /// same per-token quota and concurrency budget for a request admitted on the
    /// MCP bearer token (#176). Sharing the instance is the point: an MCP client
    /// must not get a second, independent budget by fetching attachment bytes
    /// through `get_attachment`'s `download_url` instead of over `/mcp`.
    pub fn limiter(&self) -> Arc<RateLimiter> {
        self.limiter.clone()
    }

    /// The `/mcp` sub-router: rmcp's Streamable HTTP service (GET/SSE + POST +
    /// DELETE) behind the authorization/body-limit/rate-limit middleware.
    /// Merged into the main application router by the composition root. The
    /// rate limiter is captured here so one transport's middleware shares its
    /// quota windows and concurrency pools across all requests.
    pub fn router(&self, state: &AppState) -> Router<AppState> {
        let limiter = self.limiter.clone();
        let state = state.clone();
        Router::new()
            .route("/mcp", any_service(self.clone()))
            .layer(axum::middleware::from_fn(
                move |request: Request, next: Next| {
                    let state = state.clone();
                    let limiter = limiter.clone();
                    async move {
                        authorize_mcp_transport(State(state.clone()), limiter, request, next).await
                    }
                },
            ))
    }
}

impl tower::Service<Request<Body>> for HatchdoorMcpTransport {
    type Response =
        <StreamableHttpService<HatchdoorMcpHandler, LocalSessionManager> as tower::Service<
            Request<Body>,
        >>::Response;
    type Error = std::convert::Infallible;
    type Future =
        <StreamableHttpService<HatchdoorMcpHandler, LocalSessionManager> as tower::Service<
            Request<Body>,
        >>::Future;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        <StreamableHttpService<HatchdoorMcpHandler, LocalSessionManager> as tower::Service<
            Request<Body>,
        >>::poll_ready(&mut self.service, cx)
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        self.service.call(req)
    }
}

async fn live_mcp_config(State(state): State<AppState>) -> Result<McpConfig, Response> {
    let snapshot = state.runtime_snapshot();
    AppState::runtime_mcp_config(&snapshot)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error).into_response())
}

/// Per-request gate run before any body is collected or dispatched. This is
/// the preserved security ordering from the hand-written adapter:
/// enabled check → token configured → Origin allowlist → constant-time bearer
/// compare (`auth::validate_mcp_request`) → buffered-body protocol-version
/// check with id echo → layered resource protection (#171).
async fn authorize_mcp_transport(
    State(state): State<AppState>,
    limiter: Arc<RateLimiter>,
    request: Request,
    next: Next,
) -> Response {
    let (mut parts, body) = request.into_parts();
    let config = match live_mcp_config(State(state)).await {
        Ok(config) => config,
        Err(response) => return response,
    };
    if let Err(response) = validate_mcp_request(&parts.headers, &config) {
        return *response;
    }

    // Attach the validated credential so long-lived subscriptions can be
    // attributed (and capped) per bearer token without re-reading headers in
    // the adapter. rmcp exposes these request extensions to handlers.
    if let Some(token) = &config.bearer_token {
        parts
            .extensions
            .insert(McpBearerToken(Arc::from(token.as_str())));
    }

    if parts.method == Method::POST {
        // Bind the capability-aware body limit from the same live snapshot:
        // read-only MCP accepts only the small ordinary JSON-RPC bound, while
        // write mode admits the base64 attachment allowance plus framing.
        // Buffer only up to this capability's live bound: an oversized body is
        // rejected mid-stream instead of being fully buffered first.
        let body = match to_bytes(body, config.request_body_limit()).await {
            Ok(body) => body,
            Err(_) => {
                return jsonrpc_error_response(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    Value::Null,
                    -32600,
                    "MCP request exceeds the configured request size limit".to_string(),
                );
            }
        };

        // The retired/unknown revision check needs the buffered POST body so
        // its JSON-RPC error can echo the request `id` (SEP-2575). Everything
        // above this line deliberately rejects before reading any bytes.
        let request_id = serde_json::from_slice::<Value>(&body)
            .ok()
            .and_then(|parsed| parsed.get("id").cloned())
            .unwrap_or(Value::Null);
        if let Err(response) = reject_unsupported_protocol_version(&parts.headers, request_id) {
            return *response;
        }

        // Layered resource protection (#171): tool calls are quota-limited per
        // bearer token and concurrency-capped process-wide; protocol,
        // discovery, and list handling stay outside both layers. Rejections
        // happen here — before dispatch — so they carry HTTP 429 with a
        // Retry-After header instead of a JSON-RPC error.
        if config.rate_limits_enabled
            && let Some(class) = classify_post_body(&body)
            && let Some(token) = parts.extensions.get::<McpBearerToken>()
        {
            // Concurrency first, so a busy-rejected call does not also spend
            // quota budget on a request that never dispatched.
            let guard = match limiter.try_acquire(class).await {
                Ok(guard) => guard,
                Err(retry_in) => return too_many_requests(limits::retry_after_seconds(retry_in)),
            };
            if let Err(retry_in) = limiter.check_quota(token, std::time::Instant::now()) {
                return too_many_requests(limits::retry_after_seconds(retry_in));
            }
            // The guard is deliberately held across dispatch: its Drop is what
            // frees the concurrency slots when the response (or an early
            // error/cancel) completes.
            let request = Request::from_parts(parts, Body::from(body));
            let response = next.run(request).await;
            drop(guard);
            return response;
        }

        let request = Request::from_parts(parts, Body::from(body));
        return next.run(request).await;
    }

    next.run(Request::from_parts(parts, body)).await
}

/// Classify a buffered POST body for layered limiting (#171): `None` for
/// exempt traffic (protocol lifecycle, discovery, list handling, notifications,
/// and anything unparseable — which downstream JSON-RPC framing rejects
/// without ever reaching a tool). Only `tools/call` bodies yield a class.
/// The raw-byte scan is only used to *skip* work when it cannot hide a call:
/// a body with no backslash decodes every character literally, so an absent
/// marker there proves absence. Anything else falls back to parsing so an
/// escaped method name (`"\\u0074ools/call"`) cannot slip past the quota.
fn classify_post_body(body: &[u8]) -> Option<RequestClass> {
    const MARKER: &[u8] = b"tools/call";
    let marker_absent = !body.windows(MARKER.len()).any(|window| window == MARKER);
    if marker_absent && !body.contains(&b'\\') {
        return None;
    }
    let parsed: Value = serde_json::from_slice(body).ok()?;
    let class = limits::classify(
        parsed.get("method").and_then(Value::as_str),
        parsed["params"]["name"].as_str(),
    );
    (class != RequestClass::Exempt).then_some(class)
}

/// The over-limit rejection (#171): HTTP 429 plus `Retry-After` in whole
/// seconds. Deliberately not a JSON-RPC envelope — the request never reached
/// dispatch, so no in-flight request id is owed an answer.
fn too_many_requests(retry_after_seconds: u64) -> Response {
    let mut response = (
        StatusCode::TOO_MANY_REQUESTS,
        "MCP tool-call limit reached; retry later",
    )
        .into_response();
    response
        .headers_mut()
        .insert(axum::http::header::RETRY_AFTER, retry_after_seconds.into());
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::test_embedder;
    use crate::mcp::limits::TOOL_CALLS_PER_MINUTE;
    use axum::body::to_bytes;
    use serde_json::json;
    use std::sync::Arc;
    use tempfile::TempDir;
    use tower::ServiceExt;

    const TEST_TOKEN: &str = "test-token";

    // ---------------------------------------------------------------------------
    // State fixtures (same shapes the hand-written adapter's suite used)
    // ---------------------------------------------------------------------------

    fn mcp_runtime_config(write_enabled: bool) -> crate::runtime_config::RuntimeConfig {
        let config = crate::runtime_config::RuntimeConfig::for_tests();
        config
            .save([
                ("HATCHDOOR_MCP_ENABLED".to_string(), "true".to_string()),
                (
                    "HATCHDOOR_MCP_WRITE_ENABLED".to_string(),
                    write_enabled.to_string(),
                ),
                (
                    "HATCHDOOR_MCP_BEARER_TOKEN".to_string(),
                    TEST_TOKEN.to_string(),
                ),
            ])
            .expect("configure MCP");
        config
    }

    /// A Vault-less `AppState`: an empty registry over a fresh in-memory
    /// snapshot database. `scoped_test_state` registers and indexes a Vault
    /// into it.
    fn base_state(tmp: &TempDir) -> AppState {
        let embedder = test_embedder();
        let sqlite = Arc::new(crate::cache::SqliteCache::in_memory(384).expect("in-memory cache"));
        let (mcp_tools_changed, _) = tokio::sync::broadcast::channel(16);
        let (vault_work, _vault_worker) = crate::vault_work::VaultWorkCoordinator::new();
        let managed_git = Arc::new(crate::git::ManagedGitScheduler::without_durable_state(
            vault_work.clone(),
        ));
        AppState {
            vault_registry: crate::vault_registry::VaultRegistryStore::new(
                tmp.path().join("state/vaults.json"),
            ),
            vaults: crate::vault_runtime::VaultCollectionRuntime::new(),
            vault_work,
            managed_git,
            legacy_migration_recovery: Arc::new(std::sync::RwLock::new(None)),
            startup_sqlite: sqlite,
            mcp_tools_changed,
            runtime_embedder: Arc::new(crate::embed::RuntimeEmbedder::new()),
            embedder,
            model_setup: Arc::new(crate::model_setup::ModelSetup::new(
                tmp.path().join("models"),
            )),
            model_setup_started: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            web_auth_enabled: false,
            demo_mode: false,
            runtime_config: mcp_runtime_config(false),
            startup: crate::startup::StartupTracker::ready(),
        }
    }

    /// A one-Vault state with MCP enabled (read-only), matching what a real
    /// enabled deployment serves.
    fn test_state() -> (AppState, TempDir) {
        let tmp = TempDir::new().expect("temp dir");
        let vault_root = tmp.path().join("vault");
        std::fs::create_dir_all(&vault_root).expect("create vault");
        std::fs::write(vault_root.join("Home.md"), "# Home\nalpha token\n[[Plan]]")
            .expect("write home");
        std::fs::write(vault_root.join("Plan.md"), "# Plan\nlinked note").expect("write plan");
        let mut state = base_state(&tmp);
        state.runtime_config = mcp_runtime_config(false);
        let state = scoped_test_state(state, vault_root);
        (state, tmp)
    }

    fn write_state() -> (AppState, TempDir) {
        let (state, tmp) = test_state();
        let mut state = state;
        state.runtime_config = mcp_runtime_config(true);
        (state, tmp)
    }

    /// `write_state`, but keeping the work worker `base_state` otherwise drops
    /// on the floor, so a test can run the Index turn `refresh_vault` admits.
    /// The coordinator is swapped wholesale rather than threaded through
    /// `base_state`: `refresh` reaches the queue through `state.vault_work`
    /// alone, and nothing here drives managed Git.
    fn write_state_with_worker() -> (AppState, crate::vault_work::VaultWorkWorker, TempDir) {
        let (mut state, tmp) = write_state();
        let (vault_work, worker) = crate::vault_work::VaultWorkCoordinator::new();
        state.vault_work = vault_work;
        (state, worker, tmp)
    }

    /// A write-enabled state whose single registered Vault has lost its
    /// directory since it was registered, so it reconciles with no usable
    /// local Markdown and `capabilities.browse` false. The registry refuses an
    /// unreadable path outright, which is why the directory exists for the
    /// `add` and is removed before the reconcile that matters.
    fn unusable_local_content_write_state() -> (AppState, TempDir) {
        use crate::vault_registry::{NewVaultDefinition, VaultSource};

        let tmp = TempDir::new().expect("temp dir");
        let vault_root = tmp.path().join("vault-gone");
        std::fs::create_dir_all(&vault_root).expect("create vault");
        let mut state = base_state(&tmp);
        state.runtime_config = mcp_runtime_config(true);
        let snapshot = state
            .vault_registry
            .add(
                0,
                NewVaultDefinition {
                    name: "Vault with no local content".to_string(),
                    enabled: true,
                    source: VaultSource::Local {
                        path: vault_root.clone(),
                    },
                    exclude_patterns: Vec::new(),
                    https_credentials: None,
                    archive_folder: None,
                    commit_identity: None,
                },
            )
            .expect("register test Vault");
        std::fs::remove_dir_all(&vault_root).expect("remove the Vault directory");
        state.vaults.reconcile(&state.vault_registry, &snapshot);
        (state, tmp)
    }

    /// A zero-Vault registry, for discovery/repair reachability tests (#103).
    fn empty_test_state() -> (AppState, TempDir) {
        let tmp = TempDir::new().expect("temp dir");
        let state = base_state(&tmp);
        (state, tmp)
    }

    /// A vault with a demoted `sources/` layer and a demoted note.
    fn layered_test_state() -> (AppState, TempDir) {
        let tmp = TempDir::new().expect("temp dir");
        let vault_root = tmp.path().join("vault");
        std::fs::create_dir_all(vault_root.join("wiki")).expect("wiki dir");
        std::fs::create_dir_all(vault_root.join("sources")).expect("sources dir");
        std::fs::write(
            vault_root.join("sources/.hatchdoor-layer"),
            "name: sources\ndescription: Raw captured clippings.\n",
        )
        .expect("marker");
        std::fs::write(
            vault_root.join("wiki/Page.md"),
            "---\ntags: [topic/x]\n---\n# Page\nmelatonin body",
        )
        .expect("page");
        std::fs::write(
            vault_root.join("sources/Clip.md"),
            "---\ntags: [topic/x]\n---\n# Clip\nmelatonin clipping",
        )
        .expect("clip");
        let state = base_state(&tmp);
        let state = scoped_test_state(state, vault_root);
        (state, tmp)
    }

    fn layered_write_state() -> (AppState, TempDir) {
        let (state, tmp) = layered_test_state();
        let mut state = state;
        state.runtime_config = mcp_runtime_config(true);
        (state, tmp)
    }

    /// The registered test Vault's directory on disk, for the tests that
    /// write fixture files straight into it. Replaces the retired legacy
    /// `AppState::vault_path` accessor.
    fn registered_vault_path(state: &AppState) -> std::path::PathBuf {
        use crate::vault_registry::{VaultRegistryState, VaultSource};
        let snapshot = match state.vault_registry.load().expect("load registry") {
            VaultRegistryState::Ready(snapshot) => snapshot,
            VaultRegistryState::Recovery(_) => panic!("test registry recovery"),
        };
        match snapshot
            .definitions()
            .next()
            .expect("one registered test Vault")
            .source()
        {
            VaultSource::Local { path } => path.clone(),
            other => panic!("test Vault is not Local: {other:?}"),
        }
    }

    fn scoped_test_state(state: AppState, vault_root: std::path::PathBuf) -> AppState {
        use crate::vault_registry::{NewVaultDefinition, VaultRegistryState, VaultSource};

        let snapshot = state
            .vault_registry
            .add(
                0,
                NewVaultDefinition {
                    name: "MCP test Vault".to_string(),
                    enabled: true,
                    source: VaultSource::Local {
                        path: vault_root.clone(),
                    },
                    exclude_patterns: Vec::new(),
                    https_credentials: None,
                    archive_folder: None,
                    commit_identity: None,
                },
            )
            .expect("register test Vault");
        state.vaults.reconcile(&state.vault_registry, &snapshot);
        let vault_id = match state.vault_registry.load().expect("load registry") {
            VaultRegistryState::Ready(snapshot) => snapshot
                .definitions()
                .next()
                .expect("test definition")
                .vault_id(),
            VaultRegistryState::Recovery(_) => panic!("test registry recovery"),
        };
        let index = crate::vault::VaultIndex::build(&vault_root).expect("test Vault index");
        state
            .startup_sqlite
            .replace_vault_snapshot(vault_id, &index, state.embedder.as_ref())
            .expect("publish test Vault snapshot");
        state
    }

    // ---------------------------------------------------------------------------
    // Full-stack wire helpers
    // ---------------------------------------------------------------------------

    fn transport(state: &AppState) -> Router {
        HatchdoorMcpTransport::new(state.clone())
            .router(state)
            .layer(axum::extract::DefaultBodyLimit::max(
                McpConfig::maximum_request_body_limit(),
            ))
            .with_state(state.clone())
    }

    fn auth_headers(token: &str) -> Vec<(&'static str, String)> {
        vec![("authorization", format!("Bearer {token}"))]
    }

    async fn send(
        app: Router,
        method: &str,
        headers: Vec<(&'static str, String)>,
        body: Option<String>,
    ) -> Response {
        let mut builder = Request::builder().method(method).uri("/mcp");
        for (name, value) in headers {
            builder = builder.header(name, value);
        }
        builder = builder.header("host", "localhost");
        let request = match body {
            Some(body) => builder.body(Body::from(body)).expect("request"),
            None => builder.body(Body::empty()).expect("request"),
        };
        app.oneshot(request).await.expect("response")
    }

    /// Extract the JSON-RPC message from an rmcp SSE response body. The first
    /// `data:` payload that parses as a JSON-RPC message wins; priming/retry
    /// events are skipped.
    async fn response_message(response: Response) -> Value {
        if !response.status().is_success() {
            let status = response.status();
            let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            panic!("HTTP {}: {}", status, String::from_utf8_lossy(&bytes));
        }
        assert!(
            response.status().is_success(),
            "wire status {}",
            response.status()
        );
        let content_type = response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        if !content_type.contains("event-stream") {
            return serde_json::from_slice(&bytes)
                .unwrap_or_else(|error| panic!("plain-JSON body parses ({error}): {bytes:?}"));
        }
        let text = String::from_utf8(bytes.to_vec()).expect("SSE is UTF-8");
        for line in text.lines() {
            let Some(data) = line.strip_prefix("data:") else {
                continue;
            };
            let Ok(message) = serde_json::from_str::<Value>(data.trim()) else {
                continue;
            };
            if message.get("jsonrpc").is_some() && message.get("method").is_none() {
                return message;
            }
        }
        panic!("no JSON-RPC message in SSE stream: {text}");
    }

    struct Session {
        id: String,
    }

    /// Perform the legacy initialize handshake plus notifications/initialized.
    /// Returns the negotiated protocol version from the initialize result.
    async fn initialize(app: &Router) -> (Session, Value) {
        let headers = auth_headers(TEST_TOKEN);
        let mut init_headers = headers.clone();
        init_headers.push(("accept", "application/json, text/event-stream".into()));
        init_headers.push(("content-type", "application/json".into()));

        let response = send(
            app.clone(),
            "POST",
            init_headers,
            Some(
                json!({
                    "jsonrpc": "2.0", "id": 1, "method": "initialize",
                    "params": {"protocolVersion": "2025-11-25", "capabilities": {},
                               "clientInfo": {"name":"golden-test","version":"1"}}
                })
                .to_string(),
            ),
        )
        .await;

        let session_id = response
            .headers()
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
            .expect("initialize returns Mcp-Session-Id");
        let message = response_message(response).await;
        let _ = send(
            app.clone(),
            "POST",
            {
                let mut h = auth_headers(TEST_TOKEN);
                h.push(("mcp-session-id", session_id.clone()));
                h.push(("content-type", "application/json".into()));
                h
            },
            Some(json!({"jsonrpc":"2.0","method":"notifications/initialized"}).to_string()),
        )
        .await;
        (Session { id: session_id }, message["result"].clone())
    }

    async fn rpc(app: &Router, session: &Session, payload: Value) -> Response {
        let mut headers = auth_headers(TEST_TOKEN);
        headers.push(("mcp-session-id", session.id.clone()));
        headers.push(("accept", "application/json, text/event-stream".into()));
        headers.push(("content-type", "application/json".into()));
        send(app.clone(), "POST", headers, Some(payload.to_string())).await
    }

    /// Inject the test Vault's `vault_id`/`scope` into tool arguments, as a
    /// well-behaved agent would after reading list_vaults.
    fn scoped_arguments(state: &AppState, name: &str, mut arguments: Value) -> Value {
        let Some(vault_id) = state.vaults.snapshot().vaults.keys().next().copied() else {
            return arguments;
        };
        if matches!(
            name,
            "search_notes" | "get_tree" | "get_stats" | "get_graph" | "recently_modified"
        ) {
            arguments["scope"] = json!(vault_id);
        } else if !matches!(
            name,
            "list_vaults"
                | "get_model_setup_status"
                | "accept_gemma_terms"
                | "decline_gemma_terms"
                | "batch"
        ) {
            arguments["vault_id"] = json!(vault_id);
        }
        arguments
    }

    /// Call one tool with the arguments exactly as written, without
    /// `scoped_arguments` substituting the registered Vault's real ID — for
    /// the tests that are about a malformed `vault_id` or `scope`.
    async fn call_tool_unscoped(state: &AppState, name: &str, arguments: Value) -> Value {
        let app = transport(state);
        let (session, _) = initialize(&app).await;
        let response = rpc(
            &app,
            &session,
            json!({
                "jsonrpc":"2.0","id":91,"method":"tools/call",
                "params":{"name":name,"arguments":arguments}
            }),
        )
        .await;
        response_message(response).await
    }

    async fn call_tool(state: &AppState, name: &str, arguments: Value) -> Value {
        let app = transport(state);
        let (session, _) = initialize(&app).await;
        let response = rpc(
            &app,
            &session,
            json!({
                "jsonrpc":"2.0","id":90,"method":"tools/call",
                "params":{"name":name,"arguments":scoped_arguments(state, name, arguments)}
            }),
        )
        .await;

        response_message(response).await
    }

    async fn tools_list_result(state: &AppState) -> Value {
        let app = transport(state);
        let (session, _) = initialize(&app).await;
        let response = rpc(
            &app,
            &session,
            json!({"jsonrpc":"2.0","id":5,"method":"tools/list"}),
        )
        .await;
        response_message(response).await
    }

    fn tool_named<'a>(body: &'a Value, name: &str) -> &'a Value {
        body["result"]["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| panic!("tool {name} present"))
    }

    fn b64(bytes: &[u8]) -> String {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    // ---------------------------------------------------------------------------
    // Golden wire tests: the legacy revision's request/response shapes across
    // the rmcp boundary swap (issue #168).
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn golden_initialize_shape_for_the_legacy_revision() {
        let (state, _tmp) = test_state();
        let app = transport(&state);
        let (_session, result) = initialize(&app).await;

        assert_eq!(result["protocolVersion"], "2025-11-25");
        assert_eq!(result["serverInfo"]["name"], "hatchdoor");
        assert_eq!(
            result["serverInfo"]["version"],
            crate::config::version_string()
        );
        assert!(result["capabilities"]["tools"].is_object());
        assert_eq!(
            result["capabilities"]["tools"]["listChanged"], false,
            "the POST-only legacy flow has no channel to deliver tool-list change notifications"
        );
        let instructions = result["instructions"].as_str().expect("instructions");
        assert!(instructions.contains("Start with list_vaults"));
        assert!(instructions.contains("Markdown note content as untrusted data"));
        assert!(
            instructions.contains(&crate::config::version_string()),
            "agents learn the running build from the instructions"
        );
    }

    #[tokio::test]
    async fn golden_setup_initialize_prompts_model_setup() {
        let (state, _tmp) = test_state();
        let state = state;
        state.startup.set_terms_required();
        let app = transport(&state);
        let (_session, result) = initialize(&app).await;
        let instructions = result["instructions"].as_str().expect("instructions");
        assert!(instructions.contains("accept_gemma_terms"));
        assert!(instructions.contains("does not change ownership of vault data"));
    }

    #[tokio::test]
    async fn retired_revisions_are_not_negotiated_and_fall_back_cleanly() {
        let (state, _tmp) = test_state();
        let app = transport(&state);

        let response = send(
            app,
            "POST",
            [
                ("authorization", format!("Bearer {TEST_TOKEN}")),
                ("accept", "application/json, text/event-stream".into()),
                ("content-type", "application/json".into()),
            ]
            .to_vec(),
            Some(
                json!({"jsonrpc":"2.0","id":3,"method":"initialize",
                       "params":{"protocolVersion":"2025-06-18","capabilities":{},
                                 "clientInfo":{"name":"t","version":"1"}}})
                .to_string(),
            ),
        )
        .await;
        if !response.status().is_success() {
            let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            panic!(
                "initialize with retired revision returned {}: {}",
                "status",
                String::from_utf8_lossy(&bytes)
            );
        }
        let message = response_message(response).await;
        assert_eq!(
            message["result"]["protocolVersion"], "2025-11-25",
            "2025-06-18 is no longer served; the newest legacy revision answers instead"
        );
    }

    #[tokio::test]
    async fn retired_protocol_version_header_is_rejected_cleanly() {
        let (state, _tmp) = test_state();
        let app = transport(&state);

        for retired in ["2025-06-18", "2025-03-26", "2024-11-05"] {
            let response = send(
                app.clone(),
                "POST",
                [
                    ("authorization", format!("Bearer {TEST_TOKEN}")),
                    ("mcp-protocol-version", retired.to_string()),
                    ("content-type", "application/json".into()),
                ]
                .to_vec(),
                Some(json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}).to_string()),
            )
            .await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{retired}");
            let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let message: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(message["error"]["code"], -32002, "{retired}");
            // SEP-2575: the JSON-RPC error echoes the request id, never null.
            assert_eq!(message["id"], 2, "{retired}");
            assert!(
                message["error"]["message"]
                    .as_str()
                    .unwrap_or_default()
                    .contains(retired)
            );
        }
    }

    #[tokio::test]
    async fn supported_protocol_version_header_is_accepted() {
        let (state, _tmp) = test_state();
        let app = transport(&state);
        let (session, _) = initialize(&app).await;

        // Modern-style request: the 2026-07-28 revision requires per-request
        // protocol metadata alongside the header (SEP-1319/2567).
        let mut headers = auth_headers(TEST_TOKEN);
        headers.push(("mcp-session-id", session.id));
        headers.push(("mcp-protocol-version", "2026-07-28".into()));
        headers.push(("Mcp-Method", "tools/list".into()));
        headers.push(("Mcp-Name", "tools/list".into()));
        headers.push(("content-type", "application/json".into()));
        headers.push(("accept", "application/json, text/event-stream".into()));
        let response = send(
            app,
            "POST",
            headers,
            Some(
                json!({
                    "jsonrpc":"2.0","id":2,"method":"tools/list",
                    "params": {
                        "_meta": {
                            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                            "io.modelcontextprotocol/clientCapabilities": {}
                        }
                    }
                })
                .to_string(),
            ),
        )
        .await;
        if response.status() != StatusCode::OK {
            let status = response.status();
            let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            panic!("status {status}: {}", String::from_utf8_lossy(&bytes));
        }
        let message = response_message(response).await;
        assert!(message["result"]["tools"].is_array());
    }

    #[tokio::test]
    async fn golden_tools_list_shape_for_the_legacy_revision() {
        let (state, _tmp) = test_state();
        let body = tools_list_result(&state).await;
        let names: Vec<&str> = body["result"]["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .map(|tool| tool["name"].as_str().expect("tool name"))
            .collect();

        assert_eq!(
            names,
            vec![
                "get_model_setup_status",
                "accept_gemma_terms",
                "decline_gemma_terms",
                "list_vaults",
                "search_notes",
                "get_note",
                "get_note_links",
                "resolve_wikilink",
                "get_tree",
                "get_stats",
                "get_graph",
                "get_frontmatter",
                "list_note_attachments",
                "get_attachment",
                "get_attachment_import_config",
                "recently_modified",
                "batch",
            ]
        );
        assert!(
            !names
                .iter()
                .any(|name| name.contains("write") || name.contains("delete"))
        );
        for tool in body["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .skip(3)
            .take(5)
        {
            assert_eq!(tool["annotations"]["readOnlyHint"], true);
            assert_eq!(tool["annotations"]["destructiveHint"], false);
            assert_eq!(tool["annotations"]["idempotentHint"], true);
            assert_eq!(tool["annotations"]["openWorldHint"], false);
        }
        // Every advertised tool carries its typed outputSchema (#167).
        for tool in body["result"]["tools"].as_array().unwrap() {
            assert!(
                tool["outputSchema"].is_object(),
                "{} has no outputSchema",
                tool["name"]
            );
        }
    }

    #[tokio::test]
    async fn golden_tool_call_success_shape_for_the_legacy_revision() {
        let (state, _tmp) = test_state();
        let body = call_tool(&state, "get_note", json!({"slug":"home"})).await;
        assert_eq!(body["result"]["isError"], false);
        assert_eq!(body["result"]["structuredContent"]["note"]["slug"], "home");
        assert!(
            body["result"]["structuredContent"]["note"]["content"]
                .as_str()
                .expect("content")
                .contains("alpha token")
        );
    }

    #[tokio::test]
    async fn ping_answers_over_the_session() {
        let (state, _tmp) = test_state();
        let app = transport(&state);
        let (session, _) = initialize(&app).await;
        let message = response_message(
            rpc(
                &app,
                &session,
                json!({"jsonrpc":"2.0","id":7,"method":"ping"}),
            )
            .await,
        )
        .await;
        assert_eq!(message["result"], json!({}));
    }

    #[tokio::test]
    async fn notifications_without_ids_are_accepted_not_dispatched() {
        let (state, _tmp) = test_state();
        let app = transport(&state);
        let (session, _) = initialize(&app).await;
        let mut headers = auth_headers(TEST_TOKEN);
        headers.push(("mcp-session-id", session.id));
        headers.push(("content-type", "application/json".into()));
        headers.push(("accept", "application/json, text/event-stream".into()));
        let response = send(
            app,
            "POST",
            headers,
            Some(json!({"jsonrpc":"2.0","method":"notifications/roots/list_changed"}).to_string()),
        )
        .await;
        assert_eq!(response.status(), StatusCode::ACCEPTED);
    }

    #[tokio::test]
    async fn unknown_method_is_a_protocol_error() {
        let (state, _tmp) = test_state();
        let app = transport(&state);
        let (session, _) = initialize(&app).await;
        let raw = rpc(
            &app,
            &session,
            json!({"jsonrpc":"2.0","id":8,"method":"prompts/list"}),
        )
        .await;
        let message = response_message(raw).await;
        assert_eq!(message["error"]["code"], -32601);
    }

    // ---------------------------------------------------------------------------
    // Golden wire tests: the modern revision's request/response shapes
    // (issue #169) — no initialization handshake, per-request metadata.
    // ---------------------------------------------------------------------------

    /// The `_meta` object a well-behaved `2026-07-28` client attaches to every
    /// request (SEP-1319/2567): declared protocol version and client
    /// capabilities.
    fn modern_meta(protocol_version: &str, with_capabilities: bool) -> Value {
        let mut meta = json!({
            "io.modelcontextprotocol/protocolVersion": protocol_version,
        });
        if with_capabilities {
            meta["io.modelcontextprotocol/clientCapabilities"] = json!({});
        }
        meta
    }

    async fn modern_post(
        app: Router,
        method_header: &str,
        name_header: Option<&str>,
        header_version: &str,
        payload: Value,
    ) -> Response {
        let mut headers = auth_headers(TEST_TOKEN);
        headers.push(("mcp-protocol-version", header_version.to_string()));
        headers.push(("Mcp-Method", method_header.to_string()));
        if let Some(name) = name_header {
            headers.push(("Mcp-Name", name.to_string()));
        }
        headers.push(("content-type", "application/json".into()));
        headers.push(("accept", "application/json, text/event-stream".into()));
        send(app, "POST", headers, Some(payload.to_string())).await
    }

    #[tokio::test]
    async fn golden_discover_shape_for_the_modern_revision() {
        let (state, _tmp) = test_state();
        let app = transport(&state);
        let raw = modern_post(
            app,
            "server/discover",
            Some("server/discover"),
            "2026-07-28",
            json!({
                "jsonrpc":"2.0","id":1,"method":"server/discover",
                "params":{"_meta": modern_meta("2026-07-28", true)}
            }),
        )
        .await;
        let message = response_message(raw).await;
        let result = &message["result"];
        assert_eq!(result["resultType"], "complete");
        assert_eq!(
            result["supportedVersions"],
            json!(["2026-07-28", "2025-11-25"])
        );
        assert!(result["capabilities"]["tools"].is_object());
        assert!(
            result["instructions"]
                .as_str()
                .expect("instructions")
                .contains("Start with list_vaults")
        );
        assert_eq!(
            result["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
            "hatchdoor"
        );
        assert_eq!(result["ttlMs"], 300_000);
        assert_eq!(result["cacheScope"], "private");
        assert!(message.get("error").is_none());
    }

    #[tokio::test]
    async fn modern_tools_list_is_stateless_and_carries_cache_metadata() {
        let (state, _tmp) = test_state();
        let app = transport(&state);
        let raw = modern_post(
            app,
            "tools/list",
            None,
            "2026-07-28",
            json!({
                "jsonrpc":"2.0","id":2,"method":"tools/list",
                "params":{"_meta": modern_meta("2026-07-28", true)}
            }),
        )
        .await;
        let message = response_message(raw).await;
        assert!(message.get("error").is_none(), "{}", message);
        assert!(
            !message["result"]["tools"]
                .as_array()
                .expect("tools")
                .is_empty()
        );
        assert_eq!(message["result"]["ttlMs"], 300_000);
        assert_eq!(message["result"]["cacheScope"], "private");
    }

    #[tokio::test]
    async fn modern_tool_call_completes_without_initialization() {
        let (state, _tmp) = test_state();
        let vault_id = state.vaults.snapshot().vaults.keys().next().copied();
        let app = transport(&state);
        let raw = modern_post(
            app,
            "tools/call",
            Some("get_note"),
            "2026-07-28",
            json!({
                "jsonrpc":"2.0","id":3,"method":"tools/call",
                "params":{
                    "_meta": modern_meta("2026-07-28", true),
                    "name":"get_note",
                    "arguments":{"slug":"home","vault_id": vault_id}
                }
            }),
        )
        .await;
        let message = response_message(raw).await;
        assert_eq!(message["result"]["isError"], false);
        assert_eq!(
            message["result"]["structuredContent"]["note"]["slug"],
            "home"
        );
    }

    /// The `isError: true` leg of the error-semantics matrix, golden-tested at
    /// the modern revision too: a valid call with an actionable failure stays
    /// a success-status tool result carrying the structured Vault error.
    #[tokio::test]
    async fn modern_actionable_tool_failure_returns_is_error_true_result() {
        let (state, _tmp) = test_state();
        let vault_id = state.vaults.snapshot().vaults.keys().next().copied();
        let app = transport(&state);
        let raw = raw_modern_post(
            app,
            "tools/call",
            Some("get_note"),
            json!({
                "jsonrpc":"2.0","id":16,"method":"tools/call",
                "params":{
                    "_meta": modern_meta("2026-07-28", true),
                    "name":"get_note",
                    "arguments":{"slug":"no-such-note","vault_id": vault_id}
                }
            })
            .to_string(),
        )
        .await;
        assert_eq!(raw.status(), StatusCode::OK);
        let message = response_message(raw).await;
        assert!(message.get("error").is_none(), "{}", message);
        assert_eq!(message["result"]["isError"], true);
        assert_eq!(
            message["result"]["structuredContent"]["code"],
            "note_not_found"
        );
    }

    #[tokio::test]
    async fn modern_request_without_protocol_version_header_is_rejected() {
        let (state, _tmp) = test_state();
        let app = transport(&state);
        let mut headers = auth_headers(TEST_TOKEN);
        headers.push(("Mcp-Method", "tools/list".into()));
        headers.push(("content-type", "application/json".into()));
        headers.push(("accept", "application/json, text/event-stream".into()));
        let response = send(
            app,
            "POST",
            headers,
            Some(
                json!({
                    "jsonrpc":"2.0","id":4,"method":"tools/list",
                    "params":{"_meta": modern_meta("2026-07-28", true)}
                })
                .to_string(),
            ),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"]["code"], -32020);
    }

    #[tokio::test]
    async fn modern_request_without_meta_protocol_version_is_rejected() {
        let (state, _tmp) = test_state();
        let app = transport(&state);
        let mut meta = modern_meta("2026-07-28", true);
        meta.as_object_mut()
            .unwrap()
            .remove("io.modelcontextprotocol/protocolVersion");
        let raw = modern_post(
            app,
            "tools/list",
            None,
            "2026-07-28",
            json!({"jsonrpc":"2.0","id":5,"method":"tools/list","params":{"_meta": meta}}),
        )
        .await;
        assert_eq!(raw.status(), StatusCode::BAD_REQUEST);
        let bytes = to_bytes(raw.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"]["code"], -32602);
    }

    #[tokio::test]
    async fn modern_request_with_mismatched_header_and_meta_versions_is_rejected() {
        let (state, _tmp) = test_state();
        let app = transport(&state);
        let raw = modern_post(
            app,
            "tools/list",
            None,
            "2025-11-25",
            json!({
                "jsonrpc":"2.0","id":6,"method":"tools/list",
                "params":{"_meta": modern_meta("2026-07-28", true)}
            }),
        )
        .await;
        assert_eq!(raw.status(), StatusCode::BAD_REQUEST);
        let bytes = to_bytes(raw.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"]["code"], -32020);
    }

    #[tokio::test]
    async fn modern_request_missing_client_capabilities_is_rejected() {
        let (state, _tmp) = test_state();
        let app = transport(&state);
        let raw = modern_post(
            app,
            "tools/list",
            None,
            "2026-07-28",
            json!({
                "jsonrpc":"2.0","id":7,"method":"tools/list",
                "params":{"_meta": modern_meta("2026-07-28", false)}
            }),
        )
        .await;
        let bytes = to_bytes(raw.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"]["code"], -32602);
        assert!(
            body["error"]["message"]
                .as_str()
                .unwrap()
                .contains("clientCapabilities")
        );
    }

    #[tokio::test]
    async fn modern_request_method_header_must_match_the_body_method() {
        let (state, _tmp) = test_state();
        let app = transport(&state);
        let raw = modern_post(
            app,
            "prompts/list",
            None,
            "2026-07-28",
            json!({
                "jsonrpc":"2.0","id":8,"method":"tools/list",
                "params":{"_meta": modern_meta("2026-07-28", true)}
            }),
        )
        .await;
        assert_eq!(raw.status(), StatusCode::BAD_REQUEST);
        let bytes = to_bytes(raw.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"]["code"], -32020);
        assert!(
            body["error"]["message"]
                .as_str()
                .unwrap()
                .contains("Mcp-Method")
        );
    }

    #[tokio::test]
    async fn modern_request_missing_the_method_header_is_rejected() {
        let (state, _tmp) = test_state();
        let app = transport(&state);
        let mut headers = auth_headers(TEST_TOKEN);
        headers.push(("mcp-protocol-version", "2026-07-28".to_string()));
        headers.push(("content-type", "application/json".into()));
        headers.push(("accept", "application/json, text/event-stream".into()));
        let response = send(
            app,
            "POST",
            headers,
            Some(
                json!({
                    "jsonrpc":"2.0","id":10,"method":"tools/list",
                    "params":{"_meta": modern_meta("2026-07-28", true)}
                })
                .to_string(),
            ),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"]["code"], -32020);
        assert!(
            body["error"]["message"]
                .as_str()
                .unwrap()
                .contains("Mcp-Method")
        );
    }

    #[tokio::test]
    async fn modern_tool_call_missing_the_name_header_is_rejected() {
        let (state, _tmp) = test_state();
        let vault_id = state.vaults.snapshot().vaults.keys().next().copied();
        let app = transport(&state);
        let raw = modern_post(
            app,
            "tools/call",
            None,
            "2026-07-28",
            json!({
                "jsonrpc":"2.0","id":11,"method":"tools/call",
                "params":{
                    "_meta": modern_meta("2026-07-28", true),
                    "name":"get_note",
                    "arguments":{"slug":"home","vault_id": vault_id}
                }
            }),
        )
        .await;
        assert_eq!(raw.status(), StatusCode::BAD_REQUEST);
        let bytes = to_bytes(raw.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"]["code"], -32020);
        assert!(
            body["error"]["message"]
                .as_str()
                .unwrap()
                .contains("Mcp-Name")
        );
    }

    #[tokio::test]
    async fn modern_tool_call_name_header_must_match_the_requested_tool() {
        let (state, _tmp) = test_state();
        let vault_id = state.vaults.snapshot().vaults.keys().next().copied();
        let app = transport(&state);
        let raw = modern_post(
            app,
            "tools/call",
            Some("search_notes"),
            "2026-07-28",
            json!({
                "jsonrpc":"2.0","id":9,"method":"tools/call",
                "params":{
                    "_meta": modern_meta("2026-07-28", true),
                    "name":"get_note",
                    "arguments":{"slug":"home","vault_id": vault_id}
                }
            }),
        )
        .await;
        assert_eq!(raw.status(), StatusCode::BAD_REQUEST);
        let bytes = to_bytes(raw.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"]["code"], -32020);
        assert!(
            body["error"]["message"]
                .as_str()
                .unwrap()
                .contains("Mcp-Name")
        );
    }

    // ---------------------------------------------------------------------------
    // Security gate: ordering and responses unchanged across the swap.
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn disabled_mcp_returns_not_found() {
        let (state, _tmp) = test_state();
        let state = state;
        state
            .runtime_config
            .save([("HATCHDOOR_MCP_ENABLED".to_string(), "false".to_string())])
            .expect("disable MCP");
        let app = transport(&state);
        let response = send(
            app,
            "POST",
            [("content-type", "application/json".into())].to_vec(),
            Some(json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}).to_string()),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn missing_bearer_token_is_rejected_before_dispatch() {
        let (state, _tmp) = test_state();
        let app = transport(&state);
        let response = send(
            app,
            "POST",
            [("content-type", "application/json".into())].to_vec(),
            Some(json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}).to_string()),
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"]["code"], -32001);
    }

    #[tokio::test]
    async fn wrong_bearer_token_is_rejected() {
        let (state, _tmp) = test_state();
        let app = transport(&state);
        let response = send(
            app,
            "POST",
            [
                ("authorization", "Bearer secret".to_string()),
                ("content-type", "application/json".into()),
            ]
            .to_vec(),
            Some(json!({"jsonrpc":"2.0","id":10,"method":"tools/list"}).to_string()),
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn disallowed_origin_is_rejected() {
        let (state, _tmp) = test_state();
        let app = transport(&state);
        let response = send(
            app,
            "POST",
            [
                ("origin", "https://evil.example".into()),
                ("content-type", "application/json".into()),
            ]
            .to_vec(),
            Some(json!({"jsonrpc":"2.0","id":12,"method":"tools/list"}).to_string()),
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn read_only_mcp_rejects_a_body_past_the_ordinary_request_limit() {
        let (state, _tmp) = test_state();
        let app = transport(&state);
        let config = McpConfig::from_snapshot(&state.runtime_snapshot()).expect("config");
        let oversized = " ".repeat(config.request_body_limit() + 1);
        let response = send(
            app,
            "POST",
            [
                ("authorization", format!("Bearer {TEST_TOKEN}")),
                ("content-type", "application/json".into()),
            ]
            .to_vec(),
            Some(oversized),
        )
        .await;
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"]["code"], -32600);
    }

    // ---------------------------------------------------------------------------
    // Layered resource protection (#171).
    // ---------------------------------------------------------------------------

    /// A cheap modern-stateless `tools/call` that still exercises the full
    /// quota path (it is a real tool call, just one that touches no vault).
    async fn modern_cheap_tool_call(app: Router, id: u64) -> Response {
        modern_post(
            app,
            "tools/call",
            Some("get_model_setup_status"),
            "2026-07-28",
            json!({
                "jsonrpc":"2.0","id":id,"method":"tools/call",
                "params":{
                    "_meta": modern_meta("2026-07-28", true),
                    "name":"get_model_setup_status",
                    "arguments":{}
                }
            }),
        )
        .await
    }

    #[test]
    fn classification_resists_json_string_escapes() {
        // An escaped method name decodes to a real tool call, so it must be
        // counted even though the raw bytes contain no marker.
        let escaped =
            br#"{"jsonrpc":"2.0","id":1,"method":"\u0074ools/call","params":{"name":"get_note"}}"#;
        assert_eq!(
            classify_post_body(escaped),
            Some(RequestClass::ToolCall),
            "an escaped method name must not bypass the quota"
        );
        assert_eq!(
            classify_post_body(br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#),
            None,
            "list handling stays exempt"
        );
        assert_eq!(classify_post_body(b"not json at all"), None);
    }

    #[tokio::test]
    async fn tool_calls_past_the_per_minute_quota_get_429_with_retry_after() {
        let (state, _tmp) = test_state();
        let app = transport(&state);
        for id in 1..=(super::super::limits::TOOL_CALLS_PER_MINUTE as u64) {
            let response = modern_cheap_tool_call(app.clone(), id).await;
            assert_eq!(response.status(), StatusCode::OK, "call {id} admitted");
        }
        let over_limit = modern_cheap_tool_call(app, TOOL_CALLS_PER_MINUTE as u64 + 1).await;
        assert_eq!(over_limit.status(), StatusCode::TOO_MANY_REQUESTS);
        let retry_after = over_limit
            .headers()
            .get(axum::http::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .expect("429 carries Retry-After");
        assert!(retry_after.parse::<u64>().is_ok());
    }

    #[tokio::test]
    async fn list_and_discovery_stay_outside_the_exhausted_tool_quota() {
        let (state, _tmp) = test_state();
        let app = transport(&state);
        for id in 1..=(TOOL_CALLS_PER_MINUTE as u64 + 2) {
            if modern_cheap_tool_call(app.clone(), id).await.status()
                == StatusCode::TOO_MANY_REQUESTS
            {
                break;
            }
        }
        let raw = modern_post(
            app.clone(),
            "tools/list",
            None,
            "2026-07-28",
            json!({"jsonrpc":"2.0","id":50,"method":"tools/list","params":{"_meta": modern_meta("2026-07-28", true)}}),
        )
        .await;
        assert_eq!(raw.status(), StatusCode::OK, "list handling is exempt");
        let raw = modern_post(
            app.clone(),
            "server/discover",
            Some("server/discover"),
            "2026-07-28",
            json!({"jsonrpc":"2.0","id":51,"method":"server/discover","params":{"_meta": modern_meta("2026-07-28", true)}}),
        )
        .await;
        assert_eq!(raw.status(), StatusCode::OK, "discovery is exempt");
    }

    #[tokio::test]
    async fn disabling_rate_limits_admits_calls_past_the_quota() {
        let (state, _tmp) = test_state();
        state
            .runtime_config
            .save([(
                "HATCHDOOR_MCP_RATE_LIMITS_ENABLED".to_string(),
                "false".to_string(),
            )])
            .expect("disable rate limits");
        let app = transport(&state);
        for id in 1..=(TOOL_CALLS_PER_MINUTE as u64 + 5) {
            let response = modern_cheap_tool_call(app.clone(), id).await;
            assert_eq!(
                response.status(),
                StatusCode::OK,
                "call {id} admitted while limits are disabled"
            );
        }
    }

    // ---------------------------------------------------------------------------
    // Tool behaviour through the swapped boundary.
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn write_mode_exposes_mutation_tools() {
        let (state, _tmp) = write_state();
        let body = tools_list_result(&state).await;
        let names: Vec<&str> = body["result"]["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .map(|tool| tool["name"].as_str().expect("tool name"))
            .collect();

        for expected in [
            "create_note",
            "update_note",
            "edit_note",
            "replace_section",
            "move_rename_note",
            "archive_note",
            "delete_note",
            "import_attachment",
            "move_attachment",
            "rename_attachment",
            "delete_attachment",
            "list_note_attachments",
            "create_vault",
            "edit_vault",
            "disable_vault",
            "sync_vault",
            "refresh_vault",
        ] {
            assert!(
                names.contains(&expected),
                "{expected} advertised in write mode"
            );
        }
    }

    #[tokio::test]
    async fn vault_management_is_hidden_and_rejected_without_mcp_write_permission() {
        let (state, _tmp) = test_state();
        let body = tools_list_result(&state).await;
        assert!(
            !body["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == "disable_vault")
        );

        let rejected = call_tool(
            &state,
            "disable_vault",
            json!({"expected_registry_revision": 0}),
        )
        .await;
        assert_eq!(rejected["error"]["code"], -32602);
        assert!(
            rejected["error"]["message"]
                .as_str()
                .unwrap()
                .contains("write tools are disabled")
        );
    }

    /// `refresh_vault` (#228) is the only MCP path to a Vault's next Index
    /// turn, which is what republishes the snapshot every collection read
    /// projects from. It admits the turn and returns; a second request while
    /// one is pending joins it rather than piling a second turn onto the
    /// Vault.
    #[tokio::test]
    async fn refresh_vault_admits_one_index_turn_and_coalesces_the_next() {
        let (state, _tmp) = write_state();
        let vault_id = vault_id_of(&state);

        let queued = call_tool(&state, "refresh_vault", json!({})).await;
        assert_eq!(
            queued["result"]["structuredContent"]["schedule"], "queued",
            "{queued:#}"
        );
        assert_eq!(
            queued["result"]["structuredContent"]["vault_id"],
            json!(vault_id),
            "{queued:#}"
        );

        let coalesced = call_tool(&state, "refresh_vault", json!({})).await;
        assert_eq!(
            coalesced["result"]["structuredContent"]["schedule"], "coalesced",
            "{coalesced:#}"
        );
    }

    /// The annotations and the one-property schema an agent chooses on. They
    /// match `sync_vault`/`retry_vault`: not read-only, not destructive,
    /// idempotent, not open-world.
    #[tokio::test]
    async fn refresh_vault_is_advertised_as_an_idempotent_non_destructive_vault_control() {
        let (state, _tmp) = write_state();
        let tool = tool_named(&tools_list_result(&state).await, "refresh_vault").clone();

        assert_eq!(tool["annotations"]["readOnlyHint"], false, "{tool:#}");
        assert_eq!(tool["annotations"]["destructiveHint"], false, "{tool:#}");
        assert_eq!(tool["annotations"]["idempotentHint"], true, "{tool:#}");
        assert_eq!(tool["annotations"]["openWorldHint"], false, "{tool:#}");
        assert_eq!(tool["inputSchema"]["required"], json!(["vault_id"]));
        assert_eq!(tool["inputSchema"]["additionalProperties"], false);
        assert_eq!(
            tool["inputSchema"]["properties"]
                .as_object()
                .expect("properties")
                .keys()
                .collect::<Vec<_>>(),
            vec!["vault_id"],
            "refresh_vault takes vault_id and nothing else"
        );
    }

    #[tokio::test]
    async fn refresh_vault_is_hidden_and_rejected_without_mcp_write_permission() {
        let (state, _tmp) = test_state();
        let body = tools_list_result(&state).await;
        assert!(
            !body["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == "refresh_vault")
        );

        let rejected = call_tool(&state, "refresh_vault", json!({})).await;
        assert_eq!(rejected["error"]["code"], -32602);
        assert!(
            rejected["error"]["message"]
                .as_str()
                .unwrap()
                .contains("write tools are disabled")
        );
    }

    /// Every way of naming the wrong Vault, refused exactly as the other
    /// single-Vault management tools refuse it: `all` and a malformed ID are
    /// the core's structured `invalid_vault_id`, an unregistered ID is
    /// `vault_not_found`, and an unexpected property is a protocol-level
    /// rejection from `deny_unknown_fields`.
    #[tokio::test]
    async fn refresh_vault_rejects_all_a_malformed_id_an_unknown_id_and_extra_properties() {
        let (state, _tmp) = write_state();

        for raw in ["all", "not-a-uuid"] {
            let body = call_tool_unscoped(&state, "refresh_vault", json!({"vault_id": raw})).await;
            assert_eq!(body["result"]["isError"], true, "{raw}: {body:#}");
            assert_eq!(
                body["result"]["structuredContent"]["code"], "invalid_vault_id",
                "{raw}: {body:#}"
            );
        }

        let unknown = call_tool_unscoped(
            &state,
            "refresh_vault",
            json!({"vault_id": "018f47a0-7768-4d0c-8da3-5aa28d1c31c7"}),
        )
        .await;
        assert_eq!(unknown["result"]["isError"], true, "{unknown:#}");
        assert_eq!(
            unknown["result"]["structuredContent"]["code"], "vault_not_found",
            "{unknown:#}"
        );

        let extra = call_tool(
            &state,
            "refresh_vault",
            json!({"expected_registry_revision": 0}),
        )
        .await;
        assert_eq!(extra["error"]["code"], -32602, "{extra:#}");
    }

    /// A Vault with no currently usable local Markdown has nothing to scan, so
    /// the core refuses with `capability_unavailable` marked retryable — the
    /// Vault may become browsable again — rather than a generic failure.
    #[tokio::test]
    async fn refresh_vault_on_a_vault_without_usable_local_markdown_is_retryable() {
        let (state, _tmp) = unusable_local_content_write_state();

        let body = call_tool(&state, "refresh_vault", json!({})).await;
        assert_eq!(body["result"]["isError"], true, "{body:#}");
        assert_eq!(
            body["result"]["structuredContent"]["code"], "capability_unavailable",
            "{body:#}"
        );
        assert_eq!(
            body["result"]["structuredContent"]["retryable"], true,
            "{body:#}"
        );
    }

    /// The point of the tool (#228): the freshness flags a collection read
    /// publishes are only actionable if an MCP client can act on them. A stale
    /// snapshot reads `partial: true`; one `refresh_vault` and the Index turn
    /// it admits clears it.
    #[tokio::test]
    async fn refresh_vault_clears_a_partial_collection_read_once_its_turn_completes() {
        let (state, mut worker, _tmp) = write_state_with_worker();
        let vault_id = vault_id_of(&state);
        let vault_root = registered_vault_path(&state);

        state
            .startup_sqlite
            .mark_vault_snapshot_stale(vault_id)
            .expect("stale the published snapshot");

        let before = call_tool(&state, "get_tree", json!({})).await;
        assert_eq!(
            before["result"]["structuredContent"]["partial"], true,
            "{before:#}"
        );
        assert_eq!(
            before["result"]["structuredContent"]["participants"][0]["state"], "stale",
            "{before:#}"
        );

        let queued = call_tool(&state, "refresh_vault", json!({})).await;
        assert_eq!(
            queued["result"]["structuredContent"]["schedule"], "queued",
            "{queued:#}"
        );

        // Stands in for the runtime worker's Index turn: the authoritative
        // Markdown scan and atomic snapshot publication that `refresh_vault`
        // only admits, never performs itself.
        let cache = Arc::clone(&state.startup_sqlite);
        let embedder = Arc::clone(&state.embedder);
        let outcome = worker
            .run_next(|request| async move {
                assert_eq!(request.kind(), crate::vault_work::VaultWorkKind::Index);
                let index = crate::vault::VaultIndex::build(&vault_root).expect("rebuild index");
                cache
                    .replace_vault_snapshot(request.vault_id(), &index, embedder.as_ref())
                    .expect("republish snapshot");
                Ok::<(), crate::vault_work::VaultWorkError>(())
            })
            .await
            .expect("the admitted turn ran");
        assert_eq!(outcome.request.vault_id(), vault_id);

        let after = call_tool(&state, "get_tree", json!({})).await;
        assert_eq!(
            after["result"]["structuredContent"]["partial"], false,
            "{after:#}"
        );
        assert_eq!(
            after["result"]["structuredContent"]["participants"][0]["state"], "fresh",
            "{after:#}"
        );
    }

    #[tokio::test]
    async fn unknown_argument_fields_are_rejected() {
        let (state, _tmp) = test_state();
        let body = call_tool(&state, "get_note", json!({"slug":"home", "path":"Home.md"})).await;
        assert_eq!(body["error"]["code"], -32602);
    }

    #[tokio::test]
    async fn scope_less_collection_read_is_rejected_at_the_mcp_transport() {
        let (state, _tmp) = test_state();
        let app = transport(&state);
        let (session, _) = initialize(&app).await;
        // No scope argument injected: exactly what a scope-less caller sends.
        let message = response_message(
            rpc(&app, &session, json!({"jsonrpc":"2.0","id":41,"method":"tools/call","params":{"name":"get_tree","arguments":{}}})).await,
        )
        .await;
        assert_eq!(message["error"]["code"], -32602);
    }

    #[tokio::test]
    async fn collection_tools_declare_scope_even_without_layers() {
        let (state, _tmp) = layered_test_state();
        let body = tools_list_result(&state).await;
        for tool_name in ["search_notes", "get_tree", "get_stats", "get_graph"] {
            let tool = tool_named(&body, tool_name);
            assert!(
                tool["inputSchema"]["required"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("scope")),
                "{tool_name}"
            );
        }
        let search = tool_named(&body, "search_notes");
        assert_eq!(
            search["inputSchema"]["properties"]["layers"]["items"]["type"],
            "string"
        );
        assert_eq!(search["inputSchema"]["required"], json!(["scope", "query"]));
    }

    #[tokio::test]
    async fn first_run_mcp_advertises_setup_and_vault_tools_but_blocks_vault_access() {
        let (state, _tmp) = test_state();
        let state = state;
        state.startup.set_terms_required();
        let body = tools_list_result(&state).await;
        let tools = body["result"]["tools"].as_array().expect("tools");
        for name in [
            "get_model_setup_status",
            "accept_gemma_terms",
            "decline_gemma_terms",
            "search_notes",
        ] {
            assert!(tools.iter().any(|tool| tool["name"] == name), "{name}");
        }

        let blocked = call_tool(&state, "search_notes", json!({"query":"alpha"})).await;
        assert_eq!(blocked["result"]["isError"], true);
        assert_eq!(
            blocked["result"]["content"][0]["text"],
            "Hatchdoor is still being set up. Use get_model_setup_status, accept_gemma_terms, or decline_gemma_terms first."
        );
    }

    /// `refresh_vault` is deliberately outside the collection-management
    /// exemption that keeps discovery and Vault control reachable while model
    /// setup is pending (#228). That exemption is for tools which stay
    /// meaningful at zero enabled Vaults or on a registry needing recovery;
    /// the Index turn `refresh_vault` asks for cannot run without a configured
    /// search model, so a caller gets the setup signal instead of a queued
    /// turn that would go nowhere.
    #[tokio::test]
    async fn refresh_vault_is_not_exempt_from_the_pending_model_setup_gate() {
        let (state, _tmp) = write_state();
        state.startup.set_terms_required();

        let blocked = call_tool(&state, "refresh_vault", json!({})).await;
        assert_eq!(blocked["result"]["isError"], true, "{blocked:#}");
        assert_eq!(
            blocked["result"]["content"][0]["text"],
            "Hatchdoor is still being set up. Use get_model_setup_status, accept_gemma_terms, or decline_gemma_terms first."
        );

        let listed = call_tool(&state, "list_vaults", json!({})).await;
        assert!(
            listed["result"]["structuredContent"]["vaults"].is_array(),
            "collection discovery stays reachable during pending setup: {listed:#}"
        );

        // `sync_vault` is the neighbouring Vault control that *is* exempt, so
        // the difference is pinned rather than assumed. It still fails here —
        // the test Vault is Local, with no remote to poll — but it fails with
        // the core's own refusal, which is only reachable past the setup gate.
        let synced = call_tool(&state, "sync_vault", json!({})).await;
        assert_eq!(
            synced["result"]["structuredContent"]["code"], "capability_unavailable",
            "sync_vault reaches the core during pending setup: {synced:#}"
        );
    }

    /// #191. A post-write Index turn reports its progress through the *same*
    /// tracker that carries the first-run setup lifecycle, so the collection
    /// leaves `Ready` for `Indexing` several times a minute in a write-heavy
    /// session. Every read and write tool used to answer that window with the
    /// first-run setup error, which is both wrong and unactionable: setup
    /// completed long ago, and the terms tool it names can change nothing.
    #[tokio::test]
    async fn a_reindex_is_not_reported_as_incomplete_model_setup() {
        let (state, _tmp) = write_state();
        state
            .startup
            .set_indexing(crate::startup::IndexingProgressSnapshot::default());

        for (tool, arguments) in [
            ("get_note", json!({"slug":"home"})),
            ("search_notes", json!({"query":"alpha"})),
            ("resolve_wikilink", json!({"target":"Plan"})),
            ("get_tree", json!({})),
        ] {
            let body = call_tool(&state, tool, arguments).await;
            assert_eq!(
                body["result"]["isError"], false,
                "{tool} during a reindex: {}",
                body["result"]["content"][0]["text"]
            );
        }

        // A write too: the window this opens is a *consequence* of writing, so
        // the next write in a run of edits is the call most likely to land in
        // it.
        let read = call_tool(&state, "get_note", json!({"slug":"home"})).await;
        let hash = read["result"]["structuredContent"]["note"]["content_hash"]
            .as_str()
            .expect("content hash");
        let edited = call_tool(
            &state,
            "edit_note",
            json!({
                "slug": "home",
                "old_string": "alpha token",
                "new_string": "beta token",
                "expected_content_hash": hash,
            }),
        )
        .await;
        assert_eq!(
            edited["result"]["isError"], false,
            "edit_note during a reindex: {}",
            edited["result"]["content"][0]["text"]
        );
    }

    /// The other half of #191: following the error's own advice produced a
    /// second, differently wrong answer ("a setup is already active"), so a
    /// client had to fail twice to discover the Vault was merely reindexing.
    /// Once setup is complete the terms tools stay unreachable, reindex or not,
    /// which is the invariant `READ_OPS`' doc comment already claimed.
    #[tokio::test]
    async fn a_reindex_does_not_reopen_the_model_terms_tools() {
        let (state, _tmp) = test_state();
        state
            .startup
            .set_indexing(crate::startup::IndexingProgressSnapshot::default());

        let declined = call_tool_unscoped(&state, "decline_gemma_terms", json!({})).await;
        assert_eq!(declined["result"]["isError"], true);
        assert_eq!(
            declined["result"]["content"][0]["text"],
            "A search model is already set up. Changing models after setup is not supported."
        );
    }

    #[tokio::test]
    async fn readiness_gate_exempts_vault_collection_discovery() {
        let (state, _tmp) = test_state();
        let state = state;
        state.startup.set_terms_required();

        let listed = call_tool(&state, "list_vaults", json!({})).await;
        assert_eq!(listed["result"]["isError"], false);
        assert_eq!(
            listed["result"]["structuredContent"]["vaults"]
                .as_array()
                .expect("vaults array")
                .len(),
            1
        );

        let blocked = call_tool(&state, "search_notes", json!({"query":"alpha"})).await;
        assert_eq!(blocked["result"]["isError"], true);
    }

    #[tokio::test]
    async fn readiness_gate_exempts_create_vault_at_zero_vaults() {
        let (state, tmp) = empty_test_state();
        let state = state;
        state.startup.set_terms_required();
        let vault_path = tmp.path().join("new-vault");
        std::fs::create_dir_all(&vault_path).expect("vault dir");

        let mut state = state;
        state.runtime_config = mcp_runtime_config(true);
        let body = call_tool(
            &state,
            "create_vault",
            json!({
                "expected_registry_revision": 0,
                "name": "First Vault",
                "source": {"type":"local","path": vault_path},
            }),
        )
        .await;
        assert_eq!(body["result"]["isError"], false);
        assert!(body["result"]["structuredContent"]["vault"]["vault_id"].is_string());
    }

    #[tokio::test]
    async fn model_setup_status_explains_the_nomic_fallback() {
        let (state, _tmp) = test_state();
        let state = state;
        state.startup.set_terms_required();
        let body = call_tool(&state, "get_model_setup_status", json!({})).await;
        assert_eq!(
            body["result"]["structuredContent"]["fallback"]["notice"],
            "Nomic is the fallback if you decline Gemma. It supports English only and still provides solid search, but Gemma performed better in Hatchdoor's tests, including English searches. Nomic uses about 1.3 GB of RAM while indexing; Gemma uses about 0.5 GB."
        );
    }

    #[tokio::test]
    async fn exact_reads_require_vault_qualified_slug_identity() {
        let (state, _tmp) = layered_test_state();
        let body = call_tool(&state, "get_note", json!({"slug": "clip"})).await;
        let note = &body["result"]["structuredContent"]["note"];
        assert_eq!(note["slug"], "clip");
        assert_eq!(note["layer"], "sources");

        let page = call_tool(&state, "get_note", json!({"slug": "page"})).await;
        assert_eq!(page["result"]["structuredContent"]["layer"], Value::Null);
    }

    #[tokio::test]
    async fn exact_reads_reject_legacy_path_addressing() {
        let (state, _tmp) = layered_test_state();
        let both = call_tool(
            &state,
            "get_note",
            json!({"slug": "page", "path": "wiki/Page.md"}),
        )
        .await;
        assert_eq!(both["error"]["code"], -32602);

        let neither = call_tool(&state, "get_note", json!({})).await;
        assert_eq!(neither["error"]["code"], -32602);
    }

    #[tokio::test]
    async fn search_returns_shared_collection_envelope() {
        let (state, _tmp) = layered_test_state();
        let search = call_tool(&state, "search_notes", json!({"query": "melatonin"})).await;
        let content = &search["result"]["structuredContent"];
        assert!(content.get("scope").is_some());
        assert!(content.get("collection_revision").is_some());
        let first = &content["data"]["results"][0];
        assert!(first.get("layer").is_some());
    }

    #[tokio::test]
    async fn recently_modified_returns_shared_collection_envelope() {
        let (state, _tmp) = layered_test_state();
        let default = call_tool(&state, "recently_modified", json!({})).await;
        let content = &default["result"]["structuredContent"];
        assert!(content["data"].is_array());
        assert!(content["participants"].is_array());
    }

    #[tokio::test]
    async fn retired_scope_less_query_notes_is_unreachable() {
        let (state, _tmp) = layered_test_state();
        let body = call_tool(&state, "query_notes", json!({})).await;
        assert_eq!(body["error"]["code"], -32602);
    }

    #[tokio::test]
    async fn search_rejects_legacy_metadata_filters() {
        let (state, _tmp) = layered_test_state();
        let body = call_tool(
            &state,
            "search_notes",
            json!({"query": "melatonin", "filters": {"tags": ["topic/x"]}}),
        )
        .await;
        assert_eq!(body["error"]["code"], -32602);
    }

    /// The refusal each read tool gives a malformed `vault_id`. The two groups
    /// answer differently and always have: a tool that used to proxy an
    /// `/api/v1` route reports that route's structured `invalid_vault_id`
    /// object, which an agent branches on, while the attachment and
    /// frontmatter tools resolved their Vault before any route shaped an error
    /// and refuse at the protocol level. #188 moved both off their old paths,
    /// so this pins the shapes that used to be produced by code that no longer
    /// exists.
    #[tokio::test]
    async fn a_malformed_vault_id_keeps_each_read_tools_own_refusal() {
        let (state, _tmp) = test_state();

        for (name, arguments) in [
            (
                "get_note",
                json!({"vault_id": "not-a-uuid", "slug": "home"}),
            ),
            (
                "get_note_links",
                json!({"vault_id": "not-a-uuid", "slug": "home"}),
            ),
            (
                "resolve_wikilink",
                json!({"vault_id": "not-a-uuid", "target": "Home"}),
            ),
        ] {
            let body = call_tool_unscoped(&state, name, arguments).await;
            assert_eq!(body["result"]["isError"], true, "{name}: {body:#}");
            assert_eq!(
                body["result"]["structuredContent"]["code"], "invalid_vault_id",
                "{name}: {body:#}"
            );
        }

        for (name, arguments) in [
            (
                "get_frontmatter",
                json!({"vault_id": "not-a-uuid", "slug": "home"}),
            ),
            (
                "list_note_attachments",
                json!({"vault_id": "not-a-uuid", "slug": "home"}),
            ),
            (
                "get_attachment",
                json!({"vault_id": "not-a-uuid", "relative_path": "a.png"}),
            ),
            (
                "get_attachment_import_config",
                json!({"vault_id": "not-a-uuid"}),
            ),
        ] {
            let body = call_tool_unscoped(&state, name, arguments).await;
            assert_eq!(body["error"]["code"], -32602, "{name}: {body:#}");
        }

        // A malformed `scope` is the core's own structured refusal, on every
        // collection read.
        for name in ["get_tree", "get_stats", "get_graph", "recently_modified"] {
            let body = call_tool_unscoped(&state, name, json!({"scope": "not-a-scope"})).await;
            assert_eq!(
                body["result"]["structuredContent"]["code"], "invalid_scope",
                "{name}: {body:#}"
            );
        }
    }

    /// `get_attachment` names the attachment back the way the caller asked for
    /// it, not by the canonicalised path resolution produces internally, so the
    /// `relative_path` it echoes and the `download_url` it builds agree — and
    /// that URL escapes each segment, since a Vault names attachments with
    /// spaces and non-ASCII freely.
    #[tokio::test]
    async fn get_attachment_echoes_the_relative_path_it_was_asked_with() {
        let (state, _tmp) = test_state();
        let vault_path = registered_vault_path(&state);
        std::fs::create_dir_all(vault_path.join("Media")).expect("media dir");
        std::fs::write(vault_path.join("Media/a shot.png"), b"png").expect("asset");

        let body = call_tool(
            &state,
            "get_attachment",
            json!({"relative_path": "Media/a shot.png"}),
        )
        .await;
        let result = &body["result"]["structuredContent"];
        assert_eq!(result["relative_path"], "Media/a shot.png");
        assert!(
            result["content"]["download_url"]
                .as_str()
                .expect("download_url")
                .ends_with("/assets/Media/a%20shot.png"),
            "{result:#}"
        );

        // The same file reached by a path that resolves to it: the echo stays
        // the caller's spelling rather than the resolved one.
        let body = call_tool(
            &state,
            "get_attachment",
            json!({"relative_path": "./Media/a shot.png"}),
        )
        .await;
        assert_eq!(
            body["result"]["structuredContent"]["relative_path"],
            "./Media/a shot.png"
        );
    }

    #[tokio::test]
    async fn missing_note_is_a_tool_error_not_a_protocol_error() {
        let (state, tmp0) = test_state();
        let mut state = state;
        state.runtime_config = mcp_runtime_config(true);
        drop(tmp0);
        let missing = call_tool(&state, "get_note", json!({"slug":"missing"})).await;
        assert_eq!(missing["result"]["isError"], true);
        assert!(missing.get("error").is_none());

        let edit = call_tool(
            &state,
            "edit_note",
            json!({
                "slug":"does-not-exist",
                "old_string":"a",
                "new_string":"b",
                "expected_content_hash":"deadbeef"
            }),
        )
        .await;
        assert_eq!(edit["result"]["isError"], true);
        assert!(edit.get("error").is_none());
    }

    async fn import_attachment_call(
        state: &AppState,
        content: &str,
        target: &str,
        overwrite: bool,
    ) -> Value {
        call_tool(
            state,
            "import_attachment",
            json!({"content": content, "target_relative_path": target, "overwrite": overwrite}),
        )
        .await
    }

    #[tokio::test]
    async fn attachment_import_config_reports_both_methods_for_the_named_vault() {
        let (state, _tmp) = write_state();
        let vault_id = state
            .vaults
            .snapshot()
            .vaults
            .keys()
            .next()
            .copied()
            .expect("registered test Vault");

        let body = call_tool(&state, "get_attachment_import_config", json!({})).await;
        let payload = &body["result"]["structuredContent"];
        assert_eq!(body["result"]["isError"], false);
        assert_eq!(payload["vault_id"], json!(vault_id));
        assert_eq!(payload["enabled"], true);

        let methods = payload["methods"].as_array().expect("methods array");
        assert_eq!(methods.len(), 2);
        assert_eq!(methods[0]["id"], "http_multipart");
        assert_eq!(
            methods[0]["path"],
            format!("/api/v1/vaults/{vault_id}/attachments")
        );
        assert_eq!(methods[1]["id"], "mcp_base64");
        assert!(
            payload["allowed_extensions"]
                .as_array()
                .expect("extensions")
                .contains(&json!("png"))
        );
        assert!(
            methods[0]["auth"]
                .as_str()
                .expect("auth guidance")
                .contains("MCP token is accepted only while MCP and MCP write mode are both currently enabled")
        );
    }

    #[tokio::test]
    async fn listing_note_attachments_needs_no_write_permission() {
        let (state, _tmp) = test_state();
        let body = call_tool(&state, "list_note_attachments", json!({"slug": "home"})).await;
        assert_eq!(body["result"]["isError"], false);
        assert!(body["result"]["structuredContent"]["attachments"].is_array());
    }

    #[tokio::test]
    async fn get_attachment_returns_a_working_download_url_by_default() {
        // get_attachment needs no write permission and no note context: the
        // attachment only has to exist on disk at relative_path.
        let (state, _tmp) = test_state();
        let vault_id = state
            .vaults
            .snapshot()
            .vaults
            .keys()
            .next()
            .copied()
            .expect("registered test Vault");
        let vault_path = registered_vault_path(&state);
        std::fs::create_dir_all(vault_path.join("Sources")).expect("sources dir");
        std::fs::write(vault_path.join("Sources/diagram.png"), b"png-bytes").expect("attachment");

        let body = call_tool(
            &state,
            "get_attachment",
            json!({"relative_path": "Sources/diagram.png"}),
        )
        .await;
        assert_eq!(body["result"]["isError"], false);
        let content = &body["result"]["structuredContent"];
        assert_eq!(content["vault_id"], json!(vault_id));
        assert_eq!(content["relative_path"], "Sources/diagram.png");
        assert_eq!(content["size_bytes"], 9);
        assert_eq!(content["content_type"], "image/png");
        assert_eq!(content["content"]["encoding"], "url");
        assert_eq!(
            content["content"]["download_url"],
            format!("/api/v1/vaults/{vault_id}/assets/Sources/diagram.png")
        );
        assert!(
            content["content"]["auth"]
                .as_str()
                .unwrap()
                .contains("web bearer token")
        );
    }

    #[tokio::test]
    async fn get_attachment_base64_decodes_byte_identically() {
        use base64::Engine as _;

        let (state, _tmp) = test_state();
        let vault_path = registered_vault_path(&state);
        let bytes = b"not really a png but bytes are bytes";
        std::fs::write(vault_path.join("clip.png"), bytes).expect("attachment");

        let body = call_tool(
            &state,
            "get_attachment",
            json!({"relative_path": "clip.png", "encoding": "base64"}),
        )
        .await;
        assert_eq!(body["result"]["isError"], false);
        let content = &body["result"]["structuredContent"];
        assert_eq!(content["content"]["encoding"], "base64");
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(content["content"]["content"].as_str().expect("content"))
            .expect("valid base64");
        assert_eq!(decoded, bytes);
    }

    #[tokio::test]
    async fn get_attachment_base64_refuses_past_the_configured_cap() {
        let (state, _tmp) = test_state();
        state
            .runtime_config
            .save([(
                "HATCHDOOR_MCP_MAX_BASE64_BYTES".to_string(),
                "4".to_string(),
            )])
            .expect("lower the base64 cap");
        let vault_path = registered_vault_path(&state);
        std::fs::write(vault_path.join("clip.png"), b"more than four bytes").expect("attachment");

        let body = call_tool(
            &state,
            "get_attachment",
            json!({"relative_path": "clip.png", "encoding": "base64"}),
        )
        .await;
        assert_eq!(body["error"]["code"], -32602);
        assert!(
            body["error"]["message"]
                .as_str()
                .unwrap()
                .contains("exceeds max size for base64 encoding")
        );
    }

    #[tokio::test]
    async fn get_attachment_reports_the_same_containment_error_the_assets_route_would() {
        let (state, _tmp) = test_state();

        let missing = call_tool(
            &state,
            "get_attachment",
            json!({"relative_path": "nope.png"}),
        )
        .await;
        assert_eq!(missing["result"]["isError"], true);
        assert_eq!(
            missing["result"]["structuredContent"]["code"],
            "asset_not_found"
        );

        let traversal = call_tool(
            &state,
            "get_attachment",
            json!({"relative_path": "../outside.png"}),
        )
        .await;
        assert_eq!(traversal["result"]["isError"], true);
        assert_eq!(
            traversal["result"]["structuredContent"]["code"],
            "invalid_asset_path"
        );
    }

    /// #188's cross-surface gating criterion. The four attachment and
    /// frontmatter tools used to bypass `VaultReadCore` entirely, so a Note or
    /// asset the browse surface withholds was still readable over MCP. They now
    /// go through the same core the HTTP routes do, and refuse identically.
    ///
    /// Demo mode and MCP cannot run in the same process (ADR-07), so this drives
    /// the tool dispatcher and the HTTP handler directly on a demo-mode state
    /// rather than through the MCP transport, which such an instance never
    /// exposes. Nothing visible changes on a production instance; the point is
    /// that the two surfaces answer from one policy.
    #[tokio::test]
    async fn a_withheld_note_and_asset_are_refused_identically_over_mcp_and_http() {
        let (state, _tmp) = layered_test_state();
        let vault_path = registered_vault_path(&state);
        std::fs::write(vault_path.join("sources/clip.png"), b"png").expect("demoted asset");
        let vault_id = vault_id_of(&state);

        let mut demo = state.clone();
        demo.demo_mode = true;

        let tool = |state: AppState, name: &'static str, arguments: Value| async move {
            let result = crate::mcp::tools::handle_tools_call(
                state,
                Some(json!({"name": name, "arguments": arguments})),
                &McpConfig::disabled(),
            )
            .await
            .expect("tool result");
            result["structuredContent"]["code"]
                .as_str()
                .map(str::to_string)
        };

        // An ordinary instance still reads all three.
        for (name, arguments) in [
            (
                "get_frontmatter",
                json!({"vault_id": vault_id, "slug": "clip"}),
            ),
            (
                "list_note_attachments",
                json!({"vault_id": vault_id, "slug": "clip"}),
            ),
            (
                "get_attachment",
                json!({"vault_id": vault_id, "relative_path": "sources/clip.png"}),
            ),
        ] {
            assert_eq!(
                tool(state.clone(), name, arguments).await,
                None,
                "{name} answers on an ordinary instance"
            );
        }

        // On a restricted surface the demoted Note is withheld from both
        // Note-shaped tools, as the ordinary not-found an absent Note gets.
        assert_eq!(
            tool(
                demo.clone(),
                "get_frontmatter",
                json!({"vault_id": vault_id, "slug": "clip"})
            )
            .await
            .as_deref(),
            Some("note_not_found")
        );
        assert_eq!(
            tool(
                demo.clone(),
                "list_note_attachments",
                json!({"vault_id": vault_id, "slug": "clip"})
            )
            .await
            .as_deref(),
            Some("note_not_found")
        );

        // And the demoted asset is refused with the same code the HTTP asset
        // route reports for the same path on the same instance.
        assert_eq!(
            tool(
                demo.clone(),
                "get_attachment",
                json!({"vault_id": vault_id, "relative_path": "sources/clip.png"})
            )
            .await
            .as_deref(),
            Some("asset_not_found")
        );
        let response = crate::handlers::vault_scoped_asset_handler(
            axum::extract::State(demo.clone()),
            None,
            axum::extract::Path((vault_id.to_string(), "sources/clip.png".to_string())),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("asset error body");
        let body: Value = serde_json::from_slice(&body).expect("json body");
        assert_eq!(body["code"], "asset_not_found");
    }

    #[tokio::test]
    async fn attachment_import_config_names_the_gate_that_disabled_upload() {
        let (state, _tmp) = test_state();
        let body = call_tool(&state, "get_attachment_import_config", json!({})).await;
        let payload = &body["result"]["structuredContent"];
        assert_eq!(payload["enabled"], false);
        assert_eq!(payload["write_mode_enabled"], false);
        assert_eq!(payload["vault_accepts_mutation"], true);
        assert!(payload["methods"].as_array().expect("methods").is_empty());
        assert!(
            payload["usage"]
                .as_str()
                .expect("usage")
                .contains("HATCHDOOR_MCP_WRITE_ENABLED")
        );
    }

    #[tokio::test]
    async fn write_tools_refuse_the_layer_marker_basename() {
        let (state, _tmp) = layered_test_state();
        let create = call_tool(
            &state,
            "create_note",
            json!({"relative_path": "wiki/.hatchdoor-layer", "content": "sneaky"}),
        )
        .await;
        assert_eq!(create["error"]["code"], -32602);

        let import =
            import_attachment_call(&state, &b64(b"x"), "wiki/.hatchdoor-layer", false).await;
        assert_eq!(import["error"]["code"], -32602);
    }

    #[tokio::test]
    async fn write_tools_refuse_a_noise_matched_target_path() {
        let (state, _tmp) = layered_write_state();
        let create = call_tool(
            &state,
            "create_note",
            json!({"relative_path": "notes/scratch.tmp", "content": "ignore me"}),
        )
        .await;
        assert_eq!(create["error"]["code"], -32602);
        assert!(
            create["error"]["message"]
                .as_str()
                .unwrap()
                .contains("noise-exclusion")
        );

        let import =
            import_attachment_call(&state, &b64(b"x"), ".obsidian/pasted.png", false).await;
        assert_eq!(import["error"]["code"], -32602);
    }

    #[tokio::test]
    async fn archiving_a_demoted_note_promotes_it_to_the_default_surface() {
        let (state, _tmp) = layered_write_state();
        let before = call_tool(&state, "get_note", json!({"slug": "clip"})).await;
        let note = &before["result"]["structuredContent"]["note"];
        let hash = note["content_hash"].as_str().expect("content hash");

        let archived = call_tool(
            &state,
            "archive_note",
            json!({"slug": "clip", "expected_content_hash": hash}),
        )
        .await;
        let content = &archived["result"]["structuredContent"];
        assert_eq!(content["ok"], true);
        assert_eq!(content["relative_path"], "90-archive/Clip");
        assert_eq!(content["layer"], Value::Null);
    }

    #[tokio::test]
    async fn internal_failures_hide_diagnostics_behind_stable_messages() {
        // The stable-message rule lives at the adapter boundary now; verify it
        // end to end by breaking a write's underlying Vault directory.
        let (state, _tmp) = write_state();
        let vault_path = registered_vault_path(&state);
        std::fs::remove_dir_all(&vault_path).expect("remove vault dir");

        let body = call_tool(
            &state,
            "update_note",
            json!({"slug": "home", "content": "new", "expected_content_hash": "irrelevant"}),
        )
        .await;
        assert_eq!(body["result"]["isError"], true);
        assert_eq!(
            body["result"]["structuredContent"]["code"],
            "vault_read_unavailable"
        );
    }

    #[tokio::test]
    async fn import_attachment_round_trip() {
        let (state, _tmp) = write_state();
        let body =
            import_attachment_call(&state, &b64(b"png-bytes"), "Assets/diagram.png", false).await;
        let attachment = &body["result"]["structuredContent"]["attachment"];
        assert_eq!(attachment["relative_path"], "Assets/diagram.png");
        assert_eq!(attachment["size_bytes"], 9);
        let vault_path = registered_vault_path(&state);
        assert_eq!(
            std::fs::read(vault_path.join("Assets/diagram.png")).expect("read attachment"),
            b"png-bytes"
        );

        let invalid =
            import_attachment_call(&state, "this is not valid base64!!!", "Assets/x.png", false)
                .await;
        assert_eq!(invalid["error"]["code"], -32602);

        let conflict =
            import_attachment_call(&state, &b64(b"second"), "Assets/diagram.png", false).await;
        assert_eq!(conflict["result"]["isError"], true);
        assert_eq!(
            conflict["result"]["structuredContent"]["code"],
            "write_conflict"
        );

        let overwrite =
            import_attachment_call(&state, &b64(b"second"), "Assets/diagram.png", true).await;
        assert_eq!(overwrite["result"]["structuredContent"]["ok"], true);
    }

    #[tokio::test]
    async fn write_tool_creates_a_note() {
        let (state, _tmp) = write_state();
        let created = call_tool(
            &state,
            "create_note",
            json!({"relative_path": "Projects/New.md", "content": "# New\ncreated from MCP"}),
        )
        .await;
        assert_eq!(created["result"]["structuredContent"]["ok"], true);
        assert!(
            registered_vault_path(&state)
                .join("Projects/New.md")
                .exists()
        );
    }

    #[tokio::test]
    async fn get_frontmatter_projects_metadata_without_the_body() {
        // test_state's notes have no frontmatter: an empty projection, not an
        // error (acceptance criterion).
        let (state, _tmp) = test_state();
        let body = call_tool(&state, "get_frontmatter", json!({"slug": "home"})).await;
        let content = &body["result"]["structuredContent"];
        assert_eq!(content["has_frontmatter"], false);
        assert_eq!(content["tags"], json!([]));
        assert_eq!(content["properties"], json!({}));
        let serialized = serde_json::to_string(content).expect("serialize");
        assert!(
            !serialized.contains("alpha token"),
            "body text never appears: {serialized}"
        );

        // A note with frontmatter projects tags/aliases/properties.
        let vault_path = registered_vault_path(&state);
        std::fs::write(
            vault_path.join("Tagged.md"),
            "---\ntags:\n  - space/hobby\naliases:\n  - Tagged Home\nstatus: active\n---\n\n# Tagged\nsecret body\n",
        )
        .expect("write tagged note");
        let index = crate::vault::VaultIndex::build(&vault_path).expect("index");
        let vault_id = match state.vault_registry.load().expect("load registry") {
            crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot
                .definitions()
                .next()
                .expect("test definition")
                .vault_id(),
            crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("test recovery"),
        };
        state
            .startup_sqlite
            .replace_vault_snapshot(vault_id, &index, state.embedder.as_ref())
            .expect("republish snapshot");
        let body = call_tool(&state, "get_frontmatter", json!({"slug": "tagged"})).await;
        let content = &body["result"]["structuredContent"];
        assert_eq!(content["has_frontmatter"], true);
        assert_eq!(content["tags"], json!(["space/hobby"]));
        assert_eq!(content["aliases"], json!(["Tagged Home"]));
        assert_eq!(content["properties"]["status"], "active");
        let serialized = serde_json::to_string(content).expect("serialize");
        assert!(!serialized.contains("secret body"));
    }

    #[tokio::test]
    async fn get_frontmatter_reports_the_same_content_hash_get_note_does() {
        // A note with no frontmatter block still answers a hash, because the
        // hash covers the whole file rather than the frontmatter span (#227).
        let (state, _tmp) = test_state();
        let frontmatter = call_tool(&state, "get_frontmatter", json!({"slug": "home"})).await;
        let projection = &frontmatter["result"]["structuredContent"];
        assert_eq!(
            projection["has_frontmatter"], false,
            "the fixture this criterion rests on has no frontmatter block"
        );
        let hash = projection["content_hash"]
            .as_str()
            .expect("frontmatter hash")
            .to_string();
        assert_eq!(
            hash,
            crate::cache::parse::content_hash("# Home\nalpha token\n[[Plan]]")
        );

        let note = call_tool(&state, "get_note", json!({"slug": "home"})).await;
        assert_eq!(
            note["result"]["structuredContent"]["note"]["content_hash"]
                .as_str()
                .expect("note hash"),
            hash,
            "both exact reads report one hash for the same note"
        );
    }

    #[tokio::test]
    async fn get_frontmatter_hash_is_accepted_by_a_hash_protected_mutation() {
        // The point of the field: harvest the hash at frontmatter cost, then
        // spend it on an optimistic-concurrency write without a full read.
        let (state, _tmp) = write_state();
        let frontmatter = call_tool(&state, "get_frontmatter", json!({"slug": "home"})).await;
        let hash = frontmatter["result"]["structuredContent"]["content_hash"]
            .as_str()
            .expect("frontmatter hash")
            .to_string();

        let updated = call_tool(
            &state,
            "update_frontmatter",
            json!({"slug": "home", "frontmatter": {"status": "active"}, "expected_content_hash": hash}),
        )
        .await;
        assert_eq!(
            updated["result"]["structuredContent"]["ok"], true,
            "harvested hash is spendable: {updated}"
        );
    }

    #[tokio::test]
    async fn update_frontmatter_merges_and_preserves_the_body() {
        let (state, _tmp) = write_state();
        // Matches the test_state fixture byte-for-byte (no trailing newline).
        let hash = crate::cache::parse::content_hash("# Home\nalpha token\n[[Plan]]");
        let updated = call_tool(
            &state,
            "update_frontmatter",
            json!({"slug": "home", "frontmatter": {"tags": ["one", "two"], "status": "active"}, "expected_content_hash": hash}),
        )
        .await;
        assert_eq!(updated["result"]["structuredContent"]["ok"], true);
        let content =
            std::fs::read_to_string(registered_vault_path(&state).join("Home.md")).expect("read");
        assert!(content.starts_with("---\n"), "block created: {content:?}");
        assert_eq!(
            content,
            "---\nstatus: active\ntags:\n- one\n- two\n---\n# Home\nalpha token\n[[Plan]]"
        );
        let new_hash = updated["result"]["structuredContent"]["content_hash"]
            .as_str()
            .expect("new hash")
            .to_string();

        // Shallow semantics: null deletes one key, unmentioned keys survive.
        let second = call_tool(
            &state,
            "update_frontmatter",
            json!({"slug": "home", "frontmatter": {"status": null}, "expected_content_hash": new_hash}),
        )
        .await;
        assert_eq!(second["result"]["structuredContent"]["ok"], true);
        let content =
            std::fs::read_to_string(registered_vault_path(&state).join("Home.md")).expect("read");
        assert!(content.contains("tags:"), "unmentioned key survives");
        assert!(
            !content.contains("status"),
            "null deletes the key: {content:?}"
        );

        // Stale hash fails with the same structured error as other writes.
        let stale = call_tool(
            &state,
            "update_frontmatter",
            json!({"slug": "home", "frontmatter": {"x": 1}, "expected_content_hash": hash}),
        )
        .await;
        assert_eq!(stale["result"]["isError"], true);
        assert_eq!(
            stale["result"]["structuredContent"]["code"],
            "write_conflict"
        );
    }

    #[tokio::test]
    async fn edit_note_replaces_string() {
        let (state, _tmp) = write_state();
        let hash = crate::cache::parse::content_hash("# Home\nalpha token\n[[Plan]]");
        let edited = call_tool(
            &state,
            "edit_note",
            json!({
                "slug": "home",
                "old_string": "alpha",
                "new_string": "ALPHA",
                "expected_content_hash": hash
            }),
        )
        .await;
        assert_eq!(edited["result"]["structuredContent"]["ok"], true);
        assert_eq!(
            std::fs::read_to_string(registered_vault_path(&state).join("Home.md"),).expect("read"),
            "# Home\nALPHA token\n[[Plan]]\n"
        );
    }

    #[tokio::test]
    async fn rename_note_returns_new_slug() {
        let (state, _tmp) = write_state();
        let hash = crate::cache::parse::content_hash("# Home\nalpha token\n[[Plan]]");
        let renamed = call_tool(
            &state,
            "rename_note",
            json!({"slug": "home", "new_title": "Renamed Home", "expected_content_hash": hash}),
        )
        .await;
        let content = &renamed["result"]["structuredContent"];
        assert_eq!(content["ok"], true);
        assert_eq!(content["slug"], "renamed-home");
        let vault_path = registered_vault_path(&state);
        assert!(vault_path.join("Renamed Home.md").exists());
        assert!(!vault_path.join("Home.md").exists());
    }

    #[tokio::test]
    async fn replace_section_overwrites_and_rejects_invalid_mode() {
        let (state, _tmp) = write_state();
        let hash = crate::cache::parse::content_hash("# Home\nalpha token\n[[Plan]]");
        let section = call_tool(
            &state,
            "replace_section",
            json!({
                "slug": "home",
                "heading": "# Home",
                "mode": "replace",
                "content": "# Home\nrewritten\n",
                "expected_content_hash": hash
            }),
        )
        .await;
        assert_eq!(section["result"]["structuredContent"]["ok"], true);
        assert_eq!(
            std::fs::read_to_string(registered_vault_path(&state).join("Home.md"),).expect("read"),
            "# Home\nrewritten\n"
        );

        let bad_mode = call_tool(
            &state,
            "replace_section",
            json!({
                "slug": "home",
                "heading": "# Home",
                "mode": "sideways",
                "content": "x",
                "expected_content_hash": hash
            }),
        )
        .await;
        assert_eq!(bad_mode["error"]["code"], -32602);
    }

    #[tokio::test]
    async fn search_notes_returns_compact_results() {
        let (state, _tmp) = test_state();
        let body = call_tool(&state, "search_notes", json!({"query":"Home", "limit": 5})).await;
        let results = body["result"]["structuredContent"]["data"]["results"]
            .as_array()
            .expect("results array");
        assert!(results.iter().any(|r| r["note_slug"] == "home"));
        let first = &results[0];
        for key in ["vault_id", "note_slug", "chunk_id", "content", "score"] {
            assert!(first.get(key).is_some(), "{key} present");
        }
    }

    #[tokio::test]
    async fn list_vaults_redacts_configured_credentials() {
        let (state, _tmp) = write_state();
        let body = call_tool(&state, "list_vaults", json!({})).await;
        let discovery = &body["result"]["structuredContent"];
        assert!(discovery["registry_revision"].is_u64());
        let vault = &discovery["vaults"][0];
        assert_eq!(vault["credential_configured"], false);
        assert!(vault.get("https_credentials").is_none());
    }

    // ---------------------------------------------------------------------------
    // Modern subscriptions/listen (#170).
    // ---------------------------------------------------------------------------

    use std::time::Duration;
    use tokio_stream::StreamExt;

    /// Pump one SSE response body into a channel of parsed `data:` payloads so
    /// tests can await notifications as they arrive on the long-lived stream.
    fn pump_sse(response: Response) -> tokio::sync::mpsc::Receiver<Value> {
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        let mut stream = response.into_body().into_data_stream();
        tokio::spawn(async move {
            let mut buffer = String::new();
            while let Some(chunk) = stream.next().await {
                let Ok(chunk) = chunk else { break };
                buffer.push_str(&String::from_utf8_lossy(&chunk));
                while let Some(index) = buffer.find("\n") {
                    let line: String = buffer.drain(..=index).collect();
                    let Some(data) = line.strip_prefix("data:") else {
                        continue;
                    };
                    if let Ok(message) = serde_json::from_str::<Value>(data.trim())
                        && tx.send(message).await.is_err()
                    {
                        return;
                    }
                }
            }
        });
        rx
    }

    async fn next_message(rx: &mut tokio::sync::mpsc::Receiver<Value>) -> Value {
        tokio::time::timeout(Duration::from_secs(10), rx.recv())
            .await
            .expect("message within timeout")
            .expect("stream open")
    }

    async fn open_listen(app: &Router, filter: Value) -> (tokio::sync::mpsc::Receiver<Value>, u64) {
        let raw = modern_post(
            app.clone(),
            "subscriptions/listen",
            None,
            "2026-07-28",
            json!({
                "jsonrpc":"2.0","id":100,"method":"subscriptions/listen",
                "params":{"_meta": modern_meta("2026-07-28", true),
                          "notifications": filter}
            }),
        )
        .await;
        assert_eq!(raw.status(), StatusCode::OK, "listen accepted");
        let id = 100;
        (pump_sse(raw), id)
    }

    #[tokio::test]
    async fn modern_discover_advertises_tools_list_changed_true() {
        let (state, _tmp) = test_state();
        let app = transport(&state);
        let raw = modern_post(
            app,
            "server/discover",
            Some("server/discover"),
            "2026-07-28",
            json!({"jsonrpc":"2.0","id":1,"method":"server/discover",
                   "params":{"_meta": modern_meta("2026-07-28", true)}}),
        )
        .await;
        let message = response_message(raw).await;
        assert_eq!(
            message["result"]["capabilities"]["tools"]["listChanged"], true,
            "the modern surface delivers change events, so it advertises them"
        );
    }

    #[tokio::test]
    async fn subscription_delivers_acknowledgment_and_tool_list_changed_events() {
        let (state, _tmp) = test_state();
        let app = transport(&state);
        let mut rx = open_listen(&app, json!({"toolsListChanged": true})).await.0;

        // Acknowledgment first: the accepted subset of our requested filter.
        let ack = next_message(&mut rx).await;
        assert_eq!(ack["method"], "notifications/subscriptions/acknowledged");
        assert_eq!(
            ack["params"]["notifications"],
            json!({"toolsListChanged": true})
        );
        assert!(
            ack["params"]["_meta"]["io.modelcontextprotocol/subscriptionId"].is_number(),
            "ack carries the subscription metadata"
        );

        // The catalogue changed seam fires; the client hears about it.
        state.mcp_tools_changed.send(()).expect("broadcast");
        let event = next_message(&mut rx).await;
        assert_eq!(event["method"], "notifications/tools/list_changed");
        assert!(event["params"]["_meta"]["io.modelcontextprotocol/subscriptionId"].is_number());
    }

    #[tokio::test]
    async fn unrequested_notification_categories_are_removed_from_the_acknowledgment() {
        let (state, _tmp) = test_state();
        let app = transport(&state);
        let mut rx = open_listen(
            &app,
            json!({"toolsListChanged": true, "promptsListChanged": true,
                   "resourcesListChanged": true}),
        )
        .await
        .0;
        let ack = next_message(&mut rx).await;
        assert_eq!(
            ack["params"]["notifications"],
            json!({"toolsListChanged": true}),
            "only the advertised tools.listChanged category survives"
        );
    }

    #[tokio::test]
    async fn fifth_concurrent_subscription_per_token_is_rejected_then_reopens_after_disconnect() {
        let (state, _tmp) = test_state();
        let app = transport(&state);
        let filter = json!({"toolsListChanged": true});
        let mut streams = Vec::new();
        for _ in 0..4 {
            let (rx, _) = open_listen(&app, filter.clone()).await;
            streams.push(rx);
        }
        for rx in &mut streams {
            // Each established stream must have passed its cap slot.
            let ack = next_message(rx).await;
            assert_eq!(ack["method"], "notifications/subscriptions/acknowledged");
        }

        // The cap is applied when each stream's listener task starts, a moment
        // after its acknowledgment; poll until the budget is fully consumed.
        let rejected = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let attempt = modern_post(
                    app.clone(),
                    "subscriptions/listen",
                    None,
                    "2026-07-28",
                    json!({"jsonrpc":"2.0","id":200,"method":"subscriptions/listen",
                           "params":{"_meta": modern_meta("2026-07-28", true),
                                     "notifications": filter}}),
                )
                .await;
                if !attempt.status().is_success() {
                    let bytes = to_bytes(attempt.into_body(), usize::MAX).await.unwrap();
                    return serde_json::from_slice::<Value>(&bytes).unwrap_or(json!({}));
                }
                // A streamed refusal still answers 200 and its first
                // payload is the acknowledgment; keep reading for the error.
                let mut messages = pump_sse(attempt);
                while let Ok(Some(message)) =
                    tokio::time::timeout(Duration::from_secs(2), messages.recv()).await
                {
                    if message.get("error").is_some() {
                        return message;
                    }
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("fifth live subscription refused");
        assert_eq!(rejected["error"]["code"], -32600);
        assert!(
            rejected["error"]["message"]
                .as_str()
                .unwrap()
                .contains("subscriptions")
        );

        // Disconnecting one live stream frees its slot again.
        streams.pop();
        let reopened_at = tokio::time::timeout(Duration::from_secs(10), async move {
            loop {
                let attempt = modern_post(
                    app.clone(),
                    "subscriptions/listen",
                    None,
                    "2026-07-28",
                    json!({"jsonrpc":"2.0","id":201,"method":"subscriptions/listen",
                           "params":{"_meta": modern_meta("2026-07-28", true),
                                     "notifications": {"toolsListChanged": true}}}),
                )
                .await;
                if attempt.status() == StatusCode::OK {
                    return StatusCode::OK;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await;
        assert_eq!(
            reopened_at,
            Ok(StatusCode::OK),
            "slot freed after disconnect"
        );
    }

    #[tokio::test]
    async fn legacy_sessions_cannot_open_subscriptions() {
        let (state, _tmp) = test_state();
        let app = transport(&state);
        let (session, _) = initialize(&app).await;
        let raw = rpc(
            &app,
            &session,
            json!({"jsonrpc":"2.0","id":50,"method":"subscriptions/listen",
                   "params":{"notifications":{"toolsListChanged":true}}}),
        )
        .await;
        let message = response_message(raw).await;
        assert_eq!(message["error"]["code"], -32601);
    }

    // ---------------------------------------------------------------------------
    // Error-semantics matrix (#172): malformed JSON-RPC, unknown methods and
    // tools, and structurally invalid calls are protocol errors; valid calls
    // with actionable failures stay `isError: true` tool results (covered by
    // the golden tests above, e.g. missing_note_is_a_tool_error...).
    // ---------------------------------------------------------------------------

    async fn raw_modern_post(
        app: Router,
        method_header: &str,
        name_header: Option<&str>,
        body: String,
    ) -> Response {
        let mut headers = auth_headers(TEST_TOKEN);
        headers.push(("mcp-protocol-version", "2026-07-28".to_string()));
        headers.push(("Mcp-Method", method_header.to_string()));
        if let Some(name) = name_header {
            headers.push(("Mcp-Name", name.to_string()));
        }
        headers.extend(json_post_headers());
        send(app, "POST", headers, Some(body)).await
    }

    fn json_post_headers() -> Vec<(&'static str, String)> {
        vec![
            ("content-type", "application/json".to_string()),
            ("accept", "application/json, text/event-stream".to_string()),
        ]
    }

    /// Read a non-success response body without panicking, so error-path
    /// golden tests can assert on the JSON-RPC protocol-error payload itself.
    async fn error_body(response: Response) -> Value {
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    }

    #[tokio::test]
    async fn malformed_json_body_is_a_protocol_error_for_both_revisions() {
        let (state, _tmp) = test_state();
        let app = transport(&state);
        let (session, _) = initialize(&app).await;

        let mut headers = auth_headers(TEST_TOKEN);
        headers.push(("mcp-session-id", session.id));
        headers.extend(json_post_headers());
        let response = send(
            app.clone(),
            "POST",
            headers,
            Some("{not json at all".to_string()),
        )
        .await;
        // The body never parses as JSON-RPC, so there is no in-envelope code
        // to pin: the JSON extractor rejects it pre-framing with a 4xx and an
        // explanatory plain-text body. The guarantee under test is that an
        // unparseable request can never dispatch as a success on either wire.
        assert!(
            response.status().is_client_error(),
            "legacy: unparseable JSON-RPC must never dispatch as success"
        );

        let modern = raw_modern_post(app, "tools/list", None, "{not json at all".to_string()).await;
        assert!(
            modern.status().is_client_error(),
            "modern: unparseable JSON-RPC must never dispatch as success"
        );
    }

    #[tokio::test]
    async fn unknown_tool_is_a_protocol_error_for_both_revisions() {
        let (state, _tmp) = test_state();
        let app = transport(&state);

        // Legacy revision (initialize-negotiated): in-envelope JSON-RPC error.
        let legacy = call_tool(&state, "no_such_tool", json!({})).await;
        assert_eq!(legacy["error"]["code"], -32602);
        assert_eq!(legacy["error"]["message"], "Unknown MCP tool: no_such_tool");

        // Modern revision: HTTP 400 with the same protocol error semantics.
        let modern = raw_modern_post(
            app,
            "tools/call",
            Some("no_such_tool"),
            json!({
                "jsonrpc":"2.0","id":15,"method":"tools/call",
                "params":{"_meta": modern_meta("2026-07-28", true),
                          "name":"no_such_tool","arguments":{}}
            })
            .to_string(),
        )
        .await;
        assert_eq!(modern.status(), StatusCode::BAD_REQUEST);
        let message = error_body(modern).await;
        assert_eq!(message["error"]["code"], -32602);
        assert_eq!(
            message["error"]["message"],
            "Unknown MCP tool: no_such_tool"
        );
    }

    #[tokio::test]
    async fn unknown_method_is_a_protocol_error_for_the_modern_revision() {
        let (state, _tmp) = test_state();
        let app = transport(&state);
        let response = raw_modern_post(
            app,
            "prompts/list",
            None,
            json!({
                "jsonrpc":"2.0","id":12,"method":"prompts/list",
                "params":{"_meta": modern_meta("2026-07-28", true)}
            })
            .to_string(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let message = error_body(response).await;
        assert_eq!(message["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn structurally_invalid_calls_are_protocol_errors_for_both_revisions() {
        let (state, _tmp) = test_state();
        let app = transport(&state);
        let (session, _) = initialize(&app).await;

        // Legacy revision: tools/call without params at all.
        let legacy = rpc(
            &app,
            &session,
            json!({"jsonrpc":"2.0","id":13,"method":"tools/call"}),
        )
        .await;
        // rmcp reports protocol errors inside an HTTP 200 envelope.
        let message = response_message(legacy).await;
        assert_eq!(message["error"]["code"], -32601);

        // Modern revision: shapeless body with otherwise valid headers.
        let modern = raw_modern_post(
            app,
            "tools/call",
            Some("get_note"),
            json!({
                "jsonrpc":"2.0","id":14,"method":"tools/call",
                "params":{"_meta": modern_meta("2026-07-28", true)}
            })
            .to_string(),
        )
        .await;
        assert_eq!(modern.status(), StatusCode::NOT_FOUND);
        let message = error_body(modern).await;
        assert_eq!(message["error"]["code"], -32601);
    }

    // ---------------------------------------------------------------------------
    // `batch` (issue #177)
    // ---------------------------------------------------------------------------

    fn vault_id_of(state: &AppState) -> crate::vault_registry::VaultId {
        *state
            .vaults
            .snapshot()
            .vaults
            .keys()
            .next()
            .expect("test Vault registered")
    }

    #[tokio::test]
    async fn batch_rejects_a_vault_management_op_before_executing_anything() {
        let (state, _tmp) = write_state();
        let vault_id = vault_id_of(&state);

        for (index, management) in [
            json!({"op": "disable_vault", "arguments": {"vault_id": vault_id, "expected_registry_revision": 0}}),
            json!({"op": "refresh_vault", "arguments": {"vault_id": vault_id}}),
        ]
        .into_iter()
        .enumerate()
        {
            let op = management["op"].clone();
            // A distinct path per case, so a note left behind by one case
            // cannot be mistaken for the other's.
            let relative_path = format!("Should/NotExist{index}.md");
            let body = call_tool(
                &state,
                "batch",
                json!({"operations": [
                    management,
                    {"op": "create_note", "arguments": {"vault_id": vault_id, "relative_path": relative_path, "content": "x"}}
                ]}),
            )
            .await;

            assert_eq!(body["error"]["code"], -32602, "{op}: {body:#}");
            assert!(
                body["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("not a valid batch operation"),
                "{op}: {body:#}"
            );
            assert!(
                !registered_vault_path(&state).join(&relative_path).exists(),
                "{op}: nothing in the batch may execute once any op is rejected up front"
            );
        }
    }

    #[tokio::test]
    async fn batch_rejects_an_unrecognized_op() {
        let (state, _tmp) = write_state();
        let vault_id = vault_id_of(&state);

        let body = call_tool(
            &state,
            "batch",
            json!({"operations": [
                {"op": "not_a_real_tool", "arguments": {"vault_id": vault_id}}
            ]}),
        )
        .await;

        assert_eq!(body["error"]["code"], -32602);
    }

    #[tokio::test]
    async fn batch_rejects_an_oversized_write_batch_wholesale() {
        let (state, _tmp) = write_state();
        let vault_id = vault_id_of(&state);

        let operations: Vec<Value> = (0..=crate::mcp::limits::BATCH_MAX_WRITE_ITEMS)
            .map(|i| {
                json!({
                    "op": "create_note",
                    "arguments": {
                        "vault_id": vault_id,
                        "relative_path": format!("Batch/Note{i}.md"),
                        "content": "x"
                    }
                })
            })
            .collect();

        let body = call_tool(&state, "batch", json!({"operations": operations})).await;

        assert_eq!(body["error"]["code"], -32602);
        assert!(
            body["error"]["message"]
                .as_str()
                .unwrap()
                .contains("write-shaped items")
        );
        assert!(
            !registered_vault_path(&state)
                .join("Batch/Note0.md")
                .exists(),
            "an over-cap batch must be refused before any item executes"
        );
    }

    #[tokio::test]
    async fn batch_rejects_an_oversized_read_batch_wholesale() {
        let (state, _tmp) = test_state();
        let vault_id = vault_id_of(&state);

        let operations: Vec<Value> = (0..=crate::mcp::limits::BATCH_MAX_READ_ITEMS)
            .map(|_| json!({"op": "get_note", "arguments": {"vault_id": vault_id, "slug": "home"}}))
            .collect();

        let body = call_tool(&state, "batch", json!({"operations": operations})).await;

        assert_eq!(body["error"]["code"], -32602);
        assert!(
            body["error"]["message"]
                .as_str()
                .unwrap()
                .contains("read-shaped items")
        );
    }

    #[tokio::test]
    async fn batch_deletes_a_note_and_an_attachment_created_earlier_in_the_same_call() {
        let (state, _tmp) = write_state();
        let vault_id = vault_id_of(&state);

        let body = call_tool(
            &state,
            "batch",
            json!({"operations": [
                {"op": "create_note", "arguments": {
                    "vault_id": vault_id, "relative_path": "Batch/ToDelete.md", "content": "gone soon"
                }},
                // Deleted with a deliberately stale hash: chained from the
                // create above, same as the append test — proves deletes
                // participate in hash chaining too.
                {"op": "delete_note", "arguments": {
                    "vault_id": vault_id, "slug": "todelete",
                    "expected_content_hash": "fnv1a64:deliberately-stale"
                }},
                {"op": "import_attachment", "arguments": {
                    "vault_id": vault_id, "target_relative_path": "Batch/asset.png",
                    "content": b64(b"asset-bytes")
                }},
                {"op": "delete_attachment", "arguments": {
                    "vault_id": vault_id, "source_relative_path": "Batch/asset.png"
                }}
            ]}),
        )
        .await;

        let content = &body["result"]["structuredContent"];
        assert_eq!(content["succeeded"], 4);
        assert_eq!(content["failed"], 0);
        assert_eq!(content["items"][1]["op"], "delete_note");
        assert_eq!(content["items"][1]["result"]["ok"], true);
        assert_eq!(content["items"][3]["op"], "delete_attachment");
        assert_eq!(content["items"][3]["result"]["ok"], true);

        let vault_path = registered_vault_path(&state);
        assert!(!vault_path.join("Batch/ToDelete.md").exists());
        assert!(!vault_path.join("Batch/asset.png").exists());
        assert!(
            vault_path
                .join(".hatchdoor-trash/Batch/ToDelete.md")
                .exists()
        );
        assert!(vault_path.join(".hatchdoor-trash/Batch/asset.png").exists());
    }

    #[tokio::test]
    async fn batch_runs_read_only_operations_without_write_permission() {
        let (state, _tmp) = test_state();
        let vault_id = vault_id_of(&state);

        let body = call_tool(
            &state,
            "batch",
            json!({"operations": [
                {"op": "get_note", "arguments": {"vault_id": vault_id, "slug": "home"}},
                {"op": "get_note_links", "arguments": {"vault_id": vault_id, "slug": "home"}}
            ]}),
        )
        .await;

        let content = &body["result"]["structuredContent"];
        assert_eq!(content["succeeded"], 2);
        assert_eq!(content["failed"], 0);
        assert_eq!(content["items"][0]["result"]["note"]["slug"], "home");
        assert_eq!(content["items"][1]["op"], "get_note_links");
    }

    #[tokio::test]
    async fn batch_is_best_effort_and_chains_hashes_between_items_on_the_same_note() {
        let (state, _tmp) = write_state();
        let vault_id = vault_id_of(&state);

        let body = call_tool(
            &state,
            "batch",
            json!({"operations": [
                {"op": "create_note", "arguments": {
                    "vault_id": vault_id, "relative_path": "Batch/Chained.md", "content": "one"
                }},
                // A deliberately failing item in the middle: best-effort means
                // this must not stop the append below from still running.
                {"op": "get_note", "arguments": {"vault_id": vault_id, "slug": "does-not-exist"}},
                // No intermediate read: this expected_content_hash is stale by
                // construction, and must still succeed because the prior
                // create in this same batch is chained into it.
                {"op": "append_to_note", "arguments": {
                    "vault_id": vault_id, "slug": "chained", "content": "\ntwo",
                    "expected_content_hash": "fnv1a64:deliberately-stale"
                }}
            ]}),
        )
        .await;

        let content = &body["result"]["structuredContent"];
        assert_eq!(content["succeeded"], 2);
        assert_eq!(content["failed"], 1);

        assert_eq!(content["items"][0]["ok"], true);
        assert_eq!(content["items"][0]["op"], "create_note");

        assert_eq!(content["items"][1]["ok"], false);
        assert_eq!(content["items"][1]["op"], "get_note");
        assert!(content["items"][1]["error"].is_object());

        assert_eq!(content["items"][2]["ok"], true);
        assert_eq!(content["items"][2]["op"], "append_to_note");

        let written =
            std::fs::read_to_string(registered_vault_path(&state).join("Batch/Chained.md"))
                .expect("read written note");
        assert_eq!(written, "one\ntwo\n");
    }

    #[tokio::test]
    async fn batch_write_items_are_gated_by_mcp_write_enabled_per_item() {
        let (state, _tmp) = test_state();
        let vault_id = vault_id_of(&state);

        let body = call_tool(
            &state,
            "batch",
            json!({"operations": [
                {"op": "create_note", "arguments": {
                    "vault_id": vault_id, "relative_path": "Batch/Refused.md", "content": "x"
                }}
            ]}),
        )
        .await;

        let content = &body["result"]["structuredContent"];
        assert_eq!(content["succeeded"], 0);
        assert_eq!(content["failed"], 1);
        assert!(
            content["items"][0]["error"]["message"]
                .as_str()
                .unwrap()
                .contains("write tools are disabled")
        );
    }
}
