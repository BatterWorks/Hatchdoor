use axum::body::Body;
use axum::http::{StatusCode, header};
use axum::response::Response;
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::Write;

/// MCP replies are JSON, not a bulk-transfer channel. Keep a hard ceiling so a
/// broad tree/query result cannot allocate an unbounded serialized response.
pub const MAX_JSONRPC_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

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
    jsonrpc_response(
        StatusCode::OK,
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result,
        }),
    )
}

pub fn jsonrpc_error_response(
    status: StatusCode,
    id: Value,
    code: i64,
    message: String,
) -> Response {
    jsonrpc_response(
        status,
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": code,
                "message": message,
            }
        }),
    )
}

fn jsonrpc_response(status: StatusCode, payload: Value) -> Response {
    match bounded_json_bytes(&payload) {
        Ok(body) => {
            let mut response = Response::new(Body::from(body));
            *response.status_mut() = status;
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/json"),
            );
            response
        }
        Err(_) => {
            let fallback = br#"{"jsonrpc":"2.0","id":null,"error":{"code":-32603,"message":"MCP response exceeds the server response size limit"}}"#;
            let mut response = Response::new(Body::from(&fallback[..]));
            *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/json"),
            );
            response
        }
    }
}

fn bounded_json_bytes(payload: &Value) -> Result<Vec<u8>, serde_json::Error> {
    let mut writer = LimitedJsonWriter::new(MAX_JSONRPC_RESPONSE_BYTES);
    serde_json::to_writer(&mut writer, payload)?;
    Ok(writer.into_inner())
}

struct LimitedJsonWriter {
    bytes: Vec<u8>,
    maximum: usize,
}

impl LimitedJsonWriter {
    fn new(maximum: usize) -> Self {
        Self {
            bytes: Vec::new(),
            maximum,
        }
    }

    fn into_inner(self) -> Vec<u8> {
        self.bytes
    }
}

impl Write for LimitedJsonWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let next_len = self.bytes.len().checked_add(bytes.len()).ok_or_else(|| {
            std::io::Error::other("MCP response exceeds the server response size limit")
        })?;
        if next_len > self.maximum {
            return Err(std::io::Error::other(
                "MCP response exceeds the server response size limit",
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
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
    let text = match bounded_json_bytes(&payload) {
        Ok(bytes) => String::from_utf8(bytes).expect("JSON serialization is valid UTF-8"),
        Err(_) => {
            return tool_error("Tool response exceeds the server response size limit".to_string());
        }
    };
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

    #[test]
    fn oversized_success_response_is_replaced_with_a_bounded_error() {
        let response = jsonrpc_success_response(
            Value::from(1),
            json!({
                "payload": "x".repeat(MAX_JSONRPC_RESPONSE_BYTES)
            }),
        );
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
