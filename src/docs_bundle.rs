//! The bundled manual (ADR-38): every Markdown page under `docs/user-vault`,
//! compiled into the binary so an install carries the manual for its own
//! version.
//!
//! The manual is Help, never a Vault. Nothing here touches the Vault
//! registry, the cache, the index or the embedding model, so the MCP docs
//! tools answer before any Vault exists and while model setup is pending.
//!
//! Each page gets a stable name derived from its path, such as
//! `get-started/install-hatchdoor-with-docker-compose`. The MCP docs tools
//! address pages by it, and wikilinks between pages are rewritten to point at
//! it. Search is plain word matching with no model behind it.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use crate::cache::parse::{frontmatter_span, parse_fence_marker, parse_frontmatter_metadata};
use crate::vault::slugify;

/// Every page, by its path under `docs/user-vault`. A page added to that
/// folder and missing here fails `every_manual_page_is_bundled`.
const PAGES: &[(&str, &str)] = &[
    (
        "01 Get started/Browse and review through the Web UI.md",
        include_str!("../docs/user-vault/01 Get started/Browse and review through the Web UI.md"),
    ),
    (
        "01 Get started/Connect your agent.md",
        include_str!("../docs/user-vault/01 Get started/Connect your agent.md"),
    ),
    (
        "01 Get started/Connect your first Vault.md",
        include_str!("../docs/user-vault/01 Get started/Connect your first Vault.md"),
    ),
    (
        "01 Get started/Install Hatchdoor with Docker Compose.md",
        include_str!("../docs/user-vault/01 Get started/Install Hatchdoor with Docker Compose.md"),
    ),
    (
        "01 Get started/Search and change notes with your agent.md",
        include_str!(
            "../docs/user-vault/01 Get started/Search and change notes with your agent.md"
        ),
    ),
    (
        "01 Get started/Understand where your data lives.md",
        include_str!("../docs/user-vault/01 Get started/Understand where your data lives.md"),
    ),
    (
        "01 Get started/Welcome to Hatchdoor.md",
        include_str!("../docs/user-vault/01 Get started/Welcome to Hatchdoor.md"),
    ),
    (
        "02 Guides/How to choose a folder layout.md",
        include_str!("../docs/user-vault/02 Guides/How to choose a folder layout.md"),
    ),
    (
        "02 Guides/How to deploy Hatchdoor with an agent.md",
        include_str!("../docs/user-vault/02 Guides/How to deploy Hatchdoor with an agent.md"),
    ),
    (
        "02 Guides/How to edit notes with the live editor.md",
        include_str!("../docs/user-vault/02 Guides/How to edit notes with the live editor.md"),
    ),
    (
        "02 Guides/How to import and work with attachments.md",
        include_str!("../docs/user-vault/02 Guides/How to import and work with attachments.md"),
    ),
    (
        "02 Guides/How to manage multiple Vaults.md",
        include_str!("../docs/user-vault/02 Guides/How to manage multiple Vaults.md"),
    ),
    (
        "02 Guides/How to organize a Vault with layers.md",
        include_str!("../docs/user-vault/02 Guides/How to organize a Vault with layers.md"),
    ),
    (
        "02 Guides/How to run an LLM wiki in Hatchdoor.md",
        include_str!("../docs/user-vault/02 Guides/How to run an LLM wiki in Hatchdoor.md"),
    ),
    (
        "02 Guides/How to set up a Git-backed Vault.md",
        include_str!("../docs/user-vault/02 Guides/How to set up a Git-backed Vault.md"),
    ),
    (
        "02 Guides/How to troubleshoot common problems.md",
        include_str!("../docs/user-vault/02 Guides/How to troubleshoot common problems.md"),
    ),
    (
        "02 Guides/How to upgrade Hatchdoor.md",
        include_str!("../docs/user-vault/02 Guides/How to upgrade Hatchdoor.md"),
    ),
    (
        "02 Guides/How to work in a Vault as an agent.md",
        include_str!("../docs/user-vault/02 Guides/How to work in a Vault as an agent.md"),
    ),
    (
        "03 Reference/HTTP API reference.md",
        include_str!("../docs/user-vault/03 Reference/HTTP API reference.md"),
    ),
    (
        "03 Reference/Markdown feature showcase.md",
        include_str!("../docs/user-vault/03 Reference/Markdown feature showcase.md"),
    ),
    (
        "03 Reference/MCP tools reference.md",
        include_str!("../docs/user-vault/03 Reference/MCP tools reference.md"),
    ),
    (
        "03 Reference/Settings and environment variables reference.md",
        include_str!(
            "../docs/user-vault/03 Reference/Settings and environment variables reference.md"
        ),
    ),
    (
        "03 Reference/Supported Markdown reference.md",
        include_str!("../docs/user-vault/03 Reference/Supported Markdown reference.md"),
    ),
    (
        "03 Reference/The LLM wiki pattern (external reference).md",
        include_str!(
            "../docs/user-vault/03 Reference/The LLM wiki pattern (external reference).md"
        ),
    ),
    (
        "03 Reference/The PARA method (external reference).md",
        include_str!("../docs/user-vault/03 Reference/The PARA method (external reference).md"),
    ),
    (
        "03 Reference/The Second Brain method (external reference).md",
        include_str!(
            "../docs/user-vault/03 Reference/The Second Brain method (external reference).md"
        ),
    ),
    (
        "03 Reference/The Zettelkasten method (external reference).md",
        include_str!(
            "../docs/user-vault/03 Reference/The Zettelkasten method (external reference).md"
        ),
    ),
    (
        "04 Concepts/How indexing and search work.md",
        include_str!("../docs/user-vault/04 Concepts/How indexing and search work.md"),
    ),
    (
        "04 Concepts/The layer system.md",
        include_str!("../docs/user-vault/04 Concepts/The layer system.md"),
    ),
    (
        "04 Concepts/The security model.md",
        include_str!("../docs/user-vault/04 Concepts/The security model.md"),
    ),
    (
        "04 Concepts/Vault lifecycle states.md",
        include_str!("../docs/user-vault/04 Concepts/Vault lifecycle states.md"),
    ),
    (
        "04 Concepts/What Hatchdoor is.md",
        include_str!("../docs/user-vault/04 Concepts/What Hatchdoor is.md"),
    ),
    (
        "04 Concepts/Why keep a second brain.md",
        include_str!("../docs/user-vault/04 Concepts/Why keep a second brain.md"),
    ),
    ("Home.md", include_str!("../docs/user-vault/Home.md")),
    (
        "What's new.md",
        include_str!("../docs/user-vault/What's new.md"),
    ),
];

/// The page an index starts from.
const HOME_PATH: &str = "Home.md";

/// How many pages `search` returns at most.
pub const SEARCH_RESULTS: usize = 5;

/// How long an excerpt may grow, in characters, before it is cut.
const EXCERPT_CHARS: usize = 240;

/// Query words too common in the manual to say anything about a page. They
/// are dropped from a query unless nothing else is left.
const STOP_WORDS: &[&str] = &[
    "a", "an", "and", "are", "be", "can", "do", "does", "for", "how", "i", "in", "is", "it", "my",
    "of", "on", "or", "the", "to", "what", "with",
];

/// One page of the manual, with its wikilinks rewritten to page names.
#[derive(Debug)]
pub struct ManualPage {
    /// The page's stable address, e.g. `guides/how-to-set-up-a-git-backed-vault`.
    pub name: String,
    /// The page's first `# ` heading, or its file name when it has none.
    pub title: String,
    /// The page without its frontmatter, every wikilink outside code
    /// rewritten to a Markdown link whose target is a page name.
    pub markdown: String,
    /// Set by `private: true` in the page's frontmatter. A private page is
    /// kept off the public `/docs/` addresses and `llms.txt` (ADR-38), but
    /// signed-in Help and the MCP docs tools still serve it.
    pub private: bool,
    /// The page without its frontmatter, wikilinks as written, so
    /// [`Manual::markdown_linking`] can point them somewhere else.
    body: &'static str,
}

/// One search answer: the page and a line of it that matched.
#[derive(Debug)]
pub struct ManualSearchHit<'a> {
    pub page: &'a ManualPage,
    pub excerpt: String,
}

/// Every page of the manual, ready to read and search.
pub struct Manual {
    /// Home first, then every other page in path order.
    pages: Vec<ManualPage>,
    words: Vec<PageWords>,
    targets: LinkTargets,
}

/// The words of one page, normalized for matching.
struct PageWords {
    title: HashSet<String>,
    /// How many headings outside code carry each word.
    headings: HashMap<String, usize>,
    /// How often each word appears anywhere in the page, code included, so
    /// a setting or command name finds the page that shows it.
    body: HashMap<String, usize>,
}

/// How well one page matches a query, compared field by field: any page
/// with more query words in its title ranks above every page with fewer,
/// then heading matches decide, then body text.
#[derive(Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct Relevance {
    title_words: usize,
    heading_words: usize,
    body_hits: usize,
}

static MANUAL: LazyLock<Manual> = LazyLock::new(|| Manual::from_sources(PAGES));

/// The bundled manual.
pub fn manual() -> &'static Manual {
    &MANUAL
}

/// Every page, Home first.
pub fn pages() -> &'static [ManualPage] {
    MANUAL.pages()
}

/// The manual's front page.
pub fn home() -> &'static ManualPage {
    MANUAL.home()
}

/// The page with this name; see [`Manual::page`].
pub fn page(name: &str) -> Option<&'static ManualPage> {
    MANUAL.page(name)
}

/// The best pages for `query`, private pages included; see
/// [`Manual::search`].
pub fn search(query: &str) -> Vec<ManualSearchHit<'static>> {
    MANUAL.search(query, true)
}

/// A page's name, from its path under `docs/user-vault`: each folder loses
/// its ordering number (`01 Get started` becomes `get-started`), and every
/// segment goes through the note slug rule, so `03 Reference/MCP tools
/// reference.md` is `reference/mcp-tools-reference`.
fn page_name(path: &str) -> String {
    let path = path.strip_suffix(".md").unwrap_or(path);
    path.split('/')
        .map(|segment| {
            let unnumbered = segment.trim_start_matches(|c: char| c.is_ascii_digit());
            let unnumbered = if unnumbered.len() < segment.len() {
                unnumbered.trim_start()
            } else {
                segment
            };
            slugify(unnumbered)
        })
        .collect::<Vec<_>>()
        .join("/")
}

impl Manual {
    /// A manual of these `(path, source)` pages. `Home.md` must be one of
    /// them. The bundled manual is built from [`PAGES`]; tests build their
    /// own from fixture pages.
    pub(crate) fn from_sources(sources: &[(&str, &'static str)]) -> Self {
        let mut sources: Vec<&(&str, &'static str)> = sources.iter().collect();
        sources.sort_by_key(|(path, _)| (*path != HOME_PATH, *path));
        assert_eq!(sources.first().map(|(path, _)| *path), Some(HOME_PATH));
        let targets = LinkTargets::new(sources.iter().map(|(path, _)| *path));
        let mut pages = Vec::with_capacity(sources.len());
        let mut words = Vec::with_capacity(sources.len());
        for (path, source) in sources {
            let name = page_name(path);
            let body = without_frontmatter(source);
            let (markdown, _unresolved) = rewrite_wikilinks(body, &name, &targets);
            let title = first_title(&markdown).unwrap_or_else(|| file_stem(path).to_string());
            words.push(PageWords::new(&title, &markdown));
            pages.push(ManualPage {
                name,
                title,
                markdown,
                private: is_private(source),
                body,
            });
        }
        Self {
            pages,
            words,
            targets,
        }
    }

    /// Every page, Home first.
    pub fn pages(&self) -> &[ManualPage] {
        &self.pages
    }

    /// The manual's front page.
    pub fn home(&self) -> &ManualPage {
        &self.pages[0]
    }

    /// The page with this name, ignoring case, surrounding slashes and any
    /// `#heading` fragment, so a link target copied out of a page resolves
    /// too.
    pub fn page(&self, name: &str) -> Option<&ManualPage> {
        let name = name.split('#').next().unwrap_or_default();
        let name = name.trim().trim_matches('/');
        self.pages
            .iter()
            .find(|page| page.name.eq_ignore_ascii_case(name))
    }

    /// The best [`SEARCH_RESULTS`] pages for `query`. Unless
    /// `include_private`, private pages are left out, and so are links to
    /// them in the excerpts. A page matches when any query word
    /// appears in it; see [`Relevance`] for the order. A query that matches
    /// nothing returns nothing.
    pub fn search(&self, query: &str, include_private: bool) -> Vec<ManualSearchHit<'_>> {
        let terms = query_terms(query);
        if terms.is_empty() {
            return Vec::new();
        }
        let mut ranked: Vec<(Relevance, usize)> = self
            .words
            .iter()
            .enumerate()
            .filter(|(index, _)| include_private || !self.pages[*index].private)
            .map(|(index, words)| (words.relevance(&terms), index))
            .filter(|(relevance, _)| *relevance != Relevance::default())
            .collect();
        // Most relevant first; equals keep the manual's own order.
        ranked.sort_by(|left, right| right.0.cmp(&left.0).then(left.1.cmp(&right.1)));
        ranked
            .into_iter()
            .take(SEARCH_RESULTS)
            .map(|(_, index)| {
                let page = &self.pages[index];
                let excerpt = if include_private {
                    excerpt(&page.markdown, &terms)
                } else {
                    // A link to a private page must not name it.
                    let markdown = self.markdown_linking(page, |target, anchor| {
                        (!target.private).then(|| page_address(&target.name, anchor))
                    });
                    excerpt(&markdown, &terms)
                };
                ManualSearchHit { page, excerpt }
            })
            .collect()
    }

    /// `page`'s Markdown with each wikilink pointing where `link` says.
    /// `link` gets the target page and the heading anchor, if any, and
    /// returns the link destination, or `None` to leave only the link's
    /// text. [`ManualPage::markdown`] is this with the page name as the
    /// destination.
    pub fn markdown_linking(
        &self,
        page: &ManualPage,
        link: impl Fn(&ManualPage, Option<&str>) -> Option<String>,
    ) -> String {
        let (markdown, _unresolved) =
            rewrite_wikilinks_with(page.body, &page.name, &self.targets, |name, anchor| {
                self.page(name).and_then(|target| link(target, anchor))
            });
        markdown
    }
}

/// A link destination for a page name and an optional heading anchor.
fn page_address(name: &str, anchor: Option<&str>) -> String {
    match anchor {
        Some(anchor) => format!("{name}#{anchor}"),
        None => name.to_string(),
    }
}

/// Whether `source`'s frontmatter carries `private: true`.
fn is_private(source: &str) -> bool {
    parse_frontmatter_metadata(source).is_ok_and(|metadata| {
        metadata.properties.get("private") == Some(&serde_json::Value::Bool(true))
    })
}

impl PageWords {
    fn new(title: &str, markdown: &str) -> Self {
        let mut headings: HashMap<String, usize> = HashMap::new();
        let mut body: HashMap<String, usize> = HashMap::new();
        for (line, in_code) in fenced_lines(markdown) {
            if !in_code && let Some(heading) = heading_text(line) {
                let distinct: HashSet<String> = words(heading).collect();
                for word in distinct {
                    *headings.entry(word).or_default() += 1;
                }
            }
            for word in words(line) {
                *body.entry(word).or_default() += 1;
            }
        }
        Self {
            title: words(title).collect(),
            headings,
            body,
        }
    }

    fn relevance(&self, terms: &[String]) -> Relevance {
        Relevance {
            title_words: terms
                .iter()
                .filter(|term| self.title.contains(*term))
                .count(),
            heading_words: terms
                .iter()
                .filter(|term| self.headings.contains_key(*term))
                .count(),
            body_hits: terms
                .iter()
                .map(|term| self.body.get(term).copied().unwrap_or(0))
                .sum(),
        }
    }
}

/// The normalized words of `text`: lowercased runs of letters and digits,
/// with a trailing plural `s` dropped so `vault` finds `Vaults`.
fn words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(normalize_word)
}

fn normalize_word(word: &str) -> String {
    let word = word.to_lowercase();
    match word.strip_suffix('s') {
        Some(stem) if stem.chars().count() >= 3 && !stem.ends_with('s') => stem.to_string(),
        _ => word,
    }
}

/// The distinct normalized words of a query, without stop words unless
/// nothing else is left. Stop words are matched before normalizing, so
/// `does` is dropped rather than folded into `doe`.
fn query_terms(query: &str) -> Vec<String> {
    let mut all: Vec<String> = Vec::new();
    let mut meaningful: Vec<String> = Vec::new();
    for raw in query.split(|c: char| !c.is_alphanumeric()) {
        if raw.is_empty() {
            continue;
        }
        let word = normalize_word(raw);
        if !STOP_WORDS.contains(&raw.to_lowercase().as_str()) && !meaningful.contains(&word) {
            meaningful.push(word.clone());
        }
        if !all.contains(&word) {
            all.push(word);
        }
    }
    if meaningful.is_empty() {
        all
    } else {
        meaningful
    }
}

/// The first prose line of `markdown` that carries a query word, then the
/// first such heading, cut to [`EXCERPT_CHARS`] around the match. A page
/// that matched only in code or on its title gets its first line of prose.
fn excerpt(markdown: &str, terms: &[String]) -> String {
    let mut first_prose: Option<&str> = None;
    let mut first_heading: Option<(&str, usize)> = None;
    for (line, in_code) in fenced_lines(markdown) {
        let trimmed = line.trim();
        if in_code || trimmed.is_empty() {
            continue;
        }
        let is_heading = heading_text(line).is_some();
        if !is_heading && first_prose.is_none() {
            first_prose = Some(trimmed);
        }
        let Some(at) = first_match(trimmed, terms) else {
            continue;
        };
        if !is_heading {
            return clip(trimmed, at);
        }
        if first_heading.is_none() {
            first_heading = Some((trimmed, at));
        }
    }
    match (first_heading, first_prose) {
        (Some((line, at)), _) => clip(line, at),
        (None, Some(line)) => clip(line, 0),
        (None, None) => String::new(),
    }
}

/// The byte offset of the first word in `line` that matches a term.
fn first_match(line: &str, terms: &[String]) -> Option<usize> {
    line.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .find(|word| terms.contains(&normalize_word(word)))
        .map(|word| word.as_ptr() as usize - line.as_ptr() as usize)
}

/// `line` cut to [`EXCERPT_CHARS`] characters, starting a little before
/// byte `at` so the match has some context, with `…` where it was cut.
fn clip(line: &str, at: usize) -> String {
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    if chars.len() <= EXCERPT_CHARS {
        return line.to_string();
    }
    let match_char = chars.iter().position(|(byte, _)| *byte >= at).unwrap_or(0);
    let start = match_char
        .saturating_sub(EXCERPT_CHARS / 3)
        .min(chars.len() - EXCERPT_CHARS);
    let end = start + EXCERPT_CHARS;
    let from = chars[start].0;
    let to = chars.get(end).map_or(line.len(), |(byte, _)| *byte);
    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    out.push_str(line[from..to].trim());
    if end < chars.len() {
        out.push('…');
    }
    out
}

/// The text of an ATX heading line, without its `#` marks. The line must
/// start with the marks, so an indented `# comment` is not a heading.
fn heading_text(line: &str) -> Option<&str> {
    let hashes = line.len() - line.trim_start_matches('#').len();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    line[hashes..]
        .strip_prefix(' ')
        .map(|text| text.trim().trim_end_matches('#').trim())
}

/// Each line of `markdown`, line ending kept, and whether it belongs to a
/// fenced code block, the fence lines themselves included.
fn fenced_lines(markdown: &str) -> impl Iterator<Item = (&str, bool)> {
    let mut fence: Option<(u8, usize)> = None;
    markdown.split_inclusive('\n').map(move |line| {
        let marker = parse_fence_marker(line.trim_start());
        match (fence, marker) {
            (Some(open), Some((marker, len))) if marker == open.0 && len >= open.1 => {
                fence = None;
            }
            (Some(_), _) => {}
            (None, Some(marker)) => fence = Some(marker),
            (None, None) => return (line, false),
        }
        (line, true)
    })
}

fn first_title(markdown: &str) -> Option<String> {
    fenced_lines(markdown)
        .filter(|(_, in_code)| !in_code)
        .find_map(|(line, _)| line.strip_prefix("# "))
        .map(|title| title.trim().to_string())
}

fn file_stem(path: &str) -> &str {
    let file = path.rsplit('/').next().unwrap_or(path);
    file.strip_suffix(".md").unwrap_or(file)
}

/// `source` without a leading frontmatter block or the blank lines after it.
fn without_frontmatter(source: &str) -> &str {
    let Some((_, end)) = frontmatter_span(source) else {
        return source;
    };
    // `end` sits on the newline before the closing `---`; the body starts on
    // the line after it.
    let closing = &source[end + 1..];
    let body = closing.split_once('\n').map_or("", |(_, rest)| rest);
    body.trim_start_matches(['\n', '\r'])
}

/// Where a wikilink's page part can point: a page's file name, or its path
/// under `docs/user-vault`, both without `.md` and compared ignoring case.
struct LinkTargets {
    by_key: HashMap<String, String>,
}

impl LinkTargets {
    fn new<'a>(paths: impl Iterator<Item = &'a str>) -> Self {
        let mut by_key = HashMap::new();
        for path in paths {
            let name = page_name(path);
            let without_ext = path.strip_suffix(".md").unwrap_or(path);
            by_key.insert(without_ext.to_lowercase(), name.clone());
            by_key.insert(file_stem(path).to_lowercase(), name);
        }
        Self { by_key }
    }

    fn resolve(&self, target: &str) -> Option<&str> {
        let target = target.trim().trim_start_matches('/');
        let target = target.strip_suffix(".md").unwrap_or(target);
        self.by_key.get(&target.to_lowercase()).map(String::as_str)
    }
}

/// `markdown` with every wikilink outside code rewritten to a Markdown link
/// whose target is a page name, plus a `#heading` anchor when the link names
/// a heading. `[[Page]]` becomes `[Page](name)`, `[[Page|label]]` becomes
/// `[label](name)`, `[[Page#Heading]]` becomes `[Page > Heading](name#heading)`
/// and `[[#Heading]]` points into `this_page`. Embeds (`![[...]]`) and links
/// that name no bundled page are left as written and returned in the second
/// value, so a test can insist there are none.
///
/// The write layer's wikilink rewrite keeps the `[[...]]` form and changes
/// only the target, so it cannot produce a Markdown link; this is the
/// manual's own.
fn rewrite_wikilinks(
    markdown: &str,
    this_page: &str,
    targets: &LinkTargets,
) -> (String, Vec<String>) {
    rewrite_wikilinks_with(markdown, this_page, targets, |name, anchor| {
        Some(page_address(name, anchor))
    })
}

/// [`rewrite_wikilinks`] with the destination chosen by `destination`, which
/// gets the page name and heading anchor and returns `None` to keep only the
/// link's text.
fn rewrite_wikilinks_with(
    markdown: &str,
    this_page: &str,
    targets: &LinkTargets,
    destination: impl Fn(&str, Option<&str>) -> Option<String>,
) -> (String, Vec<String>) {
    let mut out = String::with_capacity(markdown.len());
    let mut unresolved = Vec::new();
    for (line, in_code) in fenced_lines(markdown) {
        if in_code {
            out.push_str(line);
        } else {
            rewrite_line(
                line,
                this_page,
                targets,
                &destination,
                &mut out,
                &mut unresolved,
            );
        }
    }
    (out, unresolved)
}

/// One prose line of [`rewrite_wikilinks`]: copies `line` to `out`, rewriting
/// each wikilink that sits outside an inline code span.
fn rewrite_line(
    line: &str,
    this_page: &str,
    targets: &LinkTargets,
    destination: &dyn Fn(&str, Option<&str>) -> Option<String>,
    out: &mut String,
    unresolved: &mut Vec<String>,
) {
    let bytes = line.as_bytes();
    let mut code_ticks = 0usize;
    let mut copied = 0usize;
    let mut at = 0usize;
    while at < bytes.len() {
        if bytes[at] == b'`' {
            let run = bytes[at..].iter().take_while(|byte| **byte == b'`').count();
            if code_ticks == 0 {
                code_ticks = run;
            } else if run == code_ticks {
                code_ticks = 0;
            }
            at += run;
            continue;
        }
        if code_ticks == 0 && bytes[at..].starts_with(b"[[") {
            let Some(close) = line[at + 2..].find("]]") else {
                break;
            };
            let inner = &line[at + 2..at + 2 + close];
            let end = at + 2 + close + 2;
            let embed = at > 0 && bytes[at - 1] == b'!';
            match (!embed)
                .then(|| link_markdown(inner, this_page, targets, destination))
                .flatten()
            {
                Some(link) => {
                    out.push_str(&line[copied..at]);
                    out.push_str(&link);
                    copied = end;
                }
                None => unresolved.push(line[at..end].to_string()),
            }
            at = end;
            continue;
        }
        at += 1;
    }
    out.push_str(&line[copied..]);
}

/// The Markdown link for one wikilink's inner text, or `None` when it names
/// no bundled page.
fn link_markdown(
    inner: &str,
    this_page: &str,
    targets: &LinkTargets,
    destination: &dyn Fn(&str, Option<&str>) -> Option<String>,
) -> Option<String> {
    // Inside a table the alias pipe is written `\|`.
    let (target, alias) = match inner.split_once('|') {
        Some((target, alias)) => (
            target.strip_suffix('\\').unwrap_or(target),
            Some(alias.trim()),
        ),
        None => (inner, None),
    };
    let (page_part, heading) = match target.split_once('#') {
        Some((page_part, heading)) => (page_part.trim(), Some(heading.trim())),
        None => (target.trim(), None),
    };
    let name = if page_part.is_empty() {
        this_page
    } else {
        targets.resolve(page_part)?
    };
    let label = match (alias, heading) {
        (Some(alias), _) if !alias.is_empty() => alias.to_string(),
        (_, Some(heading)) if page_part.is_empty() => heading.to_string(),
        (_, Some(heading)) => format!("{page_part} > {heading}"),
        _ => page_part.to_string(),
    };
    let anchor = match heading {
        Some(heading) if !heading.is_empty() => Some(slugify(heading)),
        _ => None,
    };
    Some(match destination(name, anchor.as_deref()) {
        Some(destination) => format!("[{label}]({destination})"),
        None => label,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::Path;

    fn manual_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/user-vault")
    }

    #[test]
    fn every_manual_page_is_bundled() {
        let mut on_disk: Vec<String> = walkdir::WalkDir::new(manual_root())
            .into_iter()
            .map(|entry| entry.expect("readable manual folder"))
            .filter(|entry| entry.file_type().is_file())
            .map(|entry| {
                entry
                    .path()
                    .strip_prefix(manual_root())
                    .expect("inside the manual")
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .filter(|path| path.ends_with(".md"))
            .collect();
        on_disk.sort();
        let mut bundled: Vec<String> = PAGES.iter().map(|(path, _)| path.to_string()).collect();
        bundled.sort();
        assert_eq!(
            bundled, on_disk,
            "every Markdown page under docs/user-vault must be listed in PAGES, and nothing else"
        );
    }

    #[test]
    fn only_markdown_is_bundled() {
        for (path, _) in PAGES {
            assert!(path.ends_with(".md"), "{path} is not a Markdown page");
        }
    }

    #[test]
    fn page_names_are_unique_and_address_safe() {
        let mut seen = HashSet::new();
        for page in pages() {
            assert!(
                seen.insert(page.name.clone()),
                "{} is named twice",
                page.name
            );
            assert!(
                page.name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '/'),
                "{} is not address-safe",
                page.name
            );
        }
    }

    #[test]
    fn a_page_name_drops_folder_numbers_and_slugs_each_segment() {
        assert_eq!(
            page_name("01 Get started/Install Hatchdoor with Docker Compose.md"),
            "get-started/install-hatchdoor-with-docker-compose"
        );
        assert_eq!(
            page_name("03 Reference/The PARA method (external reference).md"),
            "reference/the-para-method-external-reference"
        );
        assert_eq!(page_name("Home.md"), "home");
    }

    #[test]
    fn home_comes_first_and_every_page_has_a_title() {
        assert_eq!(home().name, "home");
        assert_eq!(home().title, "Hatchdoor documentation");
        assert_eq!(pages().len(), PAGES.len());
        for page in pages() {
            assert!(!page.title.is_empty(), "{} has no title", page.name);
            assert!(
                !page.markdown.starts_with("---"),
                "{} kept its frontmatter",
                page.name
            );
        }
    }

    #[test]
    fn every_link_on_every_page_resolves_to_a_bundled_page() {
        let targets = LinkTargets::new(PAGES.iter().map(|(path, _)| *path));
        for (path, source) in PAGES {
            let (markdown, unresolved) =
                rewrite_wikilinks(without_frontmatter(source), &page_name(path), &targets);
            assert!(
                unresolved.is_empty(),
                "{path} has wikilinks that name no bundled page: {unresolved:?}"
            );
            for destination in link_destinations(&markdown) {
                let (name, anchor) = destination
                    .split_once('#')
                    .unwrap_or((destination.as_str(), ""));
                if destination.contains("://") || name.is_empty() {
                    continue;
                }
                let target = page(name).unwrap_or_else(|| {
                    panic!("{path} links to {destination}, which is not a bundled page")
                });
                if !anchor.is_empty() {
                    assert!(
                        target
                            .markdown
                            .lines()
                            .filter_map(heading_text)
                            .any(|heading| slugify(heading) == anchor),
                        "{path} links to {destination}, but {} has no such heading",
                        target.name
                    );
                }
            }
        }
    }

    /// The destination of every inline Markdown link outside code.
    fn link_destinations(markdown: &str) -> Vec<String> {
        let mut found = Vec::new();
        crate::cache::parse::for_non_code_line(markdown, |line| {
            let mut rest = line;
            while let Some(open) = rest.find("](") {
                let after = &rest[open + 2..];
                let Some(close) = after.find(')') else { break };
                found.push(after[..close].to_string());
                rest = &after[close..];
            }
        });
        found
    }

    #[test]
    fn wikilinks_become_links_to_page_names() {
        let targets = LinkTargets::new(
            [
                "02 Guides/How to set up a Git-backed Vault.md",
                "04 Concepts/The layer system.md",
            ]
            .into_iter(),
        );
        let (markdown, unresolved) = rewrite_wikilinks(
            "See [[How to set up a Git-backed Vault]], [[The layer system|layers]], \
             [[The layer system#Demoted layers]] and [[#Upward]].\n\
             | a | [[The layer system\\|layer]] |\n",
            "concepts/here",
            &targets,
        );
        assert!(unresolved.is_empty());
        assert_eq!(
            markdown,
            "See [How to set up a Git-backed Vault](guides/how-to-set-up-a-git-backed-vault), \
             [layers](concepts/the-layer-system), \
             [The layer system > Demoted layers](concepts/the-layer-system#demoted-layers) \
             and [Upward](concepts/here#upward).\n\
             | a | [layer](concepts/the-layer-system) |\n"
        );
    }

    #[test]
    fn wikilinks_inside_code_are_left_as_written() {
        let targets = LinkTargets::new(["Home.md"].into_iter());
        let source = "Write `[[Home]]` like this:\n\n```md\n[[Home]]\n```\n\n![[Home]]\n";
        let (markdown, unresolved) = rewrite_wikilinks(source, "home", &targets);
        assert_eq!(markdown, source);
        assert_eq!(
            unresolved,
            vec!["[[Home]]".to_string()],
            "the embed is reported"
        );
    }

    #[test]
    fn a_link_to_a_missing_page_is_reported_and_left_alone() {
        let targets = LinkTargets::new(["Home.md"].into_iter());
        let (markdown, unresolved) = rewrite_wikilinks("[[Nowhere]]\n", "home", &targets);
        assert_eq!(markdown, "[[Nowhere]]\n");
        assert_eq!(unresolved, vec!["[[Nowhere]]".to_string()]);
    }

    #[test]
    fn a_page_is_found_by_name_ignoring_case_and_fragment() {
        let name = "guides/how-to-set-up-a-git-backed-vault";
        assert_eq!(page(name).expect("page").name, name);
        assert_eq!(
            page("Guides/How-To-Set-Up-A-Git-Backed-Vault#remote")
                .expect("page")
                .name,
            name
        );
        assert!(page("guides/no-such-page").is_none());
    }

    #[test]
    fn searching_git_finds_the_git_guide_first() {
        let hits = search("git");
        assert_eq!(
            hits.first().expect("a hit").page.name,
            "guides/how-to-set-up-a-git-backed-vault"
        );
        assert!(hits.len() <= SEARCH_RESULTS);
        assert!(
            hits.iter().all(|hit| !hit.excerpt.is_empty()),
            "every hit carries an excerpt"
        );
    }

    #[test]
    fn a_search_with_no_match_is_empty() {
        assert!(search("zzyzzyva").is_empty());
        assert!(search("   ").is_empty());
    }

    #[test]
    fn search_ignores_case_and_plurals() {
        let lower = search("vault");
        let upper = search("VAULTS");
        assert!(!lower.is_empty());
        assert_eq!(
            lower.iter().map(|hit| &hit.page.name).collect::<Vec<_>>(),
            upper.iter().map(|hit| &hit.page.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn title_hits_rank_above_body_hits() {
        // "security" is in one title and in many bodies.
        let hits = search("security");
        assert_eq!(hits[0].page.name, "concepts/the-security-model");
        assert!(hits.len() > 1);
    }

    #[test]
    fn stop_words_do_not_decide_a_query() {
        assert_eq!(query_terms("how do I set up git"), vec!["set", "up", "git"]);
        assert_eq!(query_terms("does it sync"), vec!["sync"]);
        assert_eq!(query_terms("how to"), vec!["how", "to"]);
    }

    #[test]
    fn search_never_returns_more_than_its_cap() {
        assert_eq!(search("hatchdoor").len(), SEARCH_RESULTS);
    }

    #[test]
    fn a_heading_match_outranks_any_amount_of_body_text() {
        let headed = PageWords::new("Alpha", "# Alpha\n\n## Remote\n\nNothing else.\n");
        let wordy = PageWords::new(
            "Beta",
            "# Beta\n\nremote sync push remote sync push remote sync push\n",
        );
        let terms = query_terms("remote sync push");
        assert!(headed.relevance(&terms) > wordy.relevance(&terms));
    }

    #[test]
    fn a_title_match_outranks_any_heading_or_body_match() {
        let titled = PageWords::new("Remote", "# Remote\n\nShort.\n");
        let headed = PageWords::new(
            "Other",
            "# Other\n\n## Remote sync\n\n## Remote push\n\nremote remote remote\n",
        );
        let terms = query_terms("remote");
        assert!(titled.relevance(&terms) > headed.relevance(&terms));
    }

    #[test]
    fn code_lines_are_neither_headings_nor_excerpts() {
        let markdown =
            "# Page\n\n```yaml\n# Safe default for the port\nport: 1\n```\n\nSet the port here.\n";
        let words = PageWords::new("Page", markdown);
        assert!(!words.headings.contains_key("safe"));
        assert_eq!(
            words.body.get("port").copied(),
            Some(3),
            "code still counts as body text"
        );
        assert_eq!(
            excerpt(markdown, &query_terms("port")),
            "Set the port here."
        );
    }

    #[test]
    fn a_long_line_is_clipped_around_its_match() {
        let line = format!("{} needle {}", "word ".repeat(100), "tail ".repeat(100));
        let at = line.find("needle").expect("needle");
        let clipped = clip(&line, at);
        assert!(clipped.contains("needle"));
        assert!(clipped.starts_with('…') && clipped.ends_with('…'));
        assert!(clipped.chars().count() <= EXCERPT_CHARS + 2);
    }
}
