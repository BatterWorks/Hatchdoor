use axum::extract::rejection::PathRejection;
use axum::extract::{Path, State};
use axum::http::{StatusCode, Uri};
use axum::response::{Html, IntoResponse, Response};

use crate::app_state::AppState;
use crate::handlers::link_preview::LinkPreview;
use crate::mcp::config::parse_public_url;
use crate::vault_management::parse_vault_id;
use crate::vault_read::VaultReads;

const INDEX_PATH: &str = "frontend/dist/index.html";

/// Path prefixes that are never answered with the app, matched with
/// `starts_with`. They mirror the service worker's `navigateFallbackDenylist`
/// in `frontend/vite.config.ts`, so an address answers the same way whether a
/// returning visitor's worker serves it or a cold load reaches the server.
const SPA_RESERVED_PREFIXES: [&str; 5] =
    ["/api/", "/vault-assets/", "/health", "/docs/", "/llms.txt"];

pub async fn spa_index_handler(State(state): State<AppState>, uri: Uri) -> Response {
    match read_index() {
        Some(html) => (StatusCode::OK, Html(page(&state, html, uri.path(), None))).into_response(),
        None => frontend_not_built(),
    }
}

/// The page for a note address, `/v/{vault_id}/n/{slug}`. On a demo instance
/// it previews the note when the unauthenticated demo read of that note
/// succeeds, and Hatchdoor itself otherwise (ADR-47 decision 3).
pub async fn spa_note_handler(
    State(state): State<AppState>,
    note: Result<Path<(String, String)>, PathRejection>,
    uri: Uri,
) -> Response {
    let Some(html) = read_index() else {
        return frontend_not_built();
    };
    let preview = match note {
        Ok(Path((raw_vault_id, slug))) if state.demo_mode => {
            note_preview(&state, &raw_vault_id, slug).await
        }
        _ => None,
    };
    (
        StatusCode::OK,
        Html(page(&state, html, uri.path(), preview)),
    )
        .into_response()
}

/// Answers a request no route and no built file matched (#302).
pub async fn spa_not_found_handler(State(state): State<AppState>, uri: Uri) -> Response {
    let path = uri.path();
    not_found_response(path, || {
        read_index().map(|html| page(&state, html, path, None))
    })
}

fn frontend_not_built() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Html(
            "<h1>Frontend not built</h1><p>Run <code>cd frontend && npm install && npm run build</code>, then restart the server.</p>"
                .to_string(),
        ),
    )
        .into_response()
}

/// The built page as this instance serves it. Outside demo mode that is the
/// file as read, so a request without a token learns nothing (ADR-47
/// decision 1). A demo instance writes a link preview into it: `preview` when
/// the address names a readable note, the general wording otherwise.
fn page(state: &AppState, html: String, path: &str, preview: Option<LinkPreview>) -> String {
    if !state.demo_mode {
        return html;
    }
    let preview = preview.unwrap_or_else(|| LinkPreview::general(path));
    preview.write_into(&html, public_url(state).as_deref(), path)
}

/// The operator's `HATCHDOOR_PUBLIC_URL`, read live. It is the only source of
/// an absolute address: nothing here reads a request header (ADR-47
/// decision 5).
fn public_url(state: &AppState) -> Option<String> {
    let snapshot = state.runtime_snapshot();
    let raw = &snapshot.setting("HATCHDOOR_PUBLIC_URL")?.value;
    parse_public_url(raw).ok().flatten()
}

/// The note's preview, read through the core the demo's own note route uses
/// so the two cannot disagree about what is public. Any refusal is `None`.
async fn note_preview(state: &AppState, raw_vault_id: &str, slug: String) -> Option<LinkPreview> {
    let vault_id = parse_vault_id(raw_vault_id).ok()?;
    VaultReads::new(state)
        .read(move |core| {
            let Some(note) = core.exact_note(vault_id, &slug)? else {
                return Ok(None);
            };
            // Frontmatter that does not parse has no `description` to offer;
            // the note still previews from its body.
            let properties = core
                .exact_note_frontmatter(vault_id, &slug)
                .ok()
                .flatten()
                .map(|frontmatter| frontmatter.metadata.properties)
                .unwrap_or_default();
            Ok(Some(LinkPreview::for_note(&note.note, &properties)))
        })
        .await
        .ok()?
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
    #[cfg(test)]
    if let Some(html) = TEST_INDEX.with(|index| index.borrow().clone()) {
        return Some(html);
    }
    std::fs::read_to_string(INDEX_PATH).ok()
}

#[cfg(test)]
thread_local! {
    static TEST_INDEX: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// Serve `html` as the built page for the rest of this test, whether or not
/// `frontend/dist` exists. Each test runs on its own thread, and a
/// current-thread runtime polls the handlers on it.
#[cfg(test)]
pub(crate) fn serve_test_index(html: &str) {
    TEST_INDEX.with(|index| *index.borrow_mut() = Some(html.to_string()));
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
