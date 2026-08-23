use std::sync::Arc;

#[cfg(test)]
use std::cell::Cell;

use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use base64::Engine;

use crate::api_types::ErrorResponse;

#[cfg(test)]
thread_local! {
    static COMPARISON_WORK: Cell<usize> = const { Cell::new(0) };
}

#[cfg(test)]
fn reset_comparison_work() {
    COMPARISON_WORK.with(|work| work.set(0));
}

#[cfg(test)]
fn comparison_work() -> usize {
    COMPARISON_WORK.with(Cell::get)
}

#[cfg(test)]
fn record_comparison_work() {
    COMPARISON_WORK.with(|work| work.set(work.get() + 1));
}

/// Compare two bearer credentials without short-circuiting. Configuration
/// permits arbitrary token strings, so first derive fixed-size BLAKE3
/// representations before doing the fixed-width comparison.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let a = blake3::hash(a);
    let b = blake3::hash(b);
    let mut diff = 0u8;
    for (x, y) in a.as_bytes().iter().zip(b.as_bytes()) {
        #[cfg(test)]
        record_comparison_work();
        diff |= x ^ y;
    }
    diff == 0
}

/// Produce a URL-safe, 256-bit bearer token. Callers decide whether a token is
/// merely a candidate or should be persisted as configuration.
pub(crate) fn generate_bearer_token() -> Result<String, String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|error| format!("could not generate a bearer token: {error}"))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

/// Token required to access protected web routes, shared into the auth layer.
#[derive(Clone)]
pub struct WebToken(pub Arc<str>);

/// Middleware enforcing the web bearer token on protected routes. The token may
/// arrive as an `Authorization: Bearer <token>` header or, for `<img>`/download
/// navigations that cannot set headers, an `access_token` query parameter.
pub async fn require_web_token(
    State(token): State<WebToken>,
    request: Request,
    next: Next,
) -> Response {
    if request_is_authorized(&request, token.0.as_bytes()) {
        next.run(request).await
    } else {
        unauthorized()
    }
}

fn request_is_authorized(request: &Request, expected: &[u8]) -> bool {
    if let Some(presented) = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        && constant_time_eq(presented.as_bytes(), expected)
    {
        return true;
    }

    if let Some(presented) = request.uri().query().and_then(access_token_from_query)
        && constant_time_eq(presented.as_bytes(), expected)
    {
        return true;
    }

    false
}

/// Rewrite the `access_token` value in a query string to `REDACTED`. The web
/// token can ride in the query for `<img>`/download navigations, and the request
/// trace span logs the full URI (at debug level), so the raw token must never
/// reach the span. Other query parameters are preserved.
pub fn redact_query_token(query: &str) -> String {
    query
        .split('&')
        .map(|pair| match pair.split_once('=') {
            Some(("access_token", _)) => "access_token=REDACTED".to_string(),
            _ => pair.to_string(),
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// Tokens accepted by the attachment route. The web credential is fixed at
/// startup, while the MCP credential is read from a live snapshot per request
/// so an operator can rotate it without leaving the old one authorized.
#[derive(Clone)]
pub(crate) struct WebOrLiveMcpToken {
    pub(crate) web: Option<Arc<str>>,
    pub(crate) runtime_config: crate::runtime_config::RuntimeConfig,
}

/// Middleware enforcing either the web bearer token or the current MCP bearer
/// token. Unlike [`require_web_token`], this does not fall back to an
/// `access_token` query parameter — the routes it guards are hit by
/// out-of-band HTTP clients (e.g. `curl`), not `<img>`/download navigations.
///
/// Accepts a matching MCP bearer token only while MCP and MCP writes are both
/// enabled. This keeps runtime disablement as an immediate revocation of that
/// credential's attachment-write capability. A matching web bearer token is
/// independent of MCP write mode.
pub(crate) async fn require_web_or_live_mcp_token(
    State(tokens): State<WebOrLiveMcpToken>,
    request: Request,
    next: Next,
) -> Response {
    let presented = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));

    let mcp = match crate::mcp::McpConfig::from_snapshot(&tokens.runtime_config.snapshot()) {
        Ok(config) => config,
        // A malformed runtime configuration must never turn this security gate
        // into an unauthenticated attachment route.
        Err(_) => return unauthorized(),
    };
    let configured = tokens.web.is_some() || mcp.bearer_token.is_some();
    let matches_web = presented.is_some_and(|presented| {
        tokens
            .web
            .as_deref()
            .is_some_and(|expected| constant_time_eq(presented.as_bytes(), expected.as_bytes()))
    });
    let matches_mcp = presented.is_some_and(|presented| {
        mcp.enabled
            && mcp
                .bearer_token
                .as_deref()
                .is_some_and(|expected| constant_time_eq(presented.as_bytes(), expected.as_bytes()))
    });

    if !configured || matches_web || (matches_mcp && mcp.write_enabled) {
        next.run(request).await
    } else if matches_mcp {
        forbidden()
    } else {
        unauthorized()
    }
}

fn access_token_from_query(query: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        if key == "access_token" {
            Some(percent_decode(value))
        } else {
            None
        }
    })
}

/// Minimal percent-decoding for the query token (handles `%XX` and `+`).
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hi = (bytes[i + 1] as char).to_digit(16);
                let lo = (bytes[i + 2] as char).to_digit(16);
                if let (Some(hi), Some(lo)) = (hi, lo) {
                    out.push((hi * 16 + lo) as u8);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Bearer")],
        axum::Json(ErrorResponse {
            error: "Unauthorized".to_string(),
        }),
    )
        .into_response()
}

fn forbidden() -> Response {
    (
        StatusCode::FORBIDDEN,
        axum::Json(ErrorResponse {
            error: "MCP writes are disabled".to_string(),
        }),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_matches_only_identical_bytes() {
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secret", b"secrf t"));
        assert!(!constant_time_eq(b"secret", b"secre"));
        assert!(!constant_time_eq(b"", b"x"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn bearer_comparison_uses_fixed_work_for_equal_and_unequal_lengths() {
        let expected = b"an arbitrary configured bearer token";
        let attempts: [(&[u8], bool); 3] = [
            (expected, true),
            (b"an arbitrary configured bearer tokem", false),
            (b"wrong", false),
        ];

        let mut observed_work = Vec::new();
        for (presented, accepted) in attempts {
            reset_comparison_work();
            assert_eq!(constant_time_eq(presented, expected), accepted);
            observed_work.push(comparison_work());
        }

        assert_eq!(observed_work, vec![32, 32, 32]);
    }

    #[test]
    fn web_bearer_authorization_keeps_header_and_query_credentials_strict() {
        let expected = b"an arbitrary configured bearer token";
        for (uri, header, accepted) in [
            (
                "/protected",
                Some("Bearer an arbitrary configured bearer token"),
                true,
            ),
            (
                "/protected",
                Some("Bearer an arbitrary configured bearer tokem"),
                false,
            ),
            ("/protected", Some("Bearer wrong"), false),
            (
                "/protected?access_token=an%20arbitrary%20configured%20bearer%20token",
                None,
                true,
            ),
        ] {
            let mut request = Request::builder().uri(uri);
            if let Some(header) = header {
                request = request.header(header::AUTHORIZATION, header);
            }
            let request = request.body(axum::body::Body::empty()).expect("request");
            assert_eq!(request_is_authorized(&request, expected), accepted, "{uri}");
        }
    }

    #[test]
    fn redact_query_token_hides_only_the_access_token() {
        assert_eq!(
            redact_query_token("foo=1&access_token=super-secret&bar=2"),
            "foo=1&access_token=REDACTED&bar=2"
        );
        assert_eq!(redact_query_token("foo=1"), "foo=1");
        assert_eq!(
            redact_query_token("access_token=x"),
            "access_token=REDACTED"
        );
    }

    #[test]
    fn access_token_from_query_extracts_and_decodes() {
        assert_eq!(
            access_token_from_query("foo=1&access_token=a%20b&bar=2").as_deref(),
            Some("a b")
        );
        assert_eq!(access_token_from_query("foo=1").as_deref(), None);
    }
}
