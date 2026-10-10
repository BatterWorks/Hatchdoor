//! The bundled manual as plain Markdown at public addresses (ADR-38):
//! `/docs/<page>.md`, `/docs/index.md`, `/docs/deploy.md`, `/docs/search` and
//! `/llms.txt`. They need no token, so an agent's ordinary web fetch can read
//! the manual of the instance in front of it.
//!
//! These routes read the bundled manual and nothing else: never a Vault, the
//! registry, settings or a token. The one thing the web token changes is
//! whether private pages are served (ADR-38 decision 6). A private page
//! answers its address only with the token, never appears in `llms.txt`, and
//! appears in the index and search only for a caller who sent it. With no web
//! token configured there is nothing to send, so private pages stay hidden.

use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::{Deserialize, Serialize};

use crate::auth::request_is_authorized;
use crate::docs_bundle::{Manual, ManualPage};

/// The agent deploy page, also served at the short address `/docs/deploy.md`
/// that the docs and README advertise.
const DEPLOY_PAGE: &str = "guides/how-to-deploy-hatchdoor-with-an-agent";

/// The longest search query read, in characters. The rest is ignored.
const MAX_QUERY_CHARS: usize = 200;

const MARKDOWN: &str = "text/markdown; charset=utf-8";
const PLAIN_TEXT: &str = "text/plain; charset=utf-8";

#[derive(Clone)]
struct DocsState {
    manual: &'static Manual,
    web_token: Option<Arc<str>>,
}

impl DocsState {
    /// Whether this caller sent the web token, which is what private pages
    /// need. An instance with no web token reveals them to nobody here.
    fn reveals_private(&self, request: &Request) -> bool {
        self.web_token
            .as_ref()
            .is_some_and(|token| request_is_authorized(request, token.as_bytes()))
    }
}

/// The public manual routes over the bundled manual. The composition root
/// mounts them outside every auth layer, demo mode included: they answer the
/// same either way.
pub fn docs_router<S>(web_token: Option<Arc<str>>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    router_for(crate::docs_bundle::manual(), web_token)
}

fn router_for<S>(manual: &'static Manual, web_token: Option<Arc<str>>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/llms.txt", get(llms_txt_handler))
        .route("/docs/search", get(search_handler))
        .route("/docs/{*path}", get(page_handler))
        .with_state(DocsState { manual, web_token })
}

/// `/docs/index.md`, `/docs/deploy.md` or `/docs/<page>.md`.
async fn page_handler(
    State(state): State<DocsState>,
    Path(path): Path<String>,
    request: Request,
) -> Response {
    let reveals_private = state.reveals_private(&request);
    let manual = state.manual;
    // Links are written relative to the address asked for, so they resolve
    // from wherever the page was fetched.
    let depth = path.matches('/').count();
    let Some(name) = path.strip_suffix(".md") else {
        return not_found();
    };
    if name == "index" {
        return markdown_response(index_markdown(manual, reveals_private));
    }
    let page = if name == "deploy" {
        manual.page(DEPLOY_PAGE)
    } else {
        manual.page(name)
    };
    let Some(page) = page else {
        return not_found();
    };
    if page.private && !reveals_private {
        return (
            StatusCode::UNAUTHORIZED,
            [
                (header::WWW_AUTHENTICATE, "Bearer"),
                (header::CONTENT_TYPE, PLAIN_TEXT),
            ],
            "This page of the manual needs the web token.\n",
        )
            .into_response();
    }
    let markdown = manual.markdown_linking(page, |target, anchor| {
        public_link(target, anchor, depth, reveals_private)
    });
    let mut response = markdown_response(markdown);
    if page.private {
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("private, no-store"),
        );
    }
    response
}

/// Home, then a list of every page the caller may read.
fn index_markdown(manual: &Manual, reveals_private: bool) -> String {
    let mut out = manual.markdown_linking(manual.home(), |target, anchor| {
        public_link(target, anchor, 0, reveals_private)
    });
    out.push_str("\n\n## All pages\n\n");
    for page in visible_pages(manual, reveals_private).skip(1) {
        out.push_str(&list_line(page, ""));
    }
    out
}

/// The llms.txt index (<https://llmstxt.org>): the manual's title, a summary
/// taken from Home, then every public page, the deploy page first. Private
/// pages never appear, whoever asks.
async fn llms_txt_handler(State(state): State<DocsState>) -> Response {
    let manual = state.manual;
    let home = manual.markdown_linking(manual.home(), |target, anchor| {
        public_link(target, anchor, 0, false).map(|link| format!("docs/{link}"))
    });
    let summary = home
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .unwrap_or_default();
    let mut out = format!(
        "# Hatchdoor\n\n> {summary}\n\n\
         This is the manual of the Hatchdoor instance serving this file. Every page is plain \
         Markdown. Start with the deploy page to install Hatchdoor, or the index for the whole \
         manual.\n\n## Manual\n\n"
    );
    let deploy = manual.page(DEPLOY_PAGE).filter(|page| !page.private);
    if let Some(deploy) = deploy {
        out.push_str(&format!("- [{}](docs/deploy.md)\n", deploy.title));
    }
    out.push_str("- [Index of every page](docs/index.md)\n");
    for page in visible_pages(manual, false) {
        if deploy.is_some_and(|deploy| std::ptr::eq(deploy, page)) {
            continue;
        }
        out.push_str(&list_line(page, "docs/"));
    }
    ([(header::CONTENT_TYPE, PLAIN_TEXT)], out).into_response()
}

#[derive(Deserialize)]
struct SearchParams {
    #[serde(default)]
    q: String,
}

#[derive(Serialize)]
struct SearchResponse {
    results: Vec<SearchHit>,
}

#[derive(Serialize)]
struct SearchHit {
    /// The page name; the page is at `/docs/<name>.md`.
    name: String,
    title: String,
    excerpt: String,
}

/// The same word search as the MCP `search_docs` tool, as JSON.
async fn search_handler(
    State(state): State<DocsState>,
    Query(params): Query<SearchParams>,
    request: Request,
) -> Response {
    let query: String = params.q.chars().take(MAX_QUERY_CHARS).collect();
    let results = state
        .manual
        .search(&query, state.reveals_private(&request))
        .into_iter()
        .map(|hit| SearchHit {
            name: hit.page.name.clone(),
            title: hit.page.title.clone(),
            excerpt: hit.excerpt,
        })
        .collect();
    let mut response = axum::Json(SearchResponse { results }).into_response();
    vary_on_token(&mut response);
    response
}

/// One page as a Markdown list item, its address under `prefix`.
fn list_line(page: &ManualPage, prefix: &str) -> String {
    format!("- [{}]({prefix}{}.md)\n", page.title, page.name)
}

fn visible_pages(manual: &Manual, reveals_private: bool) -> impl Iterator<Item = &ManualPage> {
    manual
        .pages()
        .iter()
        .filter(move |page| reveals_private || !page.private)
}

/// Where a wikilink to `target` points from an address `depth` folders below
/// `/docs/`, or `None` for a private page the caller may not read, which
/// leaves only the link text.
fn public_link(
    target: &ManualPage,
    anchor: Option<&str>,
    depth: usize,
    reveals_private: bool,
) -> Option<String> {
    if target.private && !reveals_private {
        return None;
    }
    let mut link = "../".repeat(depth);
    link.push_str(&target.name);
    link.push_str(".md");
    if let Some(anchor) = anchor {
        link.push('#');
        link.push_str(anchor);
    }
    Some(link)
}

fn markdown_response(markdown: String) -> Response {
    let mut response = ([(header::CONTENT_TYPE, MARKDOWN)], markdown).into_response();
    vary_on_token(&mut response);
    response
}

/// What a page, the index and search hold depends on the web token, so a
/// cache in front must not hand one caller's answer to another.
fn vary_on_token(response: &mut Response) {
    response
        .headers_mut()
        .insert(header::VARY, HeaderValue::from_static("authorization"));
}

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, PLAIN_TEXT)],
        "No page of the Hatchdoor manual has this address. Every page is listed at /docs/index.md.\n",
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::{BTreeSet, VecDeque};

    use axum::body::{Body, to_bytes};
    use axum::http::Request as HttpRequest;
    use tower::ServiceExt;

    const TOKEN: &str = "web-token";

    fn fixture_manual() -> &'static Manual {
        Box::leak(Box::new(Manual::from_sources(&[
            (
                "Home.md",
                "# Fixture manual\n\nThe fixture front page. See [[Guide]] and [[Secret]].\n",
            ),
            (
                "02 Guides/Guide.md",
                "# Guide\n\nA public guide about tokens. Back to [[Home]], on to [[Secret#Inside]].\n",
            ),
            (
                "02 Guides/Secret.md",
                "---\nprivate: true\n---\n\n# Secret\n\n## Inside\n\nThe running version is 9.9.9, tokens and all.\n",
            ),
        ])))
    }

    fn fixture(web_token: Option<&str>) -> Router {
        router_for(fixture_manual(), web_token.map(Arc::from))
    }

    fn bundled(web_token: Option<&str>) -> Router {
        docs_router(web_token.map(Arc::from))
    }

    async fn get_text(
        app: &Router,
        uri: &str,
        token: Option<&str>,
    ) -> (StatusCode, String, Response) {
        let mut request = HttpRequest::builder().uri(uri);
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::empty()).expect("request"))
            .await
            .expect("response");
        let status = response.status();
        let (parts, body) = response.into_parts();
        let bytes = to_bytes(body, usize::MAX).await.expect("body");
        (
            status,
            String::from_utf8(bytes.to_vec()).expect("utf-8"),
            Response::from_parts(parts, Body::empty()),
        )
    }

    fn content_type(response: &Response) -> &str {
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
    }

    /// Relative link destinations outside code, as a reader would follow
    /// them; external and anchor-only links are left out.
    fn relative_links(markdown: &str) -> Vec<String> {
        let mut links = Vec::new();
        let mut in_fence = false;
        for line in markdown.lines() {
            if line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~") {
                in_fence = !in_fence;
                continue;
            }
            if in_fence {
                continue;
            }
            // Drop inline code spans, which show link syntax as examples.
            let prose: String = line.split('`').step_by(2).collect();
            let mut rest = prose.as_str();
            while let Some(at) = rest.find("](") {
                rest = &rest[at + 2..];
                let Some(end) = rest.find(')') else { break };
                let destination = &rest[..end];
                if !destination.contains("://")
                    && !destination.starts_with('#')
                    && !destination.starts_with("mailto:")
                {
                    links.push(destination.to_string());
                }
                rest = &rest[end..];
            }
        }
        links
    }

    /// `link` resolved against the address `base`, both server paths.
    fn resolve(base: &str, link: &str) -> String {
        let link = link.split('#').next().unwrap_or_default();
        let mut segments: Vec<&str> = base.split('/').collect();
        segments.pop();
        for part in link.split('/') {
            match part {
                ".." => {
                    segments.pop();
                }
                "." => {}
                part => segments.push(part),
            }
        }
        segments.join("/")
    }

    #[tokio::test]
    async fn every_page_is_served_as_markdown_without_a_token() {
        for app in [bundled(Some(TOKEN)), bundled(None)] {
            for page in crate::docs_bundle::pages()
                .iter()
                .filter(|page| !page.private)
            {
                let uri = format!("/docs/{}.md", page.name);
                let (status, body, response) = get_text(&app, &uri, None).await;
                assert_eq!(status, StatusCode::OK, "{uri}");
                assert_eq!(content_type(&response), MARKDOWN, "{uri}");
                assert!(body.contains(&page.title), "{uri}");
            }
        }
    }

    #[tokio::test]
    async fn every_link_from_llms_txt_reaches_a_page() {
        let app = bundled(Some(TOKEN));
        let mut queue = VecDeque::from(["/llms.txt".to_string()]);
        let mut seen = BTreeSet::new();
        while let Some(address) = queue.pop_front() {
            if !seen.insert(address.clone()) {
                continue;
            }
            let (status, body, _) = get_text(&app, &address, None).await;
            assert_eq!(status, StatusCode::OK, "{address}");
            for link in relative_links(&body) {
                queue.push_back(resolve(&address, &link));
            }
        }
        assert!(seen.contains("/docs/deploy.md"));
        assert!(seen.contains("/docs/index.md"));
        for page in crate::docs_bundle::pages()
            .iter()
            .filter(|page| !page.private)
        {
            assert!(
                seen.contains(&format!("/docs/{}.md", page.name)),
                "{} is not reachable from llms.txt",
                page.name
            );
        }
    }

    #[tokio::test]
    async fn llms_txt_lists_the_deploy_page_first() {
        let (status, body, response) = get_text(&bundled(None), "/llms.txt", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(content_type(&response), PLAIN_TEXT);
        assert!(body.starts_with("# Hatchdoor\n\n> "));
        let first_link = relative_links(&body).into_iter().next();
        assert_eq!(first_link.as_deref(), Some("docs/deploy.md"));
    }

    #[tokio::test]
    async fn the_deploy_alias_serves_the_agent_deploy_page() {
        let app = bundled(None);
        let deploy = crate::docs_bundle::page(DEPLOY_PAGE).expect("the deploy page exists");
        let (status, body, _) = get_text(&app, "/docs/deploy.md", None).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.starts_with(&format!("# {}", deploy.title)));
        // Served from `/docs/`, its links climb no folders.
        assert!(
            relative_links(&body)
                .iter()
                .all(|link| !link.starts_with("../"))
        );
    }

    #[tokio::test]
    async fn links_are_relative_to_the_address_asked_for() {
        let app = fixture(None);
        let (_, body, _) = get_text(&app, "/docs/guides/guide.md", None).await;
        assert!(body.contains("[Home](../home.md)"), "{body}");
        let (_, body, _) = get_text(&app, "/docs/home.md", None).await;
        assert!(body.contains("[Guide](guides/guide.md)"), "{body}");
    }

    #[tokio::test]
    async fn the_index_is_home_then_every_page() {
        let (status, body, response) = get_text(&fixture(None), "/docs/index.md", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(content_type(&response), MARKDOWN);
        assert!(body.starts_with("# Fixture manual"));
        assert!(body.contains("- [Guide](guides/guide.md)"));
    }

    #[tokio::test]
    async fn searching_git_finds_the_git_guide_first() {
        let (status, body, _) = get_text(&bundled(None), "/docs/search?q=git", None).await;
        assert_eq!(status, StatusCode::OK);
        let json: serde_json::Value = serde_json::from_str(&body).expect("json");
        assert_eq!(
            json["results"][0]["name"],
            "guides/how-to-set-up-a-git-backed-vault"
        );
        assert!(json["results"][0]["title"].is_string());
        assert!(json["results"][0]["excerpt"].is_string());
    }

    #[tokio::test]
    async fn a_long_query_is_cut_to_its_first_200_characters() {
        let app = fixture(None);
        let long = format!("{}%20guide", "x".repeat(MAX_QUERY_CHARS));
        let (_, body, _) = get_text(&app, &format!("/docs/search?q={long}"), None).await;
        let json: serde_json::Value = serde_json::from_str(&body).expect("json");
        assert_eq!(
            json["results"],
            serde_json::json!([]),
            "the word past the cap is ignored"
        );
        let (_, body, _) = get_text(&app, "/docs/search?q=guide", None).await;
        let json: serde_json::Value = serde_json::from_str(&body).expect("json");
        assert_eq!(json["results"][0]["name"], "guides/guide");
    }

    #[tokio::test]
    async fn an_unknown_page_is_a_plain_404() {
        let app = fixture(None);
        for uri in [
            "/docs/nowhere.md",
            "/docs/guides/guide",
            "/docs/guides/guide.txt",
        ] {
            let (status, body, response) = get_text(&app, uri, None).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
            assert_eq!(content_type(&response), PLAIN_TEXT, "{uri}");
            assert!(body.contains("/docs/index.md"), "{uri}");
        }
    }

    #[tokio::test]
    async fn a_private_page_needs_the_web_token() {
        let app = fixture(Some(TOKEN));
        let (status, body, _) = get_text(&app, "/docs/guides/secret.md", None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(!body.contains("9.9.9"));
        let (status, _, _) = get_text(&app, "/docs/guides/secret.md", Some("wrong")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let (status, body, response) = get_text(&app, "/docs/guides/secret.md", Some(TOKEN)).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("9.9.9"));
        assert_eq!(
            response
                .headers()
                .get(header::CACHE_CONTROL)
                .expect("cache control"),
            "private, no-store"
        );
    }

    #[tokio::test]
    async fn a_private_page_stays_out_of_every_public_listing() {
        for app in [fixture(Some(TOKEN)), fixture(None)] {
            for uri in [
                "/llms.txt",
                "/docs/index.md",
                "/docs/search?q=tokens",
                "/docs/home.md",
                "/docs/guides/guide.md",
            ] {
                let (status, body, _) = get_text(&app, uri, None).await;
                assert_eq!(status, StatusCode::OK, "{uri}");
                assert!(
                    !body.contains("secret.md"),
                    "{uri} links the private page: {body}"
                );
                assert!(
                    !body.contains("guides/secret"),
                    "{uri} names the private page: {body}"
                );
                assert!(!body.contains("9.9.9"), "{uri} quotes the private page");
            }
            // A link to it keeps its words, without the address.
            let (_, body, _) = get_text(&app, "/docs/home.md", None).await;
            assert!(
                body.contains("See [Guide](guides/guide.md) and Secret."),
                "{body}"
            );
        }
    }

    #[tokio::test]
    async fn the_web_token_brings_private_pages_into_the_index_and_search_but_not_llms_txt() {
        let app = fixture(Some(TOKEN));
        let (_, body, _) = get_text(&app, "/docs/index.md", Some(TOKEN)).await;
        assert!(body.contains("- [Secret](guides/secret.md)"), "{body}");
        let (_, body, response) = get_text(&app, "/docs/search?q=version", Some(TOKEN)).await;
        assert!(body.contains("guides/secret"), "{body}");
        assert_eq!(
            response.headers().get(header::VARY).expect("vary"),
            "authorization"
        );
        let (_, body, _) = get_text(&app, "/docs/guides/guide.md", Some(TOKEN)).await;
        assert!(
            body.contains("[Secret > Inside](../guides/secret.md#inside)"),
            "{body}"
        );
        let (_, body, _) = get_text(&app, "/llms.txt", Some(TOKEN)).await;
        assert!(!body.contains("secret"), "{body}");
    }

    #[tokio::test]
    async fn with_no_web_token_configured_a_private_page_is_never_served() {
        let app = fixture(None);
        for token in [None, Some(TOKEN)] {
            let (status, body, _) = get_text(&app, "/docs/guides/secret.md", token).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            assert!(!body.contains("9.9.9"));
            let (_, body, _) = get_text(&app, "/docs/index.md", token).await;
            assert!(!body.contains("secret.md"), "{body}");
        }
    }
}
