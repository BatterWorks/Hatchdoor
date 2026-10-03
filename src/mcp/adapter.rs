//! The typed adapter between rmcp's `ServerHandler` seam and Hatchdoor's
//! framework-independent tool catalogue (ADR-17). rmcp owns JSON-RPC framing,
//! Streamable HTTP serving, lifecycle, and version negotiation; this adapter
//! owns nothing wire-level. It converts between rmcp's typed requests/results
//! and the existing JSON-value dispatcher in `tools`, so tool behavior,
//! schemas, and structured error semantics stay byte-compatible with the
//! hand-written surface this boundary replaced.

use std::borrow::Cow;
use std::sync::Arc;

use crate::app_state::AppState;
use rmcp::model::{
    CacheScope, CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ErrorCode,
    ErrorData, Implementation, InitializeResult, ListToolsResult, ProtocolVersion,
    ServerCapabilities, ServerInfo, SubscriptionFilter, Tool, ToolAnnotations,
};
use rmcp::service::{RequestContext, SubscriptionContext};
use rmcp::{RoleServer, ServerHandler};
use serde_json::{Value, json};
use tracing::error;

use super::config::{McpConfig, SERVER_INSTRUCTIONS, SETUP_INSTRUCTIONS};
use super::protocol::JsonRpcFailure;
use super::subscriptions::{MAX_SUBSCRIPTIONS_PER_TOKEN, McpBearerToken, SubscriptionRegistry};
use super::tools;

/// Advertised protocol revisions, newest first (ADR-17). rmcp negotiates
/// `initialize` against this list; a client requesting a retired revision is
/// answered with our preferred legacy revision instead of being served it.
fn advertised_protocol_versions() -> Cow<'static, [rmcp::model::ProtocolVersion]> {
    Cow::Borrowed(&[
        rmcp::model::ProtocolVersion::V_2026_07_28,
        rmcp::model::ProtocolVersion::V_2025_11_25,
    ])
}

/// SEP-2549 cache metadata on discovery and list results: a five-minute
/// private TTL acts as the self-healing fallback for list handling — if a
/// client misses a change notification (or we cannot yet push one), its cached
/// list is refreshed at most five minutes later.
const LIST_CACHE_TTL_MS: u64 = 5 * 60 * 1000;

pub struct HatchdoorMcpHandler {
    state: AppState,
    subscriptions: Arc<SubscriptionRegistry>,
}

impl HatchdoorMcpHandler {
    pub fn new(state: AppState, subscriptions: Arc<SubscriptionRegistry>) -> Self {
        Self {
            state,
            subscriptions,
        }
    }

    fn config(&self) -> Result<McpConfig, String> {
        let snapshot = self.state.runtime_snapshot();
        AppState::runtime_mcp_config(&snapshot)
    }

    /// Remember which client called and when (#426). `client_info` reads the
    /// request's own `_meta` on the modern revision and the `initialize`
    /// handshake on a legacy session. Only the name and time are kept, and a
    /// save that fails is logged by the log itself, never returned to the
    /// caller.
    fn record_agent_connection(&self, context: &RequestContext<RoleServer>) {
        let name = context
            .client_info()
            .map(|client| {
                client
                    .title
                    .filter(|title| !title.trim().is_empty())
                    .unwrap_or(client.name)
            })
            .unwrap_or_default();
        let log = &self.state.agent_connections;
        if log.observe(&name, std::time::SystemTime::now()) {
            let log = Arc::clone(log);
            tokio::task::spawn_blocking(move || log.save());
        }
    }
}

/// The `scheme://host:port` the client reached this MCP endpoint on, which
/// transfer links fall back to when no public address is configured (ADR-34).
/// Scheme and host are each taken from the first source that gives a usable
/// value: the first element of `Forwarded` (RFC 7239), then the first value of
/// `X-Forwarded-Proto` / `X-Forwarded-Host`, then `http` and the `Host` header
/// (or an HTTP/2 request's authority). A value that is unusable is skipped, so
/// a malformed header never fails the call. Trusting these headers from any
/// sender is safe here because the link goes back to the same caller that sent
/// them, and nothing else in Hatchdoor reads them.
fn request_origin(parts: &axum::http::request::Parts) -> Option<String> {
    let headers = &parts.headers;
    let forwarded = headers
        .get(axum::http::header::FORWARDED)
        .and_then(|value| value.to_str().ok())
        .map(first_forwarded_element)
        .unwrap_or_default();
    let scheme = forwarded
        .proto
        .as_deref()
        .and_then(link_scheme)
        .or_else(|| first_header_value(headers, "x-forwarded-proto").and_then(link_scheme))
        .unwrap_or("http");
    let authority = match forwarded
        .host
        .as_deref()
        .and_then(link_authority)
        .or_else(|| first_header_value(headers, "x-forwarded-host").and_then(link_authority))
    {
        Some(authority) => authority,
        None => arriving_authority(parts)?,
    };
    Some(format!("{scheme}://{authority}"))
}

/// The authority the request itself carries: the `Host` header, or an HTTP/2
/// request's authority when it has none. An unusable `Host` yields `None`
/// rather than falling through to the URI.
fn arriving_authority(parts: &axum::http::request::Parts) -> Option<axum::http::uri::Authority> {
    match parts
        .headers
        .get(axum::http::header::HOST)
        .and_then(|value| value.to_str().ok())
    {
        Some(host) => link_authority(host),
        None => link_authority(parts.uri.authority()?.as_str()),
    }
}

/// A link can only be `http` or `https`; anything else is ignored.
fn link_scheme(raw: &str) -> Option<&'static str> {
    let raw = raw.trim();
    if raw.eq_ignore_ascii_case("https") {
        Some("https")
    } else if raw.eq_ignore_ascii_case("http") {
        Some("http")
    } else {
        None
    }
}

/// A host a link can be built on: a valid authority with no credentials in it.
fn link_authority(raw: &str) -> Option<axum::http::uri::Authority> {
    let raw = raw.trim();
    if raw.is_empty() || raw.contains('@') {
        return None;
    }
    raw.parse().ok()
}

/// The first comma-separated value of the first `name` header.
fn first_header_value<'a>(headers: &'a axum::http::HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(name)?
        .to_str()
        .ok()?
        .split(',')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// The `proto` and `host` parameters of one `Forwarded` element.
#[derive(Default)]
struct ForwardedElement {
    proto: Option<String>,
    host: Option<String>,
}

impl ForwardedElement {
    /// Keep a `name=value` pair if it is the first `proto` or `host`; any
    /// other pair, or one without `=`, is ignored.
    fn record(&mut self, pair: &str) {
        let Some((name, value)) = pair.split_once('=') else {
            return;
        };
        let slot = match name.trim().to_ascii_lowercase().as_str() {
            "proto" => &mut self.proto,
            "host" => &mut self.host,
            _ => return,
        };
        if slot.is_none() {
            *slot = Some(value.trim().to_string());
        }
    }
}

/// The parameters of the first element of a `Forwarded` header value. Elements
/// end at `,` and parameters at `;`. A value that opens with `"` is a quoted
/// string, read up to its closing quote with `\\` escaping the next character,
/// so separators inside it do not count. Parameter names are case-insensitive.
fn first_forwarded_element(value: &str) -> ForwardedElement {
    let mut element = ForwardedElement::default();
    let mut pair = String::new();
    let mut chars = value.chars();
    loop {
        match chars.next() {
            Some('"') if pair.ends_with('=') => {
                while let Some(ch) = chars.next() {
                    match ch {
                        '"' => break,
                        '\\' => pair.extend(chars.next()),
                        _ => pair.push(ch),
                    }
                }
            }
            Some(';') => {
                element.record(&pair);
                pair.clear();
            }
            Some(',') | None => {
                element.record(&pair);
                return element;
            }
            Some(ch) => pair.push(ch),
        }
    }
}

impl ServerHandler for HatchdoorMcpHandler {
    fn get_info(&self) -> ServerInfo {
        // `model_setup_pending`, not the collection's index readiness: a client
        // that happens to connect while a Vault is reindexing has a fully
        // set-up instance and needs the real instructions, not the first-run
        // ones (#191).
        let base = if self.state.startup.model_setup_pending() {
            SETUP_INSTRUCTIONS
        } else {
            SERVER_INSTRUCTIONS
        };
        // serverInfo.version is invisible to most agents (their harness eats
        // the handshake), so the version rides the instructions too.
        let instructions = format!(
            "{base} This instance runs Hatchdoor {}.",
            crate::config::version_string()
        );
        // The modern wire shape advertises `tools.listChanged: true` and
        // delivers on it via `subscriptions/listen` (#170). The legacy
        // handshake cannot open subscription streams, so `initialize`
        // below flips this back to an honest `false` for legacy sessions.
        let mut tools_capability = rmcp::model::ToolsCapability::default();
        tools_capability.list_changed = Some(true);
        let mut capabilities = ServerCapabilities::builder().enable_tools().build();
        capabilities.tools = Some(tools_capability);
        ServerInfo::new(capabilities)
            // Preferred revision for clients that request one we no longer
            // serve: the newest legacy revision, not the modern one.
            .with_protocol_version(rmcp::model::ProtocolVersion::V_2025_11_25)
            .with_server_info(Implementation::new(
                "hatchdoor",
                crate::config::version_string(),
            ))
            .with_instructions(instructions)
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [rmcp::model::ProtocolVersion]> {
        advertised_protocol_versions()
    }

    // Hatchdoor serves tools only. The rmcp defaults would answer these
    // families with empty lists; the hand-written adapter rejected them as
    // unknown methods, and that refusal is preserved here so clients get a
    // clear error instead of silently-empty results.
    async fn list_prompts(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListPromptsResult, ErrorData> {
        Err(ErrorData::method_not_found::<
            rmcp::model::ListPromptsRequestMethod,
        >())
    }

    async fn list_resources(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListResourcesResult, ErrorData> {
        Err(ErrorData::method_not_found::<
            rmcp::model::ListResourcesRequestMethod,
        >())
    }

    async fn list_resource_templates(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListResourceTemplatesResult, ErrorData> {
        Err(ErrorData::method_not_found::<
            rmcp::model::ListResourceTemplatesRequestMethod,
        >())
    }

    async fn read_resource(
        &self,
        request: rmcp::model::ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ReadResourceResponse, ErrorData> {
        let _ = request;
        Err(ErrorData::method_not_found::<
            rmcp::model::ReadResourceRequestMethod,
        >())
    }

    /// The modern `2026-07-28` lifecycle opener: replaces `initialize`, needs
    /// no session, and carries the same server information plus SEP-2549 cache
    /// metadata. rmcp validates the per-request `_meta`/header contract before
    /// dispatch reaches this method.
    /// The legacy `initialize` handshake. Replicates rmcp's default
    /// negotiation (a supported requested version wins; otherwise the server
    /// default stands) and then advertises `tools.listChanged` honestly for
    /// the negotiated revision: only the modern surface can deliver change
    /// events through `subscriptions/listen`, so a legacy session still sees
    /// `false` and keeps reissuing `tools/list`.
    async fn initialize(
        &self,
        request: rmcp::model::InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, ErrorData> {
        context.peer.set_peer_info(request.clone());
        let mut info = self.get_info();
        let supported = self.supported_protocol_versions();
        let negotiated = if supported.contains(&request.protocol_version) {
            request.protocol_version.clone()
        } else {
            info.protocol_version.clone()
        };
        if negotiated != ProtocolVersion::V_2026_07_28
            && let Some(tools_capability) = info.capabilities.tools.as_mut()
        {
            tools_capability.list_changed = Some(false);
        }
        info.protocol_version = negotiated;
        Ok(info)
    }

    async fn discover(
        &self,
        _context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::DiscoverResult, ErrorData> {
        Ok(rmcp::model::DiscoverResult::from_server_info(
            advertised_protocol_versions().into_owned(),
            self.get_info(),
        )
        .with_ttl_ms(LIST_CACHE_TTL_MS)
        .with_cache_scope(CacheScope::Private))
    }

    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let config = self.config().map_err(internal_config_error)?;
        let mut tools = tools::setup_tools_list();
        tools.extend(tools::tools_list(&config));
        Ok(
            ListToolsResult::with_all_items(tools.into_iter().map(value_to_tool).collect())
                .with_ttl_ms(LIST_CACHE_TTL_MS)
                .with_cache_scope(CacheScope::Private),
        )
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        self.record_agent_connection(&context);
        let mut config = self.config().map_err(internal_config_error)?;
        config.request_origin = context
            .extensions
            .get::<axum::http::request::Parts>()
            .and_then(request_origin);
        let params = json!({
            "name": request.name.as_ref(),
            "arguments": Value::from(request.arguments.unwrap_or_default()),
        });
        match tools::handle_tools_call(self.state.clone(), Some(params), &config).await {
            Ok(result) => Ok(tool_value_to_result(result)),
            Err(failure) => Err(dispatcher_failure_to_error_data(failure)),
        }
    }

    /// The subset of a client's `subscriptions/listen` filter Hatchdoor
    /// accepts (#170): tool-list change events only. The SDK intersects this
    /// with the request and with the advertised capabilities, so a client
    /// opting into other categories is acknowledged with those removed.
    fn accepted_subscription_filter(
        &self,
        _requested: &SubscriptionFilter,
    ) -> Option<SubscriptionFilter> {
        Some(SubscriptionFilter::builder().tools_list_changed().build())
    }

    /// One established subscription stream. Runs until the request is
    /// cancelled (client disconnect or explicit cancellation) or the server
    /// starts shutting down, which would otherwise wait on it forever (#353),
    /// and forwards
    /// each `mcp_tools_changed` broadcast as
    /// `notifications/tools/list_changed` carrying the subscription ID
    /// metadata rmcp attaches. A missed batch of events while lagged still
    /// produces one notification, telling the client its cached list is stale.
    async fn listen(&self, context: SubscriptionContext) -> Result<(), ErrorData> {
        // Attribute the subscription to the credential the transport
        // middleware validated; the marker is absent only for direct handler
        // tests, which then share one anonymous budget.
        let token = context
            .request_context()
            .extensions
            .get::<axum::http::request::Parts>()
            .and_then(|parts| parts.extensions.get::<McpBearerToken>())
            .map(|marker| marker.0.clone())
            .unwrap_or_else(|| Arc::from(""));
        let slot = self.subscriptions.try_acquire(&token).ok_or_else(|| {
            ErrorData::new(
                ErrorCode::INVALID_REQUEST,
                format!(
                    "maximum of {MAX_SUBSCRIPTIONS_PER_TOKEN} live subscriptions per bearer token",
                ),
                None,
            )
        })?;

        let mut tools_changed = self.state.mcp_tools_changed.subscribe();
        let shutdown = self.state.shutdown.clone();
        loop {
            tokio::select! {
                _ = context.cancelled() => break,
                () = shutdown.wait() => break,
                event = tools_changed.recv() => match event {
                    Ok(()) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        context.sink().notify_tool_list_changed().await.ok();
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                },
            }
        }
        drop(slot);
        Ok(())
    }
}

/// Convert one catalogue entry (the same JSON shape the old hand-written
/// `tools/list` produced) into rmcp's typed `Tool`.
fn value_to_tool(value: Value) -> Tool {
    let name = value["name"].as_str().unwrap_or_default().to_string();
    let description = value["description"].as_str().map(str::to_owned);
    let input_schema = Arc::new(
        value["inputSchema"]
            .as_object()
            .cloned()
            .expect("tool advertises an input schema"),
    );
    let output_schema = value["outputSchema"].as_object().cloned().map(Arc::new);
    let annotations = value
        .get("annotations")
        .cloned()
        .map(serde_json::from_value::<ToolAnnotations>)
        .transpose()
        .expect("annotations deserialize");
    Tool::new_with_raw(Cow::Owned(name), description.map(Cow::Owned), input_schema)
        .with_raw_output_schema(
            output_schema.expect("every advertised MCP tool has an output schema"),
        )
        .with_annotations(annotations.unwrap_or_default())
}

/// Map a finished dispatcher result (the old `tool_success`/`tool_error`/
/// `tool_structured_error` shapes) onto rmcp's typed result.
fn tool_value_to_result(value: Value) -> CallToolResponse {
    let is_error = matches!(value.get("isError"), Some(Value::Bool(true)));
    let structured_content = value.get("structuredContent").cloned();
    let text = value["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    match (is_error, structured_content) {
        (false, Some(payload)) => CallToolResult::structured(payload).into(),
        // The plain-error text is the human fallback; structured errors keep
        // the shared Vault error object so agents branch on `code`.
        (true, Some(payload)) => CallToolResult::structured_error(payload).into(),
        (_, None) => CallToolResult::error(vec![ContentBlock::text(text)]).into(),
    }
}

fn internal_config_error(message: String) -> ErrorData {
    error!(detail = %message, "Internal MCP error");
    ErrorData::new(ErrorCode::INTERNAL_ERROR, "Internal server error", None)
}

fn dispatcher_failure_to_error_data(failure: JsonRpcFailure) -> ErrorData {
    if failure.code == JsonRpcFailure::INTERNAL_ERROR_CODE {
        error!(detail = %failure.message, "Internal MCP error");
        return ErrorData::new(ErrorCode::INTERNAL_ERROR, "Internal server error", None);
    }
    ErrorData::new(ErrorCode(failure.code as i32), failure.message, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advertised_revisions_are_exactly_the_two_supported_ones() {
        let advertised = advertised_protocol_versions();
        let versions: Vec<&str> = advertised.iter().map(|version| version.as_str()).collect();
        assert_eq!(versions, super::super::config::SUPPORTED_PROTOCOL_VERSIONS);
    }

    fn parts(host: Option<&str>) -> axum::http::request::Parts {
        parts_with(host, &[])
    }

    fn parts_with(host: Option<&str>, headers: &[(&str, &str)]) -> axum::http::request::Parts {
        let mut request = axum::http::Request::builder().uri("/mcp");
        if let Some(host) = host {
            request = request.header(axum::http::header::HOST, host);
        }
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        request.body(()).expect("request").into_parts().0
    }

    fn origin(host: Option<&str>, headers: &[(&str, &str)]) -> Option<String> {
        super::request_origin(&parts_with(host, headers))
    }

    #[test]
    fn request_origin_is_the_host_the_request_arrived_on() {
        assert_eq!(
            super::request_origin(&parts(Some("127.0.0.1:42824"))),
            Some("http://127.0.0.1:42824".to_string())
        );
        assert_eq!(
            super::request_origin(&parts(Some("notes.lan"))),
            Some("http://notes.lan".to_string())
        );
        assert_eq!(super::request_origin(&parts(None)), None);
        assert_eq!(super::request_origin(&parts(Some("bad host/x"))), None);
        assert_eq!(super::request_origin(&parts(Some("user@evil"))), None);
    }

    #[test]
    fn x_forwarded_proto_sets_the_scheme_and_keeps_the_arriving_host() {
        assert_eq!(
            origin(Some("notes.lan"), &[("x-forwarded-proto", "https")]),
            Some("https://notes.lan".to_string())
        );
        assert_eq!(
            origin(Some("notes.lan"), &[("x-forwarded-proto", "https, http")]),
            Some("https://notes.lan".to_string())
        );
        assert_eq!(
            origin(Some("notes.lan"), &[("x-forwarded-proto", "HTTPS")]),
            Some("https://notes.lan".to_string())
        );
    }

    #[test]
    fn x_forwarded_host_alone_changes_the_host_and_keeps_http() {
        assert_eq!(
            origin(
                Some("127.0.0.1:42824"),
                &[("x-forwarded-host", "notes.example.com, proxy.lan")]
            ),
            Some("http://notes.example.com".to_string())
        );
    }

    #[test]
    fn forwarded_gives_scheme_and_host_and_wins_over_x_forwarded() {
        assert_eq!(
            origin(
                Some("127.0.0.1:42824"),
                &[(
                    "forwarded",
                    "for=1.2.3.4;proto=https;host=notes.example.com"
                )]
            ),
            Some("https://notes.example.com".to_string())
        );
        assert_eq!(
            origin(
                Some("127.0.0.1:42824"),
                &[
                    ("forwarded", "proto=https;host=notes.example.com"),
                    ("x-forwarded-proto", "http"),
                    ("x-forwarded-host", "other.example.com"),
                ]
            ),
            Some("https://notes.example.com".to_string())
        );
    }

    #[test]
    fn forwarded_reads_only_its_first_element_with_quotes_and_any_case() {
        assert_eq!(
            origin(
                Some("127.0.0.1"),
                &[(
                    "forwarded",
                    "For=\"[2001:db8::1]\";Proto=HTTPS;Host=\"notes.example.com:8443\", proto=http;host=inner.lan"
                )]
            ),
            Some("https://notes.example.com:8443".to_string())
        );
        // A quoted value may hold the separators; they do not end the element.
        assert_eq!(
            origin(
                Some("127.0.0.1"),
                &[("forwarded", "for=\"a,b;c\";host=notes.example.com")]
            ),
            Some("http://notes.example.com".to_string())
        );
    }

    #[test]
    fn forwarded_keeps_a_quoted_ipv6_host_and_its_port() {
        assert_eq!(
            origin(
                Some("127.0.0.1"),
                &[("forwarded", "proto=https;host=\"[2001:db8::1]:8443\"")]
            ),
            Some("https://[2001:db8::1]:8443".to_string())
        );
    }

    #[test]
    fn a_stray_quote_does_not_carry_the_parse_into_a_later_element() {
        // The quote is not at the start of a value, so it is an ordinary
        // character: the first element ends at the comma and has no host.
        assert_eq!(
            origin(
                Some("notes.lan"),
                &[("forwarded", "for=x\"y, proto=https;host=inner.example.com")]
            ),
            Some("http://notes.lan".to_string())
        );
    }

    #[test]
    fn scheme_and_host_fall_back_independently_between_header_families() {
        // Forwarded gives only the host; the scheme comes from X-Forwarded-Proto.
        assert_eq!(
            origin(
                Some("127.0.0.1"),
                &[
                    ("forwarded", "host=notes.example.com"),
                    ("x-forwarded-proto", "https"),
                ]
            ),
            Some("https://notes.example.com".to_string())
        );
    }

    #[test]
    fn unusable_forwarded_values_fall_back_to_the_next_source() {
        assert_eq!(
            origin(
                Some("notes.lan"),
                &[("forwarded", "proto=ftp"), ("x-forwarded-proto", "https")]
            ),
            Some("https://notes.lan".to_string())
        );
        assert_eq!(
            origin(Some("notes.lan"), &[("x-forwarded-proto", "ftp")]),
            Some("http://notes.lan".to_string())
        );
        assert_eq!(
            origin(Some("notes.lan"), &[("x-forwarded-host", "user@evil")]),
            Some("http://notes.lan".to_string())
        );
        assert_eq!(
            origin(Some("notes.lan"), &[("x-forwarded-host", "bad host/x")]),
            Some("http://notes.lan".to_string())
        );
        assert_eq!(
            origin(
                Some("notes.lan"),
                &[
                    ("forwarded", "host=\"user@evil\""),
                    ("x-forwarded-host", "notes.example.com")
                ]
            ),
            Some("http://notes.example.com".to_string())
        );
        assert_eq!(
            origin(Some("notes.lan"), &[("forwarded", ";;=;garbage\"")]),
            Some("http://notes.lan".to_string())
        );
        // A forwarded scheme with no usable host anywhere still has nothing to
        // build a link on.
        assert_eq!(origin(None, &[("x-forwarded-proto", "https")]), None);
    }

    #[test]
    fn retired_revisions_are_not_negotiated() {
        assert!(!super::super::config::is_supported_protocol_version(
            "2025-03-26"
        ));
        assert!(!super::super::config::is_supported_protocol_version(
            "2025-06-18"
        ));
        assert!(!super::super::config::is_supported_protocol_version(
            "2024-11-05"
        ));
    }

    #[test]
    fn tool_value_round_trips_through_typed_result() {
        let success = json!({
            "content": [{"type": "text", "text": "{\"ok\":true}"}],
            "structuredContent": {"ok": true},
            "isError": false
        });
        let rmcp::model::CallToolResponse::Complete(typed) = tool_value_to_result(success) else {
            panic!("success maps to a complete result");
        };
        assert_eq!(typed.is_error, Some(false));
        assert_eq!(
            typed.structured_content,
            Some(json!({"ok": true})),
            "structured errors keep the shared Vault error object"
        );

        let structured_error = json!({
            "content": [{"type": "text", "text": "{\"code\":\"vault_read_unavailable\"}"}],
            "structuredContent": {"code": "vault_read_unavailable"},
            "isError": true
        });
        let rmcp::model::CallToolResponse::Complete(typed) = tool_value_to_result(structured_error)
        else {
            panic!("tool errors map to complete results");
        };
        assert_eq!(typed.is_error, Some(true));
        assert_eq!(
            typed.structured_content,
            Some(json!({"code": "vault_read_unavailable"}))
        );
    }

    #[test]
    fn internal_failures_are_masked_behind_the_stable_protocol_error() {
        // -32603 internals must reach the log with their diagnostic detail but
        // surface only the stable masked message (#172 error-semantics leg).
        let masked = dispatcher_failure_to_error_data(JsonRpcFailure::internal(
            "diagnostic: vault path /srv/leaked/vault read failed",
        ));
        assert_eq!(
            masked.code,
            rmcp::model::ErrorCode(JsonRpcFailure::INTERNAL_ERROR_CODE as i32)
        );
        assert_eq!(masked.message, "Internal server error");
        assert!(
            masked.data.is_none(),
            "no diagnostics leak into the payload"
        );

        let config_failure =
            internal_config_error("diagnostic: HATCHDOOR_MCP_ENABLED missing".to_string());
        assert_eq!(
            config_failure.code,
            rmcp::model::ErrorCode(JsonRpcFailure::INTERNAL_ERROR_CODE as i32)
        );
        assert_eq!(config_failure.message, "Internal server error");
        assert!(config_failure.data.is_none());
    }
}
