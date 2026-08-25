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
}

impl ServerHandler for HatchdoorMcpHandler {
    fn get_info(&self) -> ServerInfo {
        let instructions = if self.state.startup.is_ready() {
            SERVER_INSTRUCTIONS.to_string()
        } else {
            SETUP_INSTRUCTIONS.to_string()
        };
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
            .with_server_info(Implementation::new("hatchdoor", env!("CARGO_PKG_VERSION")))
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
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let config = self.config().map_err(internal_config_error)?;
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
    /// cancelled (client disconnect or explicit cancellation) and forwards
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
        loop {
            tokio::select! {
                _ = context.cancelled() => break,
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
