//! The link preview a demo instance writes into the page it serves (ADR-47):
//! the tags a chat app or search engine reads, and the short description of a
//! note they carry.

use unicode_normalization::char::is_combining_mark;

use crate::chunk::normalize::strip_frontmatter;
use crate::vault::Note;

/// Where the built frontend serves the one preview picture (ADR-47 decision 2).
const PICTURE_PATH: &str = "/link-preview.png";
const PICTURE_WIDTH: u32 = 1200;
const PICTURE_HEIGHT: u32 = 630;
const PICTURE_ALT: &str = "The Hatchdoor mark and wordmark";

const SITE_NAME: &str = "Hatchdoor";
const GENERAL_DESCRIPTION: &str = "Self-host your Obsidian vaults: a web UI for you, MCP for your AI agents. No Obsidian, no plugins required.";

/// The longest description sent, in characters, ellipsis included.
const DESCRIPTION_CHARS: usize = 200;
const ELLIPSIS: char = '…';

/// What one address previews as.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct LinkPreview {
    /// The `<title>` element, which names the tab and the search result.
    page_title: String,
    /// `og:title`. The site name travels separately in `og:site_name`.
    title: String,
    description: String,
    kind: &'static str,
}

impl LinkPreview {
    /// The wording every address but a readable note gets.
    pub(super) fn general(path: &str) -> Self {
        let page_title = match path {
            "/graph" => format!("Graph · {SITE_NAME}"),
            "/stats" => format!("Stats · {SITE_NAME}"),
            "/settings" => format!("Settings · {SITE_NAME}"),
            _ => SITE_NAME.to_string(),
        };
        Self {
            page_title,
            title: SITE_NAME.to_string(),
            description: GENERAL_DESCRIPTION.to_string(),
            kind: "website",
        }
    }

    /// A note's own title, and its `description` property or opening prose.
    /// A note with neither keeps the general description. `properties` are
    /// the note's frontmatter properties, which a note read does not carry.
    pub(super) fn for_note(
        note: &Note,
        properties: &serde_json::Map<String, serde_json::Value>,
    ) -> Self {
        let title = collapse_whitespace(&note.title);
        if title.is_empty() {
            return Self::general("/");
        }
        Self {
            page_title: format!("{title} · {SITE_NAME}"),
            title,
            description: note_description(&note.content, properties)
                .unwrap_or_else(|| GENERAL_DESCRIPTION.to_string()),
            kind: "article",
        }
    }

    /// The built page with this preview's tags in place of its `<title>`.
    ///
    /// `public_url` is the operator's `HATCHDOOR_PUBLIC_URL`. The picture and
    /// the page's own address need an absolute address, so both are left out
    /// without it (ADR-47 decision 5). `path` is the path the request named.
    pub(super) fn write_into(&self, index: &str, public_url: Option<&str>, path: &str) -> String {
        let mut tags = vec![
            format!("<title>{}</title>", escape(&self.page_title)),
            meta("name", "description", &self.description),
            meta("property", "og:title", &self.title),
            meta("property", "og:description", &self.description),
            meta("property", "og:type", self.kind),
            meta("property", "og:site_name", SITE_NAME),
        ];
        match public_url {
            Some(base) => tags.extend([
                meta("property", "og:url", &format!("{base}{path}")),
                meta("property", "og:image", &format!("{base}{PICTURE_PATH}")),
                meta("property", "og:image:width", &PICTURE_WIDTH.to_string()),
                meta("property", "og:image:height", &PICTURE_HEIGHT.to_string()),
                meta("property", "og:image:alt", PICTURE_ALT),
                meta("name", "twitter:card", "summary_large_image"),
            ]),
            None => tags.push(meta("name", "twitter:card", "summary")),
        }
        let tags = tags.join("\n    ");

        let title = index
            .find("<title>")
            .and_then(|start| Some((start, start + index[start..].find("</title>")? + 8)));
        match (title, index.find("</head>")) {
            (Some((start, end)), _) => format!("{}{tags}{}", &index[..start], &index[end..]),
            (None, Some(head_end)) => {
                format!("{}{tags}\n  {}", &index[..head_end], &index[head_end..])
            }
            (None, None) => index.to_string(),
        }
    }
}

fn meta(attribute: &str, name: &str, content: &str) -> String {
    format!(
        "<meta {attribute}=\"{name}\" content=\"{}\" />",
        escape(content)
    )
}

/// Escape `text` for an HTML element's text or a quoted attribute value
/// (ADR-47 decision 4). One table serves both, so no caller can pick the
/// wrong one.
fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            other => escaped.push(other),
        }
    }
    escaped
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The note's `description` property when it is a non-empty string, otherwise
/// its opening prose. Either is reduced to plain text and cut to the limit.
fn note_description(
    content: &str,
    properties: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    let property = properties
        .get("description")
        .and_then(serde_json::Value::as_str)
        .map(plain_text)
        .filter(|description| !description.is_empty());
    let description = property.or_else(|| opening_prose(content))?;
    Some(cut(&description))
}

/// The first paragraph of body text, as plain text.
///
/// Headings, fenced code and diagrams, math blocks, tables, rules, comments,
/// images and link definitions are passed over until prose turns up. A quote
/// or callout counts as prose, as does a list, whose items are joined.
fn opening_prose(content: &str) -> Option<String> {
    let mut paragraph: Vec<String> = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    let mut in_math = false;
    let mut in_table = false;
    let mut comment = Comment::None;
    let mut pushed = false;

    for line in strip_frontmatter(content).lines() {
        let previous_line_pushed = std::mem::take(&mut pushed);
        let line = strip_quote_markers(line.trim());

        if let Some((marker, length)) = fence {
            if closes_fence(line, marker, length) {
                fence = None;
            }
            continue;
        }
        if in_math {
            in_math = !line.ends_with("$$");
            continue;
        }
        let line = strip_comments(line, &mut comment);
        let line = line.trim();

        if line.is_empty() {
            in_table = false;
            if paragraph.is_empty() {
                continue;
            }
            break;
        }
        if let Some(opened) = opens_fence(line) {
            if !paragraph.is_empty() {
                break;
            }
            fence = Some(opened);
            continue;
        }
        if line.starts_with("$$") {
            if !paragraph.is_empty() {
                break;
            }
            in_math = line.len() == 2 || !line.ends_with("$$");
            continue;
        }
        if is_table_separator(line) {
            // The line above was this table's header, not prose.
            if previous_line_pushed {
                paragraph.pop();
            }
            if !paragraph.is_empty() {
                break;
            }
            in_table = true;
            continue;
        }
        if in_table || line.starts_with('|') {
            if !paragraph.is_empty() {
                break;
            }
            in_table = true;
            continue;
        }
        if is_setext_underline(line) && !paragraph.is_empty() {
            // The lines above were a heading.
            paragraph.clear();
            continue;
        }
        if is_heading(line) || is_rule(line) || is_definition(line) {
            if !paragraph.is_empty() {
                break;
            }
            continue;
        }

        let text = plain_text(strip_list_marker(line));
        if !text.is_empty() {
            paragraph.push(text);
            pushed = true;
        }
    }

    (!paragraph.is_empty()).then(|| paragraph.join(" "))
}

/// Which kind of comment the walk is inside, if any.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Comment {
    None,
    /// An Obsidian `%%` comment.
    Obsidian,
    /// An HTML `<!--` comment.
    Html,
}

/// `line` without its comments. A comment may open on one line and close on a
/// later one, so the state is carried between calls.
fn strip_comments(line: &str, state: &mut Comment) -> String {
    let mut kept = String::new();
    let mut rest = line;
    loop {
        match *state {
            Comment::None => {
                let obsidian = rest.find("%%").map(|at| (at, 2, Comment::Obsidian));
                let html = rest.find("<!--").map(|at| (at, 4, Comment::Html));
                let opening = match (obsidian, html) {
                    (Some(first), Some(second)) => {
                        Some(if first.0 < second.0 { first } else { second })
                    }
                    (first, second) => first.or(second),
                };
                let Some((at, length, kind)) = opening else {
                    kept.push_str(rest);
                    return kept;
                };
                kept.push_str(&rest[..at]);
                rest = &rest[at + length..];
                *state = kind;
            }
            Comment::Obsidian | Comment::Html => {
                let closing = if *state == Comment::Obsidian {
                    "%%"
                } else {
                    "-->"
                };
                let Some(at) = rest.find(closing) else {
                    return kept;
                };
                rest = &rest[at + closing.len()..];
                *state = Comment::None;
            }
        }
    }
}

/// `line` without its leading `>` markers and, on a callout's first line,
/// without the `[!type]` marker, which leaves the callout's title.
fn strip_quote_markers(line: &str) -> &str {
    let mut rest = line;
    let mut quoted = false;
    while let Some(inner) = rest.strip_prefix('>') {
        rest = inner.trim_start();
        quoted = true;
    }
    if quoted
        && rest.starts_with("[!")
        && let Some(end) = rest.find(']')
    {
        return rest[end + 1..].trim_start_matches(['+', '-']).trim();
    }
    rest
}

fn strip_list_marker(line: &str) -> &str {
    let after_bullet = ["- ", "* ", "+ "]
        .iter()
        .find_map(|bullet| line.strip_prefix(bullet))
        .or_else(|| {
            let digits = line.chars().take_while(char::is_ascii_digit).count();
            let rest = &line[digits..];
            (digits > 0)
                .then(|| rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")))
                .flatten()
        });
    let Some(item) = after_bullet else {
        return line;
    };
    let item = item.trim_start();
    ["[ ] ", "[x] ", "[X] "]
        .iter()
        .find_map(|checkbox| item.strip_prefix(checkbox))
        .unwrap_or(item)
}

fn opens_fence(line: &str) -> Option<(char, usize)> {
    let marker = line.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let length = line.chars().take_while(|c| *c == marker).count();
    (length >= 3).then_some((marker, length))
}

fn closes_fence(line: &str, marker: char, length: usize) -> bool {
    line.chars().all(|c| c == marker) && line.chars().count() >= length
}

fn is_heading(line: &str) -> bool {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    (1..=6).contains(&hashes)
        && line[hashes..]
            .chars()
            .next()
            .is_none_or(char::is_whitespace)
}

fn is_rule(line: &str) -> bool {
    ['-', '*', '_'].iter().any(|marker| {
        line.chars().all(|c| c == *marker || c == ' ')
            && line.chars().filter(|c| c == marker).count() >= 3
    })
}

fn is_setext_underline(line: &str) -> bool {
    line.chars().all(|c| c == '=') || line.chars().all(|c| c == '-')
}

fn is_table_separator(line: &str) -> bool {
    line.contains('|')
        && line.contains('-')
        && line.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
}

/// A footnote or link reference definition: `[^1]: text`, `[name]: address`.
fn is_definition(line: &str) -> bool {
    line.starts_with('[')
        && line
            .find("]:")
            .is_some_and(|end| !line[1..end].contains(['[', ']']))
}

/// One line of Markdown reduced to the words a reader sees: links and
/// wikilinks leave their text, images, embeds, footnote marks and HTML tags
/// leave nothing, and emphasis, code and highlight marks are dropped.
fn plain_text(line: &str) -> String {
    collapse_whitespace(&decode_entities(&without_markup(strip_block_id(line))))
}

/// [`plain_text`] before entities are decoded, so a link's label, which is
/// reduced on its own, is decoded once with the line around it.
fn without_markup(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut text = String::new();
    let mut at = 0;
    while at < chars.len() {
        let character = chars[at];
        let next = chars.get(at + 1).copied();
        let previous = at.checked_sub(1).map(|before| chars[before]);
        match character {
            '\\' if next.is_some_and(|c| c.is_ascii_punctuation()) => {
                text.push(chars[at + 1]);
                at += 2;
            }
            '!' if next == Some('[') => match bracketed(&chars, at + 1) {
                Some((_, end)) => at = end,
                None => {
                    text.push(character);
                    at += 1;
                }
            },
            '[' => match bracketed(&chars, at) {
                Some((shown, end)) => {
                    text.push_str(&shown);
                    at = end;
                }
                None => {
                    text.push(character);
                    at += 1;
                }
            },
            '<' => match angle_bracketed(&chars, at) {
                Some((shown, end)) => {
                    text.push_str(&shown);
                    at = end;
                }
                None => {
                    text.push(character);
                    at += 1;
                }
            },
            '`' => {
                let ticks = chars[at..].iter().take_while(|c| **c == '`').count();
                let inside = at + ticks;
                match find_run(&chars, inside, '`', ticks) {
                    Some(close) => {
                        text.extend(&chars[inside..close]);
                        at = close + ticks;
                    }
                    None => at = inside,
                }
            }
            // An emphasis mark hugs a word on one side only. Between two
            // words (`2*3`, `snake_case`) or between two spaces (`2 * 3`) it
            // is a character the author meant.
            '*' | '_' => {
                if is_literal_mark(previous, next) {
                    text.push(character);
                }
                at += 1;
            }
            '~' | '=' if next == Some(character) => {
                if is_literal_mark(previous, chars.get(at + 2).copied()) {
                    text.push(character);
                    text.push(character);
                }
                at += 2;
            }
            other => {
                text.push(other);
                at += 1;
            }
        }
    }
    text
}

/// Whether a would-be emphasis mark with these neighbours is plain text.
fn is_literal_mark(before: Option<char>, after: Option<char>) -> bool {
    let inside_word =
        before.is_some_and(char::is_alphanumeric) && after.is_some_and(char::is_alphanumeric);
    let spaced = before.is_none_or(char::is_whitespace) && after.is_none_or(char::is_whitespace);
    inside_word || spaced
}

/// `line` without a trailing Obsidian block identifier such as ` ^a1b2c3`.
fn strip_block_id(line: &str) -> &str {
    match line.rsplit_once(" ^") {
        Some((before, id))
            if !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') =>
        {
            before
        }
        _ => line,
    }
}

/// What the bracketed construct opening at `open` shows, and where it ends.
/// `None` when the `[` opens nothing, so it stays as written.
fn bracketed(chars: &[char], open: usize) -> Option<(String, usize)> {
    if chars.get(open + 1) == Some(&'[') {
        let close = find_run(chars, open + 2, ']', 2)?;
        let inner: String = chars[open + 2..close].iter().collect();
        let shown = match inner.split_once('|') {
            Some((_, alias)) => alias.trim().to_string(),
            None => inner.trim().trim_start_matches('#').replace('#', " > "),
        };
        return Some((shown, close + 2));
    }
    let mut depth = 0usize;
    let close = (open..chars.len()).find(|&at| {
        match chars[at] {
            '[' => depth += 1,
            ']' => depth -= 1,
            _ => {}
        }
        depth == 0
    })?;
    let label: String = chars[open + 1..close].iter().collect();
    if label.starts_with('^') {
        // A footnote mark.
        return Some((String::new(), close + 1));
    }
    let end = match chars.get(close + 1) {
        // An address may hold balanced parentheses of its own.
        Some('(') => {
            let mut depth = 0usize;
            (close + 1..chars.len()).find(|&at| {
                match chars[at] {
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    _ => {}
                }
                depth == 0
            })? + 1
        }
        Some('[') => find_run(chars, close + 2, ']', 1)? + 1,
        _ => return None,
    };
    Some((without_markup(&label), end))
}

/// An autolink shows its address; any other tag shows nothing. `None` when
/// the `<` opens neither.
fn angle_bracketed(chars: &[char], open: usize) -> Option<(String, usize)> {
    let first = *chars.get(open + 1)?;
    if !(first.is_ascii_alphabetic() || first == '/') {
        return None;
    }
    let close = find_run(chars, open + 1, '>', 1)?;
    let inner: String = chars[open + 1..close].iter().collect();
    let is_address = ["http://", "https://", "mailto:"]
        .iter()
        .any(|scheme| inner.starts_with(scheme));
    Some((if is_address { inner } else { String::new() }, close + 1))
}

/// The start of the first run of exactly-or-more `length` `marker`s at or
/// after `from`.
fn find_run(chars: &[char], from: usize, marker: char, length: usize) -> Option<usize> {
    (from..chars.len().saturating_sub(length - 1))
        .find(|&at| chars[at..at + length].iter().all(|c| *c == marker))
}

fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    // `&amp;` last, so `&amp;lt;` decodes once, to `&lt;`.
    text.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// `text` within [`DESCRIPTION_CHARS`], ending on a whole word and an
/// ellipsis when something was dropped. Text with no space in its second
/// half (a script written without them, a long address) is cut between characters,
/// never between a letter and its accent or an emoji and its joiners.
fn cut(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= DESCRIPTION_CHARS {
        return text.to_string();
    }
    // One character is kept for the ellipsis.
    let limit = DESCRIPTION_CHARS - 1;
    let mut end = limit;
    if !chars[limit].is_whitespace() {
        // A space in the first half is no place to cut: it would throw most
        // of the text away to save one long word.
        match chars[..limit].iter().rposition(|c| c.is_whitespace()) {
            Some(space) if space >= limit / 2 => end = space,
            _ => {
                while end > 1 && (joins_previous(chars[end]) || chars[end - 1] == '\u{200d}') {
                    end -= 1;
                }
            }
        }
    }
    let kept: String = chars[..end].iter().collect();
    let kept = kept.trim_end_matches(|c: char| c.is_whitespace() || ",;:([{-–".contains(c));
    format!("{kept}{ELLIPSIS}")
}

/// A character that belongs to the one before it.
fn joins_previous(character: char) -> bool {
    is_combining_mark(character)
        || matches!(
            character,
            '\u{200d}' | '\u{fe0f}' | '\u{1f3fb}'..='\u{1f3ff}'
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preview(title: &str, content: &str, properties: serde_json::Value) -> LinkPreview {
        let note = Note {
            title: title.to_string(),
            slug: "note".to_string(),
            relative_path: "note.md".to_string(),
            content: content.to_string(),
            content_hash: String::new(),
            layer: None,
            metadata: Default::default(),
        };
        let properties = properties.as_object().cloned().unwrap_or_default();
        LinkPreview::for_note(&note, &properties)
    }

    fn prose(content: &str) -> Option<String> {
        opening_prose(content)
    }

    const INDEX: &str = "<html>\n  <head>\n    <meta charset=\"UTF-8\" />\n    <title>Hatchdoor</title>\n    <script></script>\n  </head>\n  <body><div id=\"root\"></div></body>\n</html>\n";

    #[test]
    fn the_description_property_wins_over_the_body() {
        let preview = preview(
            "Beacon",
            "---\ndescription: ignored here\n---\n# Beacon\n\nThe body opens here.\n",
            serde_json::json!({ "description": "  A **launch** plan\n for [[Beacon]]. " }),
        );
        assert_eq!(preview.description, "A launch plan for Beacon.");
    }

    #[test]
    fn a_description_that_is_not_a_non_empty_string_falls_back_to_the_body() {
        for properties in [
            serde_json::json!({ "description": "   " }),
            serde_json::json!({ "description": ["a", "b"] }),
            serde_json::json!({ "description": 3 }),
            serde_json::json!({}),
            serde_json::Value::Null,
        ] {
            let preview = preview("Beacon", "# Beacon\n\nThe body opens here.\n", properties);
            assert_eq!(preview.description, "The body opens here.");
        }
    }

    #[test]
    fn a_note_with_no_prose_keeps_its_title_and_the_general_description() {
        for content in [
            "",
            "---\ntags: [a]\n---\n",
            "# Only a heading\n",
            "```mermaid\ngraph TD\n  A --> B\n```\n",
            "| a | b |\n|---|---|\n| 1 | 2 |\n",
            "![[diagram.png]]\n\n![alt](picture.png)\n",
        ] {
            let preview = preview("Beacon", content, serde_json::json!({}));
            assert_eq!(preview.title, "Beacon", "{content:?}");
            assert_eq!(preview.page_title, "Beacon · Hatchdoor", "{content:?}");
            assert_eq!(preview.description, GENERAL_DESCRIPTION, "{content:?}");
            assert_eq!(preview.kind, "article");
        }
    }

    #[test]
    fn opening_prose_is_the_first_paragraph_after_frontmatter_and_headings() {
        assert_eq!(
            prose(
                "---\ntitle: x\n---\n\n# Title\n\n## Intro\nFirst line\nsecond line.\n\nSecond paragraph.\n"
            ),
            Some("First line second line.".to_string())
        );
        assert_eq!(
            prose("Title\n=====\n\nBody text.\n"),
            Some("Body text.".to_string())
        );
        assert_eq!(
            prose("Prose first.\n# Then a heading\nMore.\n"),
            Some("Prose first.".to_string())
        );
        // A table whose header row leaves no text takes no prose with it.
        assert_eq!(
            prose("Prose first.\n![[a.png]] | ![[b.png]]\n--- | ---\n"),
            Some("Prose first.".to_string())
        );
    }

    #[test]
    fn opening_prose_skips_code_diagrams_tables_images_and_comments() {
        let content = "\
# Title

```rust
fn main() {}
```

~~~
tilde fence
~~~

$$
x = y
$$

| a | b |
|---|---|
| 1 | 2 |

Name | Value
--- | ---
one | two

![[diagram.png]]
![alt text](picture.png)

---

%% a hidden
comment %%
<!-- another
one -->
[^1]: A footnote definition.
[ref]: https://example.com

At last, prose.
";
        assert_eq!(prose(content), Some("At last, prose.".to_string()));
    }

    #[test]
    fn opening_prose_reads_quotes_callouts_and_lists() {
        assert_eq!(
            prose("> A quoted line\n> and its second.\n"),
            Some("A quoted line and its second.".to_string())
        );
        assert_eq!(
            prose("> [!note]- Heads up\n> The callout body.\n"),
            Some("Heads up The callout body.".to_string())
        );
        assert_eq!(
            prose("> [!tip]\n> Only a body.\n"),
            Some("Only a body.".to_string())
        );
        assert_eq!(
            prose("- [ ] first task\n- [x] second task\n1. third\n"),
            Some("first task second task third".to_string())
        );
        assert_eq!(
            prose("> ```\n> code in a quote\n> ```\n> Then words.\n"),
            Some("Then words.".to_string())
        );
    }

    #[test]
    fn plain_text_reduces_inline_markdown_to_its_words() {
        for (markdown, expected) in [
            (
                "**Bold**, *italic*, __strong__ and _emphasis_.",
                "Bold, italic, strong and emphasis.",
            ),
            (
                "A ~~struck~~ and ==marked== word.",
                "A struck and marked word.",
            ),
            (
                "Keep snake_case, 2 * 3 and a == b.",
                "Keep snake_case, 2 * 3 and a == b.",
            ),
            (
                "Keep 2*3, x==y and a~~b too.",
                "Keep 2*3, x==y and a~~b too.",
            ),
            (
                "A [&amp;lt; label](x) decodes once.",
                "A &lt; label decodes once.",
            ),
            ("Run `cargo test` now.", "Run cargo test now."),
            ("Double ``a ` b`` ticks.", "Double a ` b ticks."),
            (
                "See [the docs](https://example.com/a_(b)) here.",
                "See the docs here.",
            ),
            ("A [reference][ref] link.", "A reference link."),
            ("A [**bold** label](x).", "A bold label."),
            ("Literal [brackets] stay.", "Literal [brackets] stay."),
            ("Link to [[Other note]].", "Link to Other note."),
            ("Link to [[folder/Other|its alias]].", "Link to its alias."),
            (
                "Link to [[Other#Section]] and [[#Local]].",
                "Link to Other > Section and Local.",
            ),
            (
                "An embed ![[Other]] and ![image](a.png) vanish.",
                "An embed and vanish.",
            ),
            ("A footnote[^1] mark.", "A footnote mark."),
            (
                "Inline <span class=\"x\">html</span> and <br/> go.",
                "Inline html and go.",
            ),
            (
                "An autolink <https://example.com> stays.",
                "An autolink https://example.com stays.",
            ),
            ("Less than 3 < 4 stays.", "Less than 3 < 4 stays."),
            (
                "Escaped \\*stars\\* and \\[brackets\\].",
                "Escaped *stars* and [brackets].",
            ),
            (
                "Entities &amp; &lt;tags&gt; decode.",
                "Entities & <tags> decode.",
            ),
            ("A block reference. ^a1b2c3", "A block reference."),
            (
                "Unclosed [[link and `code stay readable.",
                "Unclosed [[link and code stay readable.",
            ),
        ] {
            assert_eq!(plain_text(markdown), expected, "{markdown}");
        }
    }

    #[test]
    fn a_long_description_is_cut_on_a_word_with_an_ellipsis() {
        let long = "word ".repeat(80);
        let cut_text = cut(long.trim());
        assert!(cut_text.chars().count() <= DESCRIPTION_CHARS);
        assert!(cut_text.ends_with("word…"), "{cut_text}");

        // Cut inside a word: the whole word goes.
        let text = format!("{} tail", "a".repeat(196));
        assert_eq!(
            cut(&format!("{text} and more words to push it over")),
            format!("{}…", "a".repeat(196))
        );

        // Trailing punctuation does not sit before the ellipsis.
        let text = format!("{}, {}", "b".repeat(150), "c".repeat(80));
        assert_eq!(cut(&text), format!("{}…", "b".repeat(150)));

        // One early space is no reason to drop everything after it.
        let text = format!("Hello {}", "語".repeat(300));
        let cut_text = cut(&text);
        assert_eq!(cut_text.chars().count(), DESCRIPTION_CHARS);
        assert!(cut_text.starts_with("Hello 語"));

        let exact = "d".repeat(DESCRIPTION_CHARS);
        assert_eq!(cut(&exact), exact);
    }

    #[test]
    fn text_without_spaces_is_cut_between_characters() {
        let cjk = "語".repeat(300);
        let cut_text = cut(&cjk);
        assert_eq!(cut_text.chars().count(), DESCRIPTION_CHARS);
        assert!(cut_text.ends_with('…'));

        // 198 letters, then a letter whose accent would be the first
        // character dropped: the letter goes with its accent.
        let accented = format!("{}e\u{301}{}", "a".repeat(198), "z".repeat(50));
        assert_eq!(cut(&accented), format!("{}…", "a".repeat(198)));
    }

    #[test]
    fn the_long_description_property_is_cut_too() {
        let preview = preview(
            "Beacon",
            "",
            serde_json::json!({ "description": "word ".repeat(80) }),
        );
        let description = preview.description;
        assert!(description.chars().count() <= DESCRIPTION_CHARS);
        assert!(description.ends_with("word…"));
    }

    #[test]
    fn general_pages_share_the_wording_and_differ_only_in_the_tab_title() {
        for (path, page_title) in [
            ("/", "Hatchdoor"),
            ("/graph", "Graph · Hatchdoor"),
            ("/stats", "Stats · Hatchdoor"),
            ("/settings", "Settings · Hatchdoor"),
            ("/nope", "Hatchdoor"),
        ] {
            let preview = LinkPreview::general(path);
            assert_eq!(preview.page_title, page_title);
            assert_eq!(preview.title, "Hatchdoor");
            assert_eq!(preview.description, GENERAL_DESCRIPTION);
            assert_eq!(preview.kind, "website");
        }
    }

    #[test]
    fn the_tags_replace_the_title_and_carry_the_public_address() {
        let preview = preview("Beacon", "The body.\n", serde_json::json!({}));
        let page = preview.write_into(
            INDEX,
            Some("https://notes.example.com/base"),
            "/v/abc/n/Beacon%20Launch",
        );
        assert_eq!(page.matches("<title>").count(), 1);
        for tag in [
            "<title>Beacon · Hatchdoor</title>",
            "<meta name=\"description\" content=\"The body.\" />",
            "<meta property=\"og:title\" content=\"Beacon\" />",
            "<meta property=\"og:description\" content=\"The body.\" />",
            "<meta property=\"og:type\" content=\"article\" />",
            "<meta property=\"og:site_name\" content=\"Hatchdoor\" />",
            "<meta property=\"og:url\" content=\"https://notes.example.com/base/v/abc/n/Beacon%20Launch\" />",
            "<meta property=\"og:image\" content=\"https://notes.example.com/base/link-preview.png\" />",
            "<meta property=\"og:image:width\" content=\"1200\" />",
            "<meta property=\"og:image:height\" content=\"630\" />",
            "<meta name=\"twitter:card\" content=\"summary_large_image\" />",
        ] {
            assert!(page.contains(tag), "{tag} missing from {page}");
        }
        assert!(page.contains("og:image:alt"));
        // Everything else in the page is untouched.
        assert!(page.starts_with("<html>\n  <head>\n    <meta charset=\"UTF-8\" />\n    <title>"));
        assert!(page.ends_with(
            "\n    <script></script>\n  </head>\n  <body><div id=\"root\"></div></body>\n</html>\n"
        ));
    }

    #[test]
    fn without_a_public_address_the_picture_and_the_page_address_are_left_out() {
        let page = LinkPreview::general("/graph").write_into(INDEX, None, "/graph");
        assert!(page.contains("<title>Graph · Hatchdoor</title>"));
        assert!(page.contains("<meta property=\"og:title\" content=\"Hatchdoor\" />"));
        assert!(page.contains("<meta property=\"og:type\" content=\"website\" />"));
        assert!(page.contains("<meta name=\"twitter:card\" content=\"summary\" />"));
        assert!(!page.contains("og:image"));
        assert!(!page.contains("og:url"));
    }

    #[test]
    fn hostile_values_cannot_leave_their_element_or_attribute() {
        let hostile = "</title><script>alert(\"x\")</script> 'q' & <b>";
        let preview = preview(
            hostile,
            "",
            // Written as entities, so reducing the value to plain text leaves
            // real angle brackets for the escaping to deal with.
            serde_json::json!({
                "description": "\"> 3 < 4 &lt;img src=x onerror=alert(1)&gt; &lt;/title&gt; 'q' & <b>bold</b>"
            }),
        );
        let page = preview.write_into(INDEX, Some("https://notes.example.com"), "/v/abc/n/a\"b<c>");
        // The page's own script element is the only one, and the title
        // element still closes once.
        assert_eq!(page.matches("<script").count(), 1);
        assert_eq!(page.matches("</title>").count(), 1);
        assert!(!page.contains("<img"));
        assert!(!page.contains("<b>"));
        assert!(page.contains(
            "<title>&lt;/title&gt;&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt; &#39;q&#39; &amp; &lt;b&gt; · Hatchdoor</title>"
        ));
        assert!(page.contains(
            "content=\"&quot;&gt; 3 &lt; 4 &lt;img src=x onerror=alert(1)&gt; &lt;/title&gt; &#39;q&#39; &amp; bold\""
        ));
        assert!(page.contains("content=\"https://notes.example.com/v/abc/n/a&quot;b&lt;c&gt;\""));
        // Every attribute value holds no raw quote: each tag line still has
        // exactly its own four.
        for line in page
            .lines()
            .filter(|line| line.contains("og:") || line.contains("name=\"description\""))
        {
            assert_eq!(line.matches('"').count(), 4, "{line}");
        }
    }

    #[test]
    fn a_page_without_a_title_gets_the_tags_before_the_head_closes() {
        let page = LinkPreview::general("/").write_into(
            "<html><head></head><body></body></html>",
            None,
            "/",
        );
        assert!(page.starts_with("<html><head><title>Hatchdoor</title>"));
        assert!(page.ends_with("</head><body></body></html>"));
        assert_eq!(
            LinkPreview::general("/").write_into("no head here", None, "/"),
            "no head here"
        );
    }
}
