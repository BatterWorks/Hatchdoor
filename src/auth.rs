use std::sync::Arc;

#[cfg(test)]
use std::cell::Cell;

use axum::extract::{Request, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
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

/// The credential an `Authorization` header carries under the Bearer scheme.
/// The scheme name is matched case-insensitively (RFC 7235 section 2.1) and
/// any run of spaces or tabs may separate it from the credential, so `bearer
/// <token>` and `BEARER  <token>` both present `<token>`. The credential itself
/// is returned as sent, for a constant-time comparison by the caller.
pub(crate) fn bearer_credential(value: &str) -> Option<&str> {
    let is_space = |ch: char| ch == ' ' || ch == '\t';
    let (scheme, rest) = value.trim_matches(is_space).split_once(is_space)?;
    if !scheme.eq_ignore_ascii_case("Bearer") {
        return None;
    }
    let credential = rest.trim_start_matches(is_space);
    (!credential.is_empty()).then_some(credential)
}

/// The Bearer credential in a request's `Authorization` header, if any.
pub(crate) fn presented_bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(bearer_credential)
}

/// Token required to access protected web routes, shared into the auth layer.
#[derive(Clone)]
pub struct WebToken(pub Arc<str>);

/// The routes that take the web token as an `access_token` query parameter
/// (ADR-08), for a `GET` only: the ones the browser reaches through an `<img>`,
/// a download link or an `EventSource`, none of which can send a header. Every
/// other route takes the header alone, so a URL that leaks with the token in it
/// can fetch a file or follow the event stream but cannot write or reveal a
/// token (#703). `{name}` matches one non-empty path segment and `{*name}` one
/// or more. A test holds every `withAccessToken` call in the frontend to this
/// list.
pub(crate) const QUERY_TOKEN_ROUTES: [&str; 3] = [
    "/api/v1/vaults/{vault_id}/assets/{*path}",
    "/api/v1/vaults/{vault_id}/notes/{slug}/download",
    "/api/v1/vaults/events",
];

/// Whether a request may present the web token in its query string.
fn query_token_allowed(method: &Method, path: &str) -> bool {
    method == Method::GET
        && QUERY_TOKEN_ROUTES
            .iter()
            .any(|route| route_matches(route, path))
}

/// Whether `path` is an address of `route`, a pattern in
/// [`QUERY_TOKEN_ROUTES`]' notation. Compares the raw, still percent-encoded
/// segments, the same ones the router matched the request on.
fn route_matches(route: &str, path: &str) -> bool {
    let mut segments = path.split('/');
    for expected in route.split('/') {
        if expected.starts_with("{*") {
            let rest: Vec<&str> = segments.by_ref().collect();
            return !rest.is_empty() && rest.iter().all(|segment| !segment.is_empty());
        }
        let Some(segment) = segments.next() else {
            return false;
        };
        let matches = if expected.starts_with('{') {
            !segment.is_empty()
        } else {
            segment == expected
        };
        if !matches {
            return false;
        }
    }
    segments.next().is_none()
}

/// Middleware enforcing the web bearer token on protected routes. The token
/// arrives as an `Authorization: Bearer <token>` header, or as an
/// `access_token` query parameter on the [`QUERY_TOKEN_ROUTES`] alone.
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

pub(crate) fn request_is_authorized(request: &Request, expected: &[u8]) -> bool {
    if let Some(presented) = presented_bearer(request.headers())
        && constant_time_eq(presented.as_bytes(), expected)
    {
        return true;
    }

    if query_token_allowed(request.method(), request.uri().path())
        && let Some(presented) = request.uri().query().and_then(access_token_from_query)
        && constant_time_eq(presented.as_bytes(), expected)
    {
        return true;
    }

    false
}

/// Rewrite every credential in a query string to `REDACTED`: the web token's
/// `access_token`, which rides in the query for `<img>`/download navigations,
/// and a transfer link's `signature` (ADR-27). The request trace span logs the
/// full URI (at debug level), so neither may reach the span. Other query
/// parameters are preserved.
pub fn redact_query_token(query: &str) -> String {
    query
        .split('&')
        .map(|pair| match pair.split_once('=') {
            Some((key @ ("access_token" | crate::transfer_link::SIGNATURE_PARAM), _)) => {
                format!("{key}=REDACTED")
            }
            _ => pair.to_string(),
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// Tokens accepted by the attachment routes, inbound and outbound. The web
/// credential is fixed at startup, while the MCP credential is read from a
/// live snapshot per request so an operator can rotate it without leaving the
/// old one authorized.
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
///
/// A deployment with no web token configured serves the route openly, exactly
/// as the rest of the web API and the asset *read* route do (#327). The MCP
/// credential can only ever add access to a gated route, never start demanding
/// one: the browser's paste-to-upload flow has no MCP token to send, so letting
/// a live MCP token turn this gate on would break it the moment an operator set
/// MCP up.
pub(crate) async fn require_web_or_live_mcp_token(
    State(tokens): State<WebOrLiveMcpToken>,
    request: Request,
    next: Next,
) -> Response {
    if tokens.web.is_none() {
        return next.run(request).await;
    }
    let presented = presented_bearer(request.headers());

    let mcp = match crate::mcp::McpConfig::from_snapshot(&tokens.runtime_config.snapshot()) {
        Ok(config) => config,
        // A malformed runtime configuration must never turn this security gate
        // into an unauthenticated attachment route.
        Err(_) => return unauthorized(),
    };
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

    if matches_web || (matches_mcp && mcp.write_enabled) {
        next.run(request).await
    } else if matches_mcp {
        forbidden()
    } else {
        unauthorized()
    }
}

/// State for the asset-read gate: the web credential, the live snapshot the MCP
/// credential is read from, and the `/mcp` transport's own rate limiter, shared
/// so an MCP-authenticated asset read spends the same budget a tool call does.
#[derive(Clone)]
pub struct AssetReadTokens {
    pub(crate) web: Arc<str>,
    pub(crate) runtime_config: crate::runtime_config::RuntimeConfig,
    pub(crate) limiter: Arc<crate::mcp::limits::RateLimiter>,
}

/// Marks a request admitted on the MCP bearer token rather than the web one, and
/// carries the byte ceiling that credential is allowed to read through this
/// route. Absent on a web-token request, which keeps the route's own larger
/// bound.
#[derive(Clone, Copy)]
pub struct McpAssetRead {
    pub(crate) max_bytes: u64,
}

/// Middleware enforcing either the web bearer token or the current MCP bearer
/// token on the asset *read* route, which `get_attachment`'s default
/// `download_url` points at (#176).
///
/// Three differences from [`require_web_or_live_mcp_token`], all following from
/// this being a read rather than a write:
///
/// - The web token may still arrive as an `access_token` query parameter, because
///   the browser reaches this route through `<img>` tags and download
///   navigations that cannot set headers. The MCP token is header-only: an MCP
///   client is always an out-of-band HTTP client, and keeping the credential out
///   of the query string keeps it out of the request trace span.
/// - A matching MCP bearer token is accepted whenever MCP is enabled, without
///   requiring write mode. Disabling MCP is still an immediate revocation,
///   checked per request.
/// - Admission on the MCP token is *not* equivalent to admission on the web
///   token, and the two guards below are what make the difference safe. An MCP
///   client reading attachment bytes over `/mcp` is bounded by
///   `HATCHDOOR_MCP_MAX_BASE64_BYTES` (and, inline, by what one reply can carry,
///   #632) and by the #171 tool quota and concurrency
///   caps; this route's own bound is far larger (64 MiB) and it sits outside the
///   `/mcp` transport entirely. Left alone, the `download_url` would hand that
///   credential a way around both limits. So an MCP-admitted request spends the
///   same quota and concurrency budget a tool call spends, and carries
///   [`McpAssetRead`] so the handler applies the same byte ceiling.
pub(crate) async fn require_web_or_live_mcp_read_token(
    State(tokens): State<AssetReadTokens>,
    mut request: Request,
    next: Next,
) -> Response {
    let mcp = match crate::mcp::McpConfig::from_snapshot(&tokens.runtime_config.snapshot()) {
        Ok(config) => config,
        // A malformed runtime configuration must never turn this security gate
        // into an unauthenticated asset route.
        Err(_) => return unauthorized(),
    };

    if request_is_authorized(&request, tokens.web.as_bytes()) {
        return next.run(request).await;
    }

    let presented = presented_bearer(request.headers()).map(str::to_string);
    let matches_mcp = presented.as_deref().is_some_and(|presented| {
        mcp.enabled
            && mcp
                .bearer_token
                .as_deref()
                .is_some_and(|expected| constant_time_eq(presented.as_bytes(), expected.as_bytes()))
    });
    if !matches_mcp {
        return unauthorized();
    }

    request.extensions_mut().insert(McpAssetRead {
        max_bytes: mcp.max_base64_bytes,
    });

    if !mcp.rate_limits_enabled {
        return next.run(request).await;
    }

    let token = crate::mcp::subscriptions::McpBearerToken(Arc::from(
        presented.expect("an MCP match presented a token"),
    ));
    let guard = match tokens.limiter.admit_tool_call(&token).await {
        Ok(guard) => guard,
        Err(retry_in) => return too_many_requests(retry_in),
    };
    let response = next.run(request).await;
    drop(guard);
    response
}

/// Middleware for an instance serving with no web token, which ADR-07 allows
/// only on a loopback bind. Without a token, the address a request was sent to
/// and the page that sent it are all that separate the local user from any web
/// page: a page on an attacker's domain whose DNS answer flips to 127.0.0.1
/// becomes same-origin with the API (DNS rebinding), and a cross-site form can
/// post a multipart upload without rebinding at all (#629).
///
/// So two checks, read from the live settings on every request:
///
/// - The `Host` must name this machine (`localhost`, `127.0.0.1`, `[::1]`, any
///   port) or the host of `HATCHDOOR_PUBLIC_URL`. A rebinding page always sends
///   its own name. A request with no `Host` at all (HTTP/1.0, in-process tests)
///   cannot come from a browser and passes.
/// - A state-changing request (anything but `GET`, `HEAD` and `OPTIONS`) must
///   not come from another site. Its `Origin`, when present, must be this
///   request's own origin, `HATCHDOOR_PUBLIC_URL`'s, or one the MCP allow-list
///   accepts ([`crate::mcp::auth::is_allowed_origin`], ADR-09). Without an
///   `Origin`, `Sec-Fetch-Site: cross-site` is refused. `curl` sends neither and
///   passes.
///
/// It is layered only when no web token is configured and demo mode is off;
/// `/mcp` keeps its own Origin check and is not behind it.
pub(crate) async fn require_local_host_and_origin(
    State(runtime_config): State<crate::runtime_config::RuntimeConfig>,
    request: Request,
    next: Next,
) -> Response {
    let mcp = match crate::mcp::McpConfig::from_snapshot(&runtime_config.snapshot()) {
        Ok(config) => config,
        // A malformed runtime configuration must never open this gate.
        Err(_) => return forbidden_request(FORBIDDEN_HOST),
    };
    let public_url = mcp.public_url.as_deref();
    let host = request_host(&request);

    if let Some(host) = host
        && !host_is_local_or_public(host, public_url)
    {
        return forbidden_request(FORBIDDEN_HOST);
    }

    if !matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    ) {
        let headers = request.headers();
        let origin = headers.get(header::ORIGIN).map(|value| value.to_str());
        let refused = match origin {
            Some(Ok(origin)) => !origin_is_allowed(origin, host, public_url, &mcp.allowed_origins),
            Some(Err(_)) => true,
            None => headers
                .get("sec-fetch-site")
                .and_then(|value| value.to_str().ok())
                .is_some_and(|site| site.trim().eq_ignore_ascii_case("cross-site")),
        };
        if refused {
            return forbidden_request(FORBIDDEN_ORIGIN);
        }
    }

    next.run(request).await
}

const FORBIDDEN_HOST: &str = "This Hatchdoor has no web token, so it only answers requests \
     addressed to localhost, 127.0.0.1, [::1] or HATCHDOOR_PUBLIC_URL. Set \
     HATCHDOOR_WEB_BEARER_TOKEN or HATCHDOOR_PUBLIC_URL to reach it under another name.";
const FORBIDDEN_ORIGIN: &str = "This Hatchdoor has no web token, so it refuses changes \
     sent from another website.";

/// The `Host` a request was addressed to: the header, or the URI's authority
/// for an HTTP/2 request that carries it there instead.
fn request_host(request: &Request) -> Option<&str> {
    request
        .headers()
        .get(header::HOST)
        // A Host that is not text cannot name this machine: compare it as
        // empty, which nothing allows, rather than skip the check.
        .map(|value| value.to_str().unwrap_or_default())
        .or_else(|| {
            request
                .uri()
                .authority()
                .map(|authority| authority.as_str())
        })
        .map(str::trim)
}

/// `authority` without its port: `[::1]:42824` gives `[::1]`, `localhost:5173`
/// gives `localhost`.
fn host_name(authority: &str) -> &str {
    if authority.starts_with('[') {
        return authority
            .find(']')
            .map_or(authority, |end| &authority[..=end]);
    }
    match authority.rsplit_once(':') {
        Some((name, port)) if port.chars().all(|ch| ch.is_ascii_digit()) => name,
        _ => authority,
    }
}

/// The authority (`host[:port]`) of an absolute `http`/`https` address.
fn url_authority(url: &str) -> Option<&str> {
    let (scheme, rest) = url.split_once("://")?;
    if !(scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")) {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next()?;
    (!authority.is_empty()).then_some(authority)
}

fn host_is_local_or_public(host: &str, public_url: Option<&str>) -> bool {
    let name = host_name(host);
    if ["localhost", "127.0.0.1", "[::1]"]
        .iter()
        .any(|local| name.eq_ignore_ascii_case(local))
    {
        return true;
    }
    public_url
        .and_then(url_authority)
        .is_some_and(|public| host_name(public).eq_ignore_ascii_case(name))
}

fn origin_is_allowed(
    origin: &str,
    host: Option<&str>,
    public_url: Option<&str>,
    allowed_origins: &[String],
) -> bool {
    let origin = origin.trim();
    if let (Some(authority), Some(host)) = (url_authority(origin), host)
        && authority.eq_ignore_ascii_case(host)
    {
        return true;
    }
    if let (Some(origin_authority), Some(public_authority)) =
        (url_authority(origin), public_url.and_then(url_authority))
        && origin_authority.eq_ignore_ascii_case(public_authority)
    {
        return true;
    }
    crate::mcp::auth::is_allowed_origin(origin, allowed_origins)
}

fn forbidden_request(message: &str) -> Response {
    (
        StatusCode::FORBIDDEN,
        axum::Json(ErrorResponse {
            error: message.to_string(),
        }),
    )
        .into_response()
}

/// `429` with `Retry-After`, for a request refused by the MCP tool budget
/// outside `/mcp` (this route, and transfer-link downloads).
pub(crate) fn too_many_requests(retry_in: std::time::Duration) -> Response {
    (
        StatusCode::TOO_MANY_REQUESTS,
        [(
            header::RETRY_AFTER,
            crate::mcp::limits::retry_after_seconds(retry_in).to_string(),
        )],
    )
        .into_response()
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
                "/api/v1/vaults/events?access_token=an%20arbitrary%20configured%20bearer%20token",
                None,
                true,
            ),
            (
                "/api/v1/vaults/events?access_token=an%20arbitrary%20configured%20bearer%20tokem",
                None,
                false,
            ),
            // #703: a route outside `QUERY_TOKEN_ROUTES` takes the header only.
            (
                "/protected?access_token=an%20arbitrary%20configured%20bearer%20token",
                None,
                false,
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
    fn bearer_scheme_is_case_insensitive_and_whitespace_tolerant() {
        for (value, credential) in [
            ("Bearer abc", Some("abc")),
            ("bearer abc", Some("abc")),
            ("BEARER abc", Some("abc")),
            ("Bearer   abc", Some("abc")),
            ("Bearer \t abc ", Some("abc")),
            ("Bearer", None),
            ("Bearer   ", None),
            ("Bearerabc", None),
            ("Basic abc", None),
            ("Bear abc", None),
        ] {
            assert_eq!(bearer_credential(value), credential, "{value:?}");
        }
    }

    #[test]
    fn host_names_drop_only_a_numeric_port() {
        assert_eq!(host_name("localhost:5173"), "localhost");
        assert_eq!(host_name("[::1]:42824"), "[::1]");
        assert_eq!(host_name("[::1]"), "[::1]");
        assert_eq!(host_name("notes.example.com"), "notes.example.com");
        assert!(!host_is_local_or_public("localhost.attacker.example", None));
        assert!(
            !host_is_local_or_public("::1", None),
            "an IPv6 Host is bracketed"
        );
        assert!(host_is_local_or_public(
            "notes.example.com:443",
            Some("https://notes.example.com/sub")
        ));
    }

    #[test]
    fn origins_are_allowed_by_sameness_public_url_or_the_mcp_list() {
        let list = vec![
            "http://127.0.0.1".to_string(),
            "http://localhost".to_string(),
        ];
        assert!(origin_is_allowed(
            "http://127.0.0.1:42824",
            Some("127.0.0.1:42824"),
            None,
            &[]
        ));
        assert!(origin_is_allowed(
            "http://localhost:5173",
            None,
            None,
            &list
        ));
        assert!(origin_is_allowed(
            "https://notes.example.com",
            Some("127.0.0.1:42824"),
            Some("https://notes.example.com/hatchdoor"),
            &[]
        ));
        assert!(!origin_is_allowed(
            "https://evil.example",
            Some("127.0.0.1:42824"),
            None,
            &list
        ));
        assert!(!origin_is_allowed(
            "null",
            Some("127.0.0.1:42824"),
            None,
            &list
        ));
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
    fn redact_query_token_hides_a_transfer_link_signature() {
        assert_eq!(
            redact_query_token("expires=1&replace=false&nonce=n&signature=s3cr3t"),
            "expires=1&replace=false&nonce=n&signature=REDACTED"
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

    #[test]
    fn the_query_token_is_allowed_on_the_listed_get_routes_only() {
        let vault = "00000000-0000-4000-8000-000000000000";
        for (method, path, allowed) in [
            (
                Method::GET,
                format!("/api/v1/vaults/{vault}/assets/a.png"),
                true,
            ),
            (
                Method::GET,
                format!("/api/v1/vaults/{vault}/assets/dir/sub/a%20b.pdf"),
                true,
            ),
            (
                Method::GET,
                format!("/api/v1/vaults/{vault}/notes/home/download"),
                true,
            ),
            (Method::GET, "/api/v1/vaults/events".to_string(), true),
            // Not a GET.
            (Method::POST, "/api/v1/vaults/events".to_string(), false),
            (
                Method::PUT,
                format!("/api/v1/vaults/{vault}/assets/a.png"),
                false,
            ),
            // Near misses on the path.
            (
                Method::GET,
                format!("/api/v1/vaults/{vault}/assets/"),
                false,
            ),
            (Method::GET, format!("/api/v1/vaults/{vault}/assets"), false),
            (
                Method::GET,
                format!("/api/v1/vaults/{vault}/notes/home"),
                false,
            ),
            (
                Method::GET,
                format!("/api/v1/vaults/{vault}/notes/home/download/x"),
                false,
            ),
            (
                Method::GET,
                "/api/v1/vaults//notes/home/download".to_string(),
                false,
            ),
            (Method::GET, "/api/v1/vaults/events/x".to_string(), false),
            (Method::GET, "/api/v1/vaults".to_string(), false),
            (Method::GET, "/api/settings".to_string(), false),
            (
                Method::POST,
                "/api/settings/web-token/reveal".to_string(),
                false,
            ),
            (Method::GET, "/api/v1/folders".to_string(), false),
        ] {
            assert_eq!(
                query_token_allowed(&method, &path),
                allowed,
                "{method} {path}"
            );
        }
    }

    /// Every `withAccessToken(...)` call in the frontend builds an address on
    /// one of [`QUERY_TOKEN_ROUTES`]: a call that puts the token on any other
    /// route would get `401` from the server and leak the token into a URL for
    /// nothing (#703). Each call must pass a string or template literal, read
    /// here with every `${...}` stood in for by one path segment.
    #[test]
    fn every_frontend_query_token_url_is_on_an_allowed_route() {
        fn sources(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).expect("read frontend dir") {
                let path = entry.expect("dir entry").path();
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if path.is_dir() {
                    sources(&path, out);
                } else if (name.ends_with(".ts") || name.ends_with(".tsx"))
                    && !name.contains(".test.")
                {
                    out.push(path);
                }
            }
        }

        /// The literal that opens `rest`, with each interpolation replaced by
        /// `x`, or `None` when `rest` does not open with a literal.
        fn literal_url(rest: &str) -> Option<String> {
            let rest = rest.trim_start();
            let quote = rest.chars().next()?;
            if !matches!(quote, '"' | '\'' | '`') {
                return None;
            }
            let mut url = String::new();
            let mut chars = rest[1..].chars().peekable();
            while let Some(ch) = chars.next() {
                if ch == quote {
                    return Some(url);
                }
                if quote == '`' && ch == '$' && chars.peek() == Some(&'{') {
                    chars.next();
                    let mut depth = 1;
                    for inner in chars.by_ref() {
                        match inner {
                            '{' => depth += 1,
                            '}' => depth -= 1,
                            _ => {}
                        }
                        if depth == 0 {
                            break;
                        }
                    }
                    url.push('x');
                } else {
                    url.push(ch);
                }
            }
            None
        }

        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("frontend/src");
        let mut files = Vec::new();
        sources(&root, &mut files);
        let mut calls = 0;
        for file in files {
            let text = std::fs::read_to_string(&file).expect("read source");
            for (at, _) in text.match_indices("withAccessToken(") {
                if text[..at].ends_with("function ") {
                    continue;
                }
                calls += 1;
                let rest = &text[at + "withAccessToken(".len()..];
                let url = literal_url(rest).unwrap_or_else(|| {
                    panic!(
                        "{}: withAccessToken must be given a string or template literal",
                        file.display()
                    )
                });
                assert!(
                    query_token_allowed(&Method::GET, &url),
                    "{}: withAccessToken builds {url:?}, which is not on QUERY_TOKEN_ROUTES",
                    file.display()
                );
            }
        }
        assert!(calls >= 5, "found only {calls} withAccessToken calls");
    }
}
