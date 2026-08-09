use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: Option<String>,
    pub id: Option<Value>,
    pub method: String,
    pub params: Option<Value>,
}

#[derive(Debug)]
pub struct JsonRpcFailure {
    pub code: i64,
    pub message: String,
    /// When true, the dispatcher renders this as an `isError` tool result rather
    /// than a JSON-RPC protocol error — used for conditions like "note not found"
    /// that read tools already surface as tool errors, so both stay consistent.
    pub tool_level: bool,
}

impl JsonRpcFailure {
    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self {
            code: -32602,
            message: message.into(),
            tool_level: false,
        }
    }

    pub fn method_not_found(message: impl Into<String>) -> Self {
        Self {
            code: -32601,
            message: message.into(),
            tool_level: false,
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            code: -32603,
            message: message.into(),
            tool_level: false,
        }
    }

    /// A "not found" failure that read and write tools both surface as an
    /// `isError` tool result (not a protocol error).
    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            code: -32602,
            message: message.into(),
            tool_level: true,
        }
    }
}

pub fn jsonrpc_success_response(id: Value, result: Value) -> Response {
    (
        StatusCode::OK,
        Json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result,
        })),
    )
        .into_response()
}

pub fn jsonrpc_error_response(
    status: StatusCode,
    id: Value,
    code: i64,
    message: String,
) -> Response {
    (
        status,
        Json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": code,
                "message": message,
            }
        })),
    )
        .into_response()
}

/// The `notifications/tools/list_changed` JSON-RPC notification (no `id`), sent
/// to tell a client its cached tool list is stale and it should re-`tools/list`.
/// Built here so the shape is defined once; a streaming MCP transport writes it
/// to the client when `AppState::mcp_tools_changed` fires.
pub fn tools_list_changed_notification() -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "notifications/tools/list_changed",
    })
}

pub fn tool_success(payload: Value) -> Value {
    let text = serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string());
    json!({
        "content": [
            {
                "type": "text",
                "text": text
            }
        ],
        "structuredContent": payload,
        "isError": false
    })
}

pub fn tool_error(message: String) -> Value {
    json!({
        "content": [
            {
                "type": "text",
                "text": message
            }
        ],
        "isError": true
    })
}

/// A domain failure returned by a tool.  Unlike a JSON-RPC invalid-params
/// error, this preserves the shared Vault API's stable error object so agents
/// can branch on `code` rather than matching human text.
pub fn tool_structured_error(payload: Value) -> Value {
    let text = serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string());
    json!({
        "content": [{ "type": "text", "text": text }],
        "structuredContent": payload,
        "isError": true
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_list_changed_notification_is_an_idless_jsonrpc_notification() {
        let notification = tools_list_changed_notification();
        assert_eq!(notification["jsonrpc"], "2.0");
        assert_eq!(notification["method"], "notifications/tools/list_changed");
        // A notification carries no id (it expects no response) and no params.
        assert!(notification.get("id").is_none());
        assert!(notification.get("params").is_none());
    }
}
