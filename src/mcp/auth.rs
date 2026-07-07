//! Per-request MCP authorization: bearer-token check, Origin allow-listing
//! (anti-DNS-rebinding), and protocol-version validation. This is the security
//! gate the `/mcp` transport runs before dispatching any tool; it is distinct
//! from the static configuration parsing in `config`.

use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::Value;

use crate::mcp::protocol::jsonrpc_error_response;

use super::config::{McpConfig, is_supported_protocol_version};

pub fn validate_mcp_request(headers: &HeaderMap, config: &McpConfig) -> Result<(), Box<Response>> {
    if !config.enabled {
        return Err(Box::new(StatusCode::NOT_FOUND.into_response()));
    }

    if config.bearer_token.is_none() {
        return Err(Box::new(jsonrpc_error_response(
            StatusCode::UNAUTHORIZED,
            Value::Null,
            -32001,
            "MCP requires HATCHDOOR_MCP_BEARER_TOKEN".to_string(),
        )));
    }

    if let Some(origin) = headers.get(header::ORIGIN).and_then(header_to_str)
        && !is_allowed_origin(origin, &config.allowed_origins)
    {
        return Err(Box::new(jsonrpc_error_response(
            StatusCode::FORBIDDEN,
            Value::Null,
            -32000,
            "Forbidden MCP origin".to_string(),
        )));
    }

    if let Some(expected_token) = &config.bearer_token {
        let expected = format!("Bearer {expected_token}");
        let authorized = headers
            .get(header::AUTHORIZATION)
            .and_then(header_to_str)
            .map(|value| crate::auth::constant_time_eq(value.as_bytes(), expected.as_bytes()))
            .unwrap_or(false);
        if !authorized {
            return Err(Box::new(jsonrpc_error_response(
                StatusCode::UNAUTHORIZED,
                Value::Null,
                -32001,
                "Missing or invalid MCP bearer token".to_string(),
            )));
        }
    }

    if let Some(protocol_version) = headers
        .get("MCP-Protocol-Version")
        .or_else(|| headers.get("Mcp-Protocol-Version"))
        .and_then(header_to_str)
        && !is_supported_protocol_version(protocol_version)
    {
        return Err(Box::new(jsonrpc_error_response(
            StatusCode::BAD_REQUEST,
            Value::Null,
            -32002,
            format!("Unsupported MCP protocol version: {protocol_version}"),
        )));
    }

    Ok(())
}

fn header_to_str(value: &HeaderValue) -> Option<&str> {
    value.to_str().ok()
}

pub fn is_allowed_origin(origin: &str, allowed_origins: &[String]) -> bool {
    let origin = origin.trim().trim_end_matches('/');
    allowed_origins
        .iter()
        .map(|allowed| allowed.trim().trim_end_matches('/'))
        .any(|allowed| origin_matches_allowed(origin, allowed))
}

pub fn origin_matches_allowed(origin: &str, allowed: &str) -> bool {
    if origin == allowed {
        return true;
    }

    let Some((scheme, host)) = allowed.split_once("://") else {
        return false;
    };

    if !matches!(host, "localhost" | "127.0.0.1" | "[::1]") {
        return false;
    }

    let with_port_prefix = format!("{scheme}://{host}:");
    origin
        .strip_prefix(&with_port_prefix)
        .map(|port| !port.is_empty() && port.chars().all(|ch| ch.is_ascii_digit()))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::config::McpConfig;

    #[test]
    fn validate_mcp_request_rejects_read_only_without_token() {
        let mut config = McpConfig::disabled();
        config.enabled = true;
        config.write_enabled = false;
        config.bearer_token = None;

        let result = validate_mcp_request(&HeaderMap::new(), &config);
        let response = *result.expect_err("read-only MCP without a token must be rejected");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn origin_matching_allows_only_exact_or_local_port_variants() {
        assert!(origin_matches_allowed(
            "http://127.0.0.1:42824",
            "http://127.0.0.1"
        ));
        assert!(origin_matches_allowed(
            "http://localhost:5173",
            "http://localhost"
        ));
        assert!(origin_matches_allowed(
            "https://app.example",
            "https://app.example"
        ));
        assert!(!origin_matches_allowed(
            "https://app.example:443",
            "https://app.example"
        ));
        assert!(!origin_matches_allowed(
            "https://evil.example",
            "https://app.example"
        ));
    }
}
