//! `GET /api/v1/whats-new`: which versions this instance has run and the
//! release highlights between them (ADR-42), for the signed-in app.
//!
//! The highlights live in the bundled manual's What's new page, one section
//! per release, newest first:
//!
//! ```markdown
//! ## v2.8.0 - 2026-10-20
//!
//! - **Action needed:** Something to do before upgrading. [[Some page#Some heading]]
//! - A plain line about what changed.
//! ```
//!
//! Each section holds 3 to 6 one-line items, action-needed items first. An
//! item may end with one link into the manual. Anything else is a malformed
//! page, which `the_bundled_page_parses` turns into a failed test rather than
//! a broken screen.
//!
//! The answer names the running version, so `src/server.rs` mounts the route
//! behind the web token and refuses it in demo mode (ADR-38 decision 6).

use std::sync::LazyLock;

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::app_state::AppState;
use crate::instance_state::VersionRecord;

/// The What's new page's name in the bundled manual.
pub const PAGE: &str = "whats-new";

/// The fewest and most items a release section may hold.
const MIN_HIGHLIGHTS: usize = 3;
const MAX_HIGHLIGHTS: usize = 6;

const ACTION_NEEDED: &str = "**Action needed:**";

/// One release's section of the What's new page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct Release {
    /// The bare version, such as `2.8.0`.
    version: String,
    /// The release date, `YYYY-MM-DD`.
    date: String,
    /// Action-needed items first.
    highlights: Vec<Highlight>,
}

/// One line of a release section.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct Highlight {
    /// The line as Markdown, without its action-needed marker or its link.
    text: String,
    action_needed: bool,
    /// The manual page that explains it, if the line links one.
    link: Option<HighlightLink>,
}

/// A link from a highlight into the manual.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct HighlightLink {
    /// The link's text.
    label: String,
    /// The page name, as `read_docs` and `/docs/<page>.md` take it.
    page: String,
    /// The heading anchor, in the note slug rule, when the link names one.
    heading: Option<String>,
}

/// The bundled page's releases, newest first. The page is checked by
/// `the_bundled_page_parses`; should it still fail here, the failure is
/// logged and no release is listed.
fn releases() -> &'static [Release] {
    static RELEASES: LazyLock<Vec<Release>> = LazyLock::new(|| {
        let Some(page) = crate::docs_bundle::page(PAGE) else {
            tracing::error!("The bundled manual has no {PAGE} page");
            return Vec::new();
        };
        parse_releases(&page.markdown).unwrap_or_else(|error| {
            tracing::error!("The bundled {PAGE} page is malformed: {error}");
            Vec::new()
        })
    });
    &RELEASES
}

/// Parse a What's new page, wikilinks already rewritten to Markdown links
/// (as [`crate::docs_bundle::ManualPage::markdown`] holds it), into its
/// release sections. Text before the first section is the intro and is not
/// read.
fn parse_releases(markdown: &str) -> Result<Vec<Release>, String> {
    let mut releases: Vec<Release> = Vec::new();
    let mut in_code = false;
    for line in markdown.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_code = !in_code;
        }
        if let Some(heading) = line.strip_prefix("## ") {
            if in_code {
                return Err(format!("a code block is open at \"{line}\""));
            }
            let release = parse_heading(heading)?;
            if let Some(newer) = releases.last()
                && parse_version(&release.version) >= parse_version(&newer.version)
            {
                return Err(format!(
                    "v{} comes after v{}; releases go newest first",
                    release.version, newer.version
                ));
            }
            releases.push(release);
            continue;
        }
        let Some(release) = releases.last_mut() else {
            continue;
        };
        if trimmed.is_empty() {
            continue;
        }
        let Some(item) = line.strip_prefix("- ") else {
            return Err(format!(
                "v{} has a line that is not a one-line \"- \" item: \"{line}\"",
                release.version
            ));
        };
        let highlight =
            parse_item(item).map_err(|error| format!("v{}: {error}", release.version))?;
        if highlight.action_needed
            && release
                .highlights
                .last()
                .is_some_and(|before| !before.action_needed)
        {
            return Err(format!(
                "v{} lists an action-needed item after a plain one; action-needed items go first",
                release.version
            ));
        }
        release.highlights.push(highlight);
    }
    for release in &releases {
        let count = release.highlights.len();
        if !(MIN_HIGHLIGHTS..=MAX_HIGHLIGHTS).contains(&count) {
            return Err(format!(
                "v{} has {count} items; a release has {MIN_HIGHLIGHTS} to {MAX_HIGHLIGHTS}",
                release.version
            ));
        }
    }
    Ok(releases)
}

/// `major.minor.patch` as numbers, for ordering, or `None` for anything that
/// is not three dot-separated numbers.
pub fn parse_version(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.split('.').map(|part| part.parse::<u64>().ok());
    let parsed = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(parsed)
}

/// `v2.8.0 - 2026-10-20`.
fn parse_heading(heading: &str) -> Result<Release, String> {
    let malformed =
        || format!("the heading \"## {heading}\" is not \"## v<version> - <YYYY-MM-DD>\"");
    let (version, date) = heading
        .trim()
        .strip_prefix('v')
        .and_then(|rest| rest.split_once(" - "))
        .ok_or_else(malformed)?;
    let date_ok = date.len() == 10
        && date.char_indices().all(|(at, c)| match at {
            4 | 7 => c == '-',
            _ => c.is_ascii_digit(),
        });
    if parse_version(version).is_none() || !date_ok {
        return Err(malformed());
    }
    Ok(Release {
        version: version.to_string(),
        date: date.to_string(),
        highlights: Vec::new(),
    })
}

/// One item, without its `- `.
fn parse_item(item: &str) -> Result<Highlight, String> {
    let item = item.trim();
    let (action_needed, rest) = match item.strip_prefix(ACTION_NEEDED) {
        Some(rest) => (true, rest.trim_start()),
        None => (false, item),
    };
    if rest.contains("[[") {
        return Err(format!(
            "\"{item}\" links to a page the manual does not have"
        ));
    }
    let (text, link) = match trailing_link(rest) {
        Some((text, link)) => (text, Some(link)),
        None => (rest, None),
    };
    let text = text.trim_end();
    if has_manual_link(text) {
        return Err(format!(
            "\"{item}\" has a manual link that is not at its end; an item carries at most one, last"
        ));
    }
    if text.is_empty() {
        return Err(format!("\"{item}\" has no text"));
    }
    Ok(Highlight {
        text: text.to_string(),
        action_needed,
        link,
    })
}

/// Whether `text` holds a Markdown link into the manual, as opposed to an
/// outside address.
fn has_manual_link(text: &str) -> bool {
    text.match_indices("](").any(|(at, _)| {
        let destination = &text[at + 2..];
        let destination = destination.split(')').next().unwrap_or_default();
        !destination.contains("://")
    })
}

/// A `[label](page#heading)` that ends `line`, split off from the text before
/// it. External links are not manual links and are left in the text.
fn trailing_link(line: &str) -> Option<(&str, HighlightLink)> {
    let inner = line.strip_suffix(')')?;
    let middle = inner.rfind("](")?;
    let destination = &inner[middle + 2..];
    let open = inner[..middle].rfind('[')?;
    if destination.contains("://") || destination.contains(' ') {
        return None;
    }
    let (page, heading) = match destination.split_once('#') {
        Some((page, heading)) => (page, Some(heading.to_string())),
        None => (destination, None),
    };
    Some((
        &line[..open],
        HighlightLink {
            label: inner[open + 1..middle].to_string(),
            page: page.to_string(),
            heading,
        },
    ))
}

/// The releases after `previous` up to and including `current`, newest
/// first. None when there is no previous version, which is a fresh install.
fn releases_between<'a>(releases: &'a [Release], record: &VersionRecord) -> Vec<&'a Release> {
    let Some(previous) = record.previous.as_deref().and_then(parse_version) else {
        return Vec::new();
    };
    let Some(current) = parse_version(&record.current) else {
        return Vec::new();
    };
    releases
        .iter()
        .filter(|release| {
            parse_version(&release.version)
                .is_some_and(|version| version > previous && version <= current)
        })
        .collect()
}

#[derive(Debug, Serialize)]
struct WhatsNewResponse<'a> {
    /// The running version as the binary reports it, dev suffix included.
    version: String,
    previous_version: Option<&'a str>,
    /// Whether this instance was first started with nothing in place and
    /// still runs the version it started on. False again after its first
    /// upgrade, so an instance that began fresh still hears what changed.
    fresh_install: bool,
    releases: Vec<&'a Release>,
}

pub async fn whats_new_handler(State(state): State<AppState>) -> Response {
    let record = &state.instance_versions;
    let mut response = Json(WhatsNewResponse {
        version: crate::config::version_string(),
        previous_version: record.previous.as_deref(),
        fresh_install: record.fresh_install.as_deref() == Some(record.current.as_str()),
        releases: releases_between(releases(), record),
    })
    .into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    const PAGE_FIXTURE: &str = "# What's new\n\nIntro, not a release. - not an item either\n\n\
## v2.9.0 - 2026-12-01\n\n\
- **Action needed:** Back up first. [Backups](get-started/understand-where-your-data-lives#backups)\n\
- Faster search.\n\
- A `code` span and an [outside link](https://example.com) stay in the text.\n\n\
## v2.8.0 - 2026-10-20\n\n\
- One.\n- Two.\n- Three. [Install](get-started/install-hatchdoor-with-docker-compose)\n";

    fn record(current: &str, previous: Option<&str>) -> VersionRecord {
        VersionRecord {
            current: current.into(),
            previous: previous.map(Into::into),
            fresh_install: None,
        }
    }

    #[test]
    fn versions_parse_as_three_numbers_only() {
        assert_eq!(parse_version("2.10.3"), Some((2, 10, 3)));
        assert!(parse_version("2.8.0") > parse_version("2.7.12"));
        for bad in ["2.8", "2.8.0.1", "v2.8.0", "2.8.x", ""] {
            assert_eq!(parse_version(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_bundled_page_parses() {
        let page = crate::docs_bundle::page(PAGE).expect("the manual has a What's new page");
        assert!(
            page.private,
            "What's new names the running version, so it is private"
        );
        if let Err(error) = parse_releases(&page.markdown) {
            panic!("docs/user-vault/What's new.md is malformed: {error}");
        }
    }

    #[test]
    fn a_page_parses_into_releases_newest_first() {
        let releases = parse_releases(PAGE_FIXTURE).expect("parses");
        assert_eq!(
            releases
                .iter()
                .map(|release| (release.version.as_str(), release.date.as_str()))
                .collect::<Vec<_>>(),
            [("2.9.0", "2026-12-01"), ("2.8.0", "2026-10-20")]
        );
        assert_eq!(
            releases[0].highlights[0],
            Highlight {
                text: "Back up first.".into(),
                action_needed: true,
                link: Some(HighlightLink {
                    label: "Backups".into(),
                    page: "get-started/understand-where-your-data-lives".into(),
                    heading: Some("backups".into()),
                }),
            }
        );
        assert_eq!(
            releases[0].highlights[2],
            Highlight {
                text: "A `code` span and an [outside link](https://example.com) stay in the text."
                    .into(),
                action_needed: false,
                link: None,
            }
        );
        assert_eq!(
            releases[1].highlights[2]
                .link
                .as_ref()
                .map(|link| link.heading.clone()),
            Some(None)
        );
    }

    #[test]
    fn a_page_with_no_release_is_empty() {
        assert_eq!(parse_releases("# What's new\n\nIntro.\n"), Ok(Vec::new()));
    }

    #[test]
    fn a_malformed_section_fails_with_a_clear_message() {
        let section =
            |heading: &str, items: &str| format!("# What's new\n\n## {heading}\n\n{items}");
        let cases = [
            (
                section("v2.8.0 - 2026-10-20", "- One.\n- Two.\n"),
                "v2.8.0 has 2 items",
            ),
            (
                section("v2.8.0 - 2026-10-20", "- 1\n- 2\n- 3\n- 4\n- 5\n- 6\n- 7\n"),
                "v2.8.0 has 7 items",
            ),
            (
                section(
                    "v2.8.0 - 2026-10-20",
                    "- One.\n- **Action needed:** Two.\n- Three.\n",
                ),
                "action-needed items go first",
            ),
            (
                section("2.8.0 - 2026-10-20", "- 1\n- 2\n- 3\n"),
                "is not \"## v<version>",
            ),
            (
                section("v2.8 - 2026-10-20", "- 1\n- 2\n- 3\n"),
                "is not \"## v<version>",
            ),
            (
                section("v2.8.0 - October", "- 1\n- 2\n- 3\n"),
                "is not \"## v<version>",
            ),
            (
                section("v2.8.0 - 2026-10-20", "- 1\n- 2\n- 3\n\nA paragraph.\n"),
                "not a one-line \"- \" item",
            ),
            (
                section("v2.8.0 - 2026-10-20", "- 1 [[Nowhere]]\n- 2\n- 3\n"),
                "links to a page the manual does not have",
            ),
            (
                section("v2.8.0 - 2026-10-20", "- [a](x) then text\n- 2\n- 3\n"),
                "has a manual link that is not at its end",
            ),
            (
                format!(
                    "{}\n## v2.9.0 - 2026-12-01\n\n- 1\n- 2\n- 3\n",
                    section("v2.8.0 - 2026-10-20", "- 1\n- 2\n- 3\n")
                ),
                "v2.9.0 comes after v2.8.0",
            ),
        ];
        for (page, expected) in cases {
            let error = parse_releases(&page).expect_err(&page);
            assert!(
                error.contains(expected),
                "{error:?} should say {expected:?}"
            );
        }
    }

    #[test]
    fn only_releases_after_the_previous_version_are_listed() {
        let releases = parse_releases(PAGE_FIXTURE).expect("parses");
        let versions = |record: &VersionRecord| {
            releases_between(&releases, record)
                .iter()
                .map(|release| release.version.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            versions(&record("2.9.0", Some("2.7.0"))),
            ["2.9.0", "2.8.0"]
        );
        assert_eq!(versions(&record("2.9.0", Some("2.8.0"))), ["2.9.0"]);
        assert_eq!(versions(&record("2.8.0", Some("2.7.0"))), ["2.8.0"]);
        assert!(versions(&record("2.9.0", Some("2.9.0"))).is_empty());
        assert!(
            versions(&record("2.9.0", None)).is_empty(),
            "a fresh install"
        );
    }

    async fn get(uri: &str, token: Option<&str>) -> (StatusCode, String) {
        let router: axum::Router = crate::handlers::docs_router(Some("web-secret".into()));
        let mut request = Request::builder().uri(uri);
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        let response = router
            .oneshot(request.body(Body::empty()).expect("request"))
            .await
            .expect("response");
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        (status, String::from_utf8(body.to_vec()).expect("utf-8"))
    }

    #[tokio::test]
    async fn the_page_stays_off_the_public_docs_addresses() {
        let address = format!("/docs/{PAGE}.md");
        assert_eq!(get(&address, None).await.0, StatusCode::UNAUTHORIZED);
        assert_eq!(get(&address, Some("web-secret")).await.0, StatusCode::OK);
        for token in [None, Some("web-secret")] {
            let (_, llms) = get("/llms.txt", token).await;
            assert!(!llms.contains(PAGE), "llms.txt lists {PAGE}");
        }
        let (_, index) = get("/docs/index.md", None).await;
        assert!(!index.contains(PAGE), "the public index lists {PAGE}");
    }
}
