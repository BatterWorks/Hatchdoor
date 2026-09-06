//! Shared MCP result/error vocabulary. JSON-RPC framing itself is rmcp-owned
//! (ADR-17); what remains here are the tool-result shapes the dispatcher and
//! the typed adapter share, plus the bounded HTTP error bodies our transport
//! middleware returns before any rmcp framing exists (auth failures, request
//! size limits).

use axum::body::Body;
use axum::http::{StatusCode, header};
use axum::response::Response;
use serde_json::{Value, json};
use std::io::Write;

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
    /// The code whose dispatcher-level rendering masks diagnostics behind the
    /// stable `Internal server error` message.
    pub const INTERNAL_ERROR_CODE: i64 = -32603;

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

/// A plain-JSON HTTP error response with a JSON-RPC-shaped body. Used only by
/// the authorization/limit middleware for pre-framing rejections; everything
/// past that gate is framed by rmcp.
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

pub(crate) struct LimitedJsonWriter {
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

pub const MAX_JSONRPC_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

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

/// The field that states a tool payload's outcome: `false` on the structured
/// errors [`tool_structured_error`] builds, `true` on the write receipts
/// `results.rs` serialises. Read payloads carry it on neither, so the rule for
/// a caller is that the field present and false means the call was refused.
///
/// It exists because a client reading `structuredContent` as the tool's typed
/// answer never sees the enclosing `isError`. Every tool advertises an
/// `outputSchema` built from its success type, which invites exactly that
/// reading, and on failure the payload silently became a different shape with
/// no field in common with the advertised one (#255).
pub const OUTCOME_FIELD: &str = "ok";

/// A domain failure returned by a tool.  Unlike a JSON-RPC invalid-params
/// error, this preserves the shared Vault API's stable error object so agents
/// can branch on `code` rather than matching human text.
///
/// The payload also carries [`OUTCOME_FIELD`] set to `false`, alongside the
/// error object's own `code`, `message`, `retryable`, and any `vault_id`. This
/// is the single construction point for the shape, so no individual tool has to
/// remember to set it, and the field goes in before the text rendering so the
/// two halves of the result cannot disagree. It is deliberately added here
/// rather than on the shared Vault error type itself: that type also serialises
/// into HTTP bodies and into `batch` item `error` values, neither of which
/// changes shape.
///
/// A payload that is not a JSON object keeps whatever it already was, and a
/// tool error carrying only a plain-text message ([`tool_error`]) has no
/// structured payload to mark at all.
pub fn tool_structured_error(mut payload: Value) -> Value {
    if let Some(object) = payload.as_object_mut() {
        object.insert(OUTCOME_FIELD.to_string(), json!(false));
    }
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
    fn a_structured_error_marks_itself_as_a_failure_in_both_renderings() {
        let result = tool_structured_error(json!({
            "code": "write_conflict",
            "message": "note changed since it was read",
            "retryable": true,
        }));

        assert_eq!(result["isError"], true);
        let payload = &result["structuredContent"];
        assert_eq!(payload[OUTCOME_FIELD], false);
        // The error object's own fields survive untouched: an agent still
        // branches on `code` rather than on human text.
        assert_eq!(payload["code"], "write_conflict");
        assert_eq!(payload["retryable"], true);

        let text: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().expect("text"))
                .expect("the text content is a serialisation of the payload");
        assert_eq!(&text, payload);
    }

    #[test]
    fn a_payload_that_is_not_an_object_is_left_alone() {
        let result = tool_structured_error(json!("just a string"));
        assert_eq!(result["structuredContent"], json!("just a string"));
        assert_eq!(result["isError"], true);
    }

    #[test]
    fn a_success_payload_gains_nothing() {
        let result = tool_success(json!({"ok": true, "slug": "home"}));
        assert_eq!(result["isError"], false);
        assert_eq!(
            result["structuredContent"],
            json!({"ok": true, "slug": "home"})
        );
    }
}
