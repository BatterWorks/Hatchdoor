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
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ErrorCode, ErrorData,
    Implementation, ListToolsResult, ServerCapabilities, ServerInfo, Tool, ToolAnnotations,
};
use rmcp::service::RequestContext;
use rmcp::{RoleServer, ServerHandler};
use serde_json::{Value, json};
use tracing::error;

use super::config::{McpConfig, SERVER_INSTRUCTIONS, SETUP_INSTRUCTIONS};
use super::protocol::JsonRpcFailure;
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

pub struct HatchdoorMcpHandler {
    state: AppState,
}

impl HatchdoorMcpHandler {
    pub fn new(state: AppState) -> Self {
        Self { state }
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
        // The legacy wire shape advertises `tools.listChanged: false`
        // explicitly (the POST-only flow cannot deliver list-change events).
        let mut tools_capability = rmcp::model::ToolsCapability::default();
        tools_capability.list_changed = Some(false);
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

    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let config = self.config().map_err(internal_config_error)?;
        let mut tools = tools::setup_tools_list();
        tools.extend(tools::tools_list(&config));
        Ok(ListToolsResult::with_all_items(
            tools.into_iter().map(value_to_tool).collect(),
        ))
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
}
