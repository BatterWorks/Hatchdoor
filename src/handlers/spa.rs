use axum::http::{StatusCode, Uri};
use axum::response::{Html, IntoResponse, Response};

const INDEX_PATH: &str = "frontend/dist/index.html";

/// Path prefixes that are never answered with the app, matched with
/// `starts_with`. They mirror the service worker's `navigateFallbackDenylist`
/// in `frontend/vite.config.ts`, so an address answers the same way whether a
/// returning visitor's worker serves it or a cold load reaches the server.
const SPA_RESERVED_PREFIXES: [&str; 5] =
    ["/api/", "/vault-assets/", "/health", "/docs/", "/llms.txt"];

pub async fn spa_index_handler() -> impl IntoResponse {
    match read_index() {
        Some(html) => (StatusCode::OK, Html(html)).into_response(),
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            Html(
                "<h1>Frontend not built</h1><p>Run <code>cd frontend && npm install && npm run build</code>, then restart the server.</p>"
                    .to_string(),
            ),
        )
            .into_response(),
    }
}

/// Answers a request no route and no built file matched (#302).
pub async fn spa_not_found_handler(uri: Uri) -> Response {
    not_found_response(uri.path(), read_index)
}

/// Outside the reserved prefixes the app is served with a 404 status, so it
/// can render its own not-found state. Inside them, or with no frontend built,
/// the answer stays a bare 404.
fn not_found_response(path: &str, index: impl FnOnce() -> Option<String>) -> Response {
    if SPA_RESERVED_PREFIXES
        .iter()
        .any(|prefix| path.starts_with(prefix))
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    match index() {
        Some(html) => (StatusCode::NOT_FOUND, Html(html)).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

fn read_index() -> Option<String> {
    std::fs::read_to_string(INDEX_PATH).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    async fn answer(path: &str, index: Option<&str>) -> (StatusCode, String) {
        let response = not_found_response(path, || index.map(str::to_string));
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    const APP: &str = "<div id=\"root\"></div>";

    #[tokio::test]
    async fn an_unrecognised_address_gets_the_app_with_a_404() {
        for path in [
            "/nope",
            "/setting",
            "/v/vault/n/20-projects/Beacon.md",
            "/api",
        ] {
            assert_eq!(
                answer(path, Some(APP)).await,
                (StatusCode::NOT_FOUND, APP.to_string()),
                "{path}"
            );
        }
    }

    #[tokio::test]
    async fn a_reserved_prefix_gets_a_bare_404() {
        for path in [
            "/api/nope",
            "/vault-assets/nope.png",
            "/health/nope",
            "/healthz",
            "/docs/nope.md",
            "/llms.txt",
        ] {
            assert_eq!(
                answer(path, Some(APP)).await,
                (StatusCode::NOT_FOUND, String::new()),
                "{path}"
            );
        }
    }

    #[tokio::test]
    async fn no_built_frontend_gets_a_bare_404() {
        assert_eq!(
            answer("/nope", None).await,
            (StatusCode::NOT_FOUND, String::new())
        );
    }

    #[test]
    fn reserved_prefixes_match_the_service_worker_denylist() {
        // The service worker serves its cached index for every navigation it
        // does not deny. If these prefixes drifted from that list, the same
        // address would answer differently for a returning visitor and a cold
        // load again.
        let config = std::fs::read_to_string("frontend/vite.config.ts").expect("vite config");
        let start = config
            .find("navigateFallbackDenylist: [")
            .expect("denylist present");
        let end = start + config[start..].find("],").expect("denylist end");
        let mut denylist: Vec<String> = config[start..end]
            .lines()
            .filter_map(|line| line.trim().strip_prefix("/^"))
            .map(|pattern| {
                // `\/api\//,` is the regex `^/api/`: drop the comma and the
                // closing delimiter, then the escapes.
                let pattern = pattern.trim_end_matches(',');
                pattern
                    .strip_suffix('/')
                    .unwrap_or(pattern)
                    .replace('\\', "")
            })
            .collect();
        let mut reserved: Vec<String> = SPA_RESERVED_PREFIXES
            .iter()
            .map(|prefix| (*prefix).to_string())
            .collect();
        denylist.sort();
        reserved.sort();
        assert_eq!(reserved, denylist);
    }
}
