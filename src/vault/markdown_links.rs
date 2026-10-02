//! Markdown-form note links (ADR-28): `[t](p.md)`, `[t](<p.md>)` and the
//! reference form `[t][r]` with `[r]: p.md`.
//!
//! One scanner serves the link graph and every rewriter in the write layer, so
//! what counts as a note link and what a rename is willing to edit cannot
//! drift apart. `frontend/src/components/note-page/markdownLinks.ts` carries
//! the same recognition for the renderer; the two have to change together.

use std::collections::HashMap;
use std::ops::Range;

use crate::cache::parse::parse_fence_marker;

use super::paths::{
    LinkForm, folder_distance, normalize_title, resolve_path_ladder, strip_md_extension,
};

/// Where a link's destination sits in the scanned content. `span` excludes the
/// angle brackets of the `<p.md>` form, which `angle` records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LinkDestination {
    pub(crate) span: Range<usize>,
    pub(crate) angle: bool,
}

/// One Markdown link construct, located by byte ranges into the scanned text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MarkdownLink {
    /// `[text](destination "title")`.
    Inline {
        whole: Range<usize>,
        text: Range<usize>,
        destination: LinkDestination,
    },
    /// `[label]: destination "title"` on a line of its own. `line` runs to the
    /// end of the line ending, so removing it removes the line.
    Definition {
        line: Range<usize>,
        label: String,
        destination: LinkDestination,
    },
    /// `[text][label]`, `[label][]` or `[label]`, which take their destination
    /// from the definition of the same label. `label` is normalised the way
    /// definitions are, so the two compare equal.
    Reference {
        whole: Range<usize>,
        text: Range<usize>,
        label: String,
    },
    /// `![alt](destination)`. Never a note link (ADR-28); recorded only so
    /// the link style vote (ADR-33) can count Markdown embeds.
    Image { destination: LinkDestination },
}

/// Every Markdown link in `content` outside fenced code and inline code spans,
/// in document order. Wikilinks are skipped, and of image syntax only the
/// inline `![t](p)` form is recorded, as [`MarkdownLink::Image`].
pub(crate) fn scan_markdown_links(content: &str) -> Vec<MarkdownLink> {
    let mut links = Vec::new();
    let mut fenced_marker: Option<(u8, usize)> = None;
    let mut line_start = 0usize;
    for raw in content.split_inclusive('\n') {
        let offset = line_start;
        line_start += raw.len();
        let line = match raw.strip_suffix('\n') {
            Some(line) => line.strip_suffix('\r').unwrap_or(line),
            None => raw,
        };
        let trimmed = line.trim_start();
        if let Some((marker, min_len)) = fenced_marker {
            if let Some((close_marker, close_len)) = parse_fence_marker(trimmed)
                && close_marker == marker
                && close_len >= min_len
            {
                fenced_marker = None;
            }
            continue;
        }
        if let Some(marker) = parse_fence_marker(trimmed) {
            fenced_marker = Some(marker);
            continue;
        }
        if let Some((label, destination)) = parse_definition(line.as_bytes()) {
            links.push(MarkdownLink::Definition {
                line: offset..offset + raw.len(),
                label,
                destination: LinkDestination {
                    span: destination.span.start + offset..destination.span.end + offset,
                    angle: destination.angle,
                },
            });
            continue;
        }
        scan_inline(line, offset, &mut links);
    }
    links
}

fn scan_inline(line: &str, offset: usize, links: &mut Vec<MarkdownLink>) {
    let bytes = line.as_bytes();
    let shift = |range: Range<usize>| range.start + offset..range.end + offset;
    let mut idx = 0usize;
    while idx < bytes.len() {
        match bytes[idx] {
            b'\\' => idx += 2,
            b'`' => idx = skip_code_span(bytes, idx),
            b'[' if bytes.get(idx + 1) == Some(&b'[') => {
                // A wikilink. Its own reader handles it; nothing inside it is a
                // Markdown link.
                idx = find_from(bytes, idx + 2, b"]]").map_or(idx + 2, |end| end + 2);
            }
            b'[' => {
                let is_image = idx > 0 && bytes[idx - 1] == b'!';
                let Some(close) = matching_bracket(bytes, idx) else {
                    idx += 1;
                    continue;
                };
                let text = idx + 1..close;
                // `[^1]` is a footnote, never a link.
                if bytes.get(idx + 1) == Some(&b'^') {
                    idx = close + 1;
                    continue;
                }
                if bytes.get(close + 1) == Some(&b'(')
                    && let Some((destination, end)) = parse_inline_destination(bytes, close + 1)
                {
                    let destination = LinkDestination {
                        span: shift(destination.span),
                        angle: destination.angle,
                    };
                    links.push(if is_image {
                        MarkdownLink::Image { destination }
                    } else {
                        MarkdownLink::Inline {
                            whole: shift(idx..end),
                            text: shift(text),
                            destination,
                        }
                    });
                    idx = end;
                    continue;
                }
                if bytes.get(close + 1) == Some(&b'[')
                    && let Some(label_close) = find_from(bytes, close + 2, b"]")
                {
                    let label = if label_close == close + 2 {
                        &line[text.clone()]
                    } else {
                        &line[close + 2..label_close]
                    };
                    if !is_image {
                        links.push(MarkdownLink::Reference {
                            whole: shift(idx..label_close + 1),
                            text: shift(text),
                            label: normalize_label(label),
                        });
                    }
                    idx = label_close + 1;
                    continue;
                }
                // A shortcut reference. Brackets that turn out not to be one
                // are plain text, and a real link may sit inside them, so the
                // scan carries on from just past the opening bracket.
                if !is_image {
                    links.push(MarkdownLink::Reference {
                        whole: shift(idx..close + 1),
                        text: shift(text.clone()),
                        label: normalize_label(&line[text]),
                    });
                }
                idx += 1;
            }
            _ => idx += 1,
        }
    }
}

/// Past the code span opening at `start`. A run nothing closes makes the rest
/// of the line code, the rule `cache::parse` applies to wikilinks, so the two
/// link forms agree about what is prose (ADR-28).
fn skip_code_span(bytes: &[u8], start: usize) -> usize {
    let run = backtick_run(bytes, start);
    let mut idx = start + run;
    while idx < bytes.len() {
        if bytes[idx] == b'`' {
            let close = backtick_run(bytes, idx);
            if close == run {
                return idx + close;
            }
            idx += close;
        } else {
            idx += 1;
        }
    }
    bytes.len()
}

fn backtick_run(bytes: &[u8], start: usize) -> usize {
    bytes[start..].iter().take_while(|&&b| b == b'`').count()
}

fn find_from(bytes: &[u8], start: usize, needle: &[u8]) -> Option<usize> {
    if start > bytes.len() {
        return None;
    }
    bytes[start..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|position| position + start)
}

/// The `]` closing the `[` at `open`, honouring nesting, escapes and code
/// spans, which bind tighter than link brackets.
fn matching_bracket(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut idx = open;
    while idx < bytes.len() {
        match bytes[idx] {
            b'\\' => {
                idx += 2;
                continue;
            }
            b'`' => {
                idx = skip_code_span(bytes, idx);
                continue;
            }
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(idx);
                }
            }
            _ => {}
        }
        idx += 1;
    }
    None
}

/// Parse `(destination "title")` starting at the `(` at `open`, returning the
/// destination and the index just past the closing `)`.
fn parse_inline_destination(bytes: &[u8], open: usize) -> Option<(LinkDestination, usize)> {
    let mut idx = skip_spaces(bytes, open + 1);
    let (destination, after) = parse_destination(bytes, idx, true)?;
    idx = skip_spaces(bytes, after);
    if idx > after {
        idx = skip_title(bytes, idx)?;
        idx = skip_spaces(bytes, idx);
    }
    (bytes.get(idx) == Some(&b')')).then_some((destination, idx + 1))
}

/// A destination at `start`: `<...>` on one line, or a run with no spaces or
/// control characters and balanced parentheses. `inline` stops the plain form
/// at an unbalanced `)`, which closes an inline link.
fn parse_destination(bytes: &[u8], start: usize, inline: bool) -> Option<(LinkDestination, usize)> {
    if bytes.get(start) == Some(&b'<') {
        let mut idx = start + 1;
        while idx < bytes.len() {
            match bytes[idx] {
                b'\\' => idx += 2,
                b'>' => {
                    return Some((
                        LinkDestination {
                            span: start + 1..idx,
                            angle: true,
                        },
                        idx + 1,
                    ));
                }
                b'<' => return None,
                _ => idx += 1,
            }
        }
        return None;
    }
    let mut idx = start;
    let mut depth = 0usize;
    while idx < bytes.len() {
        match bytes[idx] {
            b'\\' if idx + 1 < bytes.len() => idx += 2,
            byte if byte <= b' ' || byte == 0x7f => break,
            b'(' => {
                depth += 1;
                idx += 1;
            }
            b')' if inline && depth == 0 => break,
            b')' => {
                depth = depth.saturating_sub(1);
                idx += 1;
            }
            _ => idx += 1,
        }
    }
    Some((
        LinkDestination {
            span: start..idx,
            angle: false,
        },
        idx,
    ))
}

/// Past an optional link title in `"..."`, `'...'` or `(...)`.
fn skip_title(bytes: &[u8], start: usize) -> Option<usize> {
    let close = match bytes.get(start) {
        Some(b'"') => b'"',
        Some(b'\'') => b'\'',
        Some(b'(') => b')',
        _ => return Some(start),
    };
    let mut idx = start + 1;
    while idx < bytes.len() {
        match bytes[idx] {
            b'\\' => idx += 2,
            byte if byte == close => return Some(idx + 1),
            _ => idx += 1,
        }
    }
    None
}

fn skip_spaces(bytes: &[u8], start: usize) -> usize {
    let mut idx = start;
    while matches!(bytes.get(idx), Some(b' ' | b'\t')) {
        idx += 1;
    }
    idx
}

/// `[label]: destination "title"`, indented at most three spaces, with nothing
/// else on the line.
fn parse_definition(bytes: &[u8]) -> Option<(String, LinkDestination)> {
    let indent = bytes.iter().take_while(|&&b| b == b' ').count();
    if indent > 3 || bytes.get(indent) != Some(&b'[') || bytes.get(indent + 1) == Some(&b'^') {
        return None;
    }
    let mut idx = indent + 1;
    while idx < bytes.len() {
        match bytes[idx] {
            b'\\' => idx += 2,
            b'[' => return None,
            b']' => break,
            _ => idx += 1,
        }
    }
    let label = std::str::from_utf8(bytes.get(indent + 1..idx)?).ok()?;
    if label.trim().is_empty() || bytes.get(idx + 1) != Some(&b':') {
        return None;
    }
    let start = skip_spaces(bytes, idx + 2);
    let (destination, after) = parse_destination(bytes, start, false)?;
    if destination.span.is_empty() && !destination.angle {
        return None;
    }
    let mut idx = skip_spaces(bytes, after);
    if idx > after {
        idx = skip_title(bytes, idx)?;
        idx = skip_spaces(bytes, idx);
    }
    (idx == bytes.len()).then(|| (normalize_label(label), destination))
}

/// Reference labels match case-insensitively with runs of whitespace folded.
fn normalize_label(label: &str) -> String {
    label
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// A destination that names a note: its path, decoded, and how many bytes of
/// the raw destination that path occupies (everything before `#anchor`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NoteLinkTarget {
    pub(crate) path: String,
    pub(crate) raw_path_len: usize,
}

/// Whether `raw` (a destination as written) is a note link, and to what path.
///
/// A note link is a local path ending `.md` once any `#anchor` is removed.
/// Anything with a URL scheme, or protocol-relative, is external and never a
/// note link, whatever it ends in.
pub(crate) fn note_link_target(raw: &str) -> Option<NoteLinkTarget> {
    let raw_path_len = unescaped_hash(raw).unwrap_or(raw.len());
    let raw_path = &raw[..raw_path_len];
    if raw_path.starts_with("//") || has_url_scheme(raw_path) {
        return None;
    }
    let path = percent_decode(&unescape_backslashes(raw_path));
    let file_name = path.rsplit(['/', '\\']).next().unwrap_or(&path);
    if !file_name.ends_with(".md") || file_name.len() <= ".md".len() {
        return None;
    }
    Some(NoteLinkTarget { path, raw_path_len })
}

fn unescaped_hash(raw: &str) -> Option<usize> {
    let bytes = raw.as_bytes();
    let mut idx = 0usize;
    while idx < bytes.len() {
        match bytes[idx] {
            b'\\' => idx += 2,
            b'#' => return Some(idx),
            _ => idx += 1,
        }
    }
    None
}

fn has_url_scheme(raw: &str) -> bool {
    let Some((scheme, _)) = raw.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
}

/// Drop the backslash from an escaped ASCII punctuation character, as Markdown
/// does inside a destination.
fn unescape_backslashes(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\'
            && let Some(&next) = chars.peek()
            && next.is_ascii_punctuation()
        {
            out.push(next);
            chars.next();
            continue;
        }
        out.push(c);
    }
    out
}

/// Decode `%XX` escapes. A `%` not followed by two hex digits is a literal
/// percent sign, so `Save 20% now.md` resolves whether or not its author
/// encoded it. Bytes that do not decode to UTF-8 leave the input as it was.
pub(crate) fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut idx = 0usize;
    while idx < bytes.len() {
        if bytes[idx] == b'%'
            && let (Some(high), Some(low)) = (
                bytes.get(idx + 1).and_then(|b| (*b as char).to_digit(16)),
                bytes.get(idx + 2).and_then(|b| (*b as char).to_digit(16)),
            )
        {
            out.push((high * 16 + low) as u8);
            idx += 3;
            continue;
        }
        out.push(bytes[idx]);
        idx += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| raw.to_string())
}

/// Encode a path for writing into a destination, escaping only what would
/// break the link: whitespace, `%`, `#`, square brackets and parentheses in
/// the plain form; `%`, `#` and angle brackets inside `<...>`. Accented and
/// non-Latin letters stay readable.
pub(crate) fn encode_link_path(path: &str, angle: bool) -> String {
    let mut out = String::with_capacity(path.len());
    for c in path.chars() {
        let escape = match c {
            '%' | '#' => true,
            '<' | '>' => angle,
            '[' | ']' | '(' | ')' => !angle,
            c if c.is_control() => true,
            c if c.is_whitespace() => !angle,
            _ => false,
        };
        if escape {
            let mut buffer = [0u8; 4];
            for byte in c.encode_utf8(&mut buffer).bytes() {
                out.push_str(&format!("%{byte:02X}"));
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Notes addressed by path and by filename, for resolving Markdown note links
/// through [`resolve_path_ladder`]. Keys are lowercased the way
/// `VaultIndex::by_path_title` is, and the first note to claim a path keeps
/// it, in the same layer-priority order.
#[derive(Debug, Clone, Default)]
pub(crate) struct NotePaths {
    /// Normalised Vault-relative path without `.md` to slug.
    by_path: HashMap<String, String>,
    /// Normalised filename stem to `(relative path, slug)`, sorted by path.
    by_name: HashMap<String, Vec<(String, String)>>,
}

impl NotePaths {
    pub(crate) fn insert(&mut self, relative_without_ext: &str, slug: &str) {
        self.by_path
            .entry(normalize_title(relative_without_ext))
            .or_insert_with(|| slug.to_string());
        let name = relative_without_ext
            .rsplit('/')
            .next()
            .unwrap_or(relative_without_ext);
        self.by_name
            .entry(normalize_title(name))
            .or_default()
            .push((relative_without_ext.to_string(), slug.to_string()));
    }

    /// Order every name's candidates by path, so equidistant namesakes resolve
    /// deterministically. Call once all notes are inserted.
    pub(crate) fn sort(&mut self) {
        for candidates in self.by_name.values_mut() {
            candidates.sort();
        }
    }

    /// The slug `path` (decoded, `.md` included) names from a note in
    /// `note_dir`, and the rung it resolved on.
    pub(crate) fn resolve(&self, path: &str, note_dir: &str) -> Option<(&str, LinkForm)> {
        resolve_path_ladder(
            path,
            note_dir,
            |candidate| {
                self.by_path
                    .get(&normalize_title(strip_md_extension(candidate)))
                    .map(String::as_str)
            },
            |name| {
                self.by_name
                    .get(&normalize_title(strip_md_extension(name)))?
                    .iter()
                    .min_by_key(|(relative, _)| folder_distance(note_dir, relative))
                    .map(|(_, slug)| slug.as_str())
            },
        )
    }

    /// A copy with one note moved from `from` to `to` (both Vault-relative,
    /// without `.md`), for asking how a link will resolve after a move.
    pub(crate) fn relocated(&self, slug: &str, from: &str, to: &str) -> Self {
        let mut moved = self.clone();
        let from_key = normalize_title(from);
        if moved
            .by_path
            .get(&from_key)
            .is_some_and(|owner| owner == slug)
        {
            moved.by_path.remove(&from_key);
        }
        for candidates in moved.by_name.values_mut() {
            candidates.retain(|(_, candidate)| candidate != slug);
        }
        moved.insert(to, slug);
        moved.sort();
        moved
    }
}

/// The path from a note in `from_dir` to the note at `target` (both
/// Vault-relative, `target` without `.md`), as a Markdown link writes it.
pub(crate) fn relative_note_path(from_dir: &str, target: &str) -> String {
    let from: Vec<&str> = from_dir
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    let to: Vec<&str> = target.split('/').filter(|part| !part.is_empty()).collect();
    let shared = from
        .iter()
        .zip(to.iter())
        .take_while(|(left, right)| left == right)
        .count();
    let mut parts: Vec<&str> = vec![".."; from.len() - shared];
    parts.extend(&to[shared..]);
    format!("{}.md", parts.join("/"))
}

/// The Vault-relative folder of a note's extension-free relative path.
pub(crate) fn note_dir(relative_without_ext: &str) -> &str {
    relative_without_ext
        .rsplit_once('/')
        .map_or("", |(dir, _)| dir)
}

/// Every Markdown note link that counts toward the link graph, as the
/// destination text it was written with. A reference definition counts only
/// when some link in the note uses it, since an unused definition renders
/// nothing; the first definition of a label is the one that applies.
pub(crate) fn note_link_destinations(content: &str) -> Vec<&str> {
    let links = scan_markdown_links(content);
    let definitions = first_definitions(&links);
    let mut used_labels = std::collections::HashSet::new();
    let mut destinations = Vec::new();
    for link in &links {
        match link {
            MarkdownLink::Inline { destination, .. } => {
                destinations.push(&content[destination.span.clone()]);
            }
            MarkdownLink::Reference { label, .. } => {
                if let Some(destination) = definitions.get(label.as_str())
                    && used_labels.insert(label.as_str())
                {
                    destinations.push(&content[destination.span.clone()]);
                }
            }
            MarkdownLink::Definition { .. } | MarkdownLink::Image { .. } => {}
        }
    }
    destinations
        .into_iter()
        .filter(|destination| note_link_target(destination).is_some())
        .collect()
}

/// How many vault files `content` embeds with inline Markdown image syntax.
/// An external URL or a protocol-relative one is not a vault file.
pub(crate) fn local_image_count(content: &str) -> usize {
    scan_markdown_links(content)
        .iter()
        .filter(|link| match link {
            MarkdownLink::Image { destination } => {
                let raw = &content[destination.span.clone()];
                !raw.is_empty() && !raw.starts_with("//") && !has_url_scheme(raw)
            }
            _ => false,
        })
        .count()
}

fn first_definitions(links: &[MarkdownLink]) -> HashMap<&str, &LinkDestination> {
    let mut definitions = HashMap::new();
    for link in links {
        if let MarkdownLink::Definition {
            label, destination, ..
        } = link
        {
            definitions.entry(label.as_str()).or_insert(destination);
        }
    }
    definitions
}

/// What a rewriter wants done with one note link.
pub(crate) enum NoteLinkEdit {
    Keep,
    /// Point the link at this path (decoded, `.md` included). The anchor, the
    /// title and the link text stay as written.
    Retarget(String),
    /// Remove the link and keep its text, because a Markdown link's text is
    /// the author's prose rather than the target's name.
    Unlink,
}

/// Apply `decide` to every Markdown note link in `content`, leaving every
/// byte it does not name as it was.
///
/// A reference-style link is rewritten through its definition line, so every
/// use follows. Unlinking a definition removes the line and turns each use of
/// its label into plain text.
pub(crate) fn rewrite_note_links(
    content: &str,
    mut decide: impl FnMut(&NoteLinkTarget) -> NoteLinkEdit,
) -> String {
    let links = scan_markdown_links(content);
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    let mut unlinked_labels = std::collections::HashSet::new();
    let mut decided_labels = std::collections::HashSet::new();
    for link in &links {
        let (destination, whole, text, label) = match link {
            MarkdownLink::Inline {
                whole,
                text,
                destination,
            } => (destination, whole, Some(text), None),
            MarkdownLink::Definition {
                line,
                label,
                destination,
            } => (destination, line, None, Some(label)),
            MarkdownLink::Reference { .. } | MarkdownLink::Image { .. } => continue,
        };
        let raw = &content[destination.span.clone()];
        let Some(target) = note_link_target(raw) else {
            continue;
        };
        match decide(&target) {
            NoteLinkEdit::Keep => {}
            NoteLinkEdit::Retarget(path) => {
                let start = destination.span.start;
                edits.push((
                    start..start + target.raw_path_len,
                    encode_link_path(&path, destination.angle),
                ));
            }
            NoteLinkEdit::Unlink => {
                let replacement =
                    text.map_or(String::new(), |text| content[text.clone()].to_string());
                edits.push((whole.clone(), replacement));
                // Only the definition that applies takes its uses with it.
                if let Some(label) = label
                    && decided_labels.insert(label.as_str())
                {
                    unlinked_labels.insert(label.as_str());
                }
            }
        }
        if let Some(label) = label {
            decided_labels.insert(label.as_str());
        }
    }
    for link in &links {
        if let MarkdownLink::Reference { whole, text, label } = link
            && unlinked_labels.contains(label.as_str())
        {
            edits.push((whole.clone(), content[text.clone()].to_string()));
        }
    }
    if edits.is_empty() {
        return content.to_string();
    }
    edits.sort_by_key(|(range, _)| range.start);
    let mut out = String::with_capacity(content.len());
    let mut cursor = 0usize;
    for (range, replacement) in edits {
        if range.start < cursor {
            continue;
        }
        out.push_str(&content[cursor..range.start]);
        out.push_str(&replacement);
        cursor = range.end;
    }
    out.push_str(&content[cursor..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn destinations(content: &str) -> Vec<&str> {
        note_link_destinations(content)
    }

    #[test]
    fn recognises_inline_angle_and_reference_note_links() {
        let content = concat!(
            "See [a](../20-projects/Beacon%20Launch.md) and [b](<../x/Two Words.md>).\n",
            "Then [c][plan] and [plan].\n",
            "\n",
            "[plan]: /20-projects/Plan.md \"The plan\"\n",
        );
        assert_eq!(
            destinations(content),
            vec![
                "../20-projects/Beacon%20Launch.md",
                "../x/Two Words.md",
                "/20-projects/Plan.md",
            ]
        );
    }

    #[test]
    fn ignores_non_note_targets_images_code_and_wikilinks() {
        let content = concat!(
            "[pdf](report.pdf) [video](clip.mp4) [web](https://example.com/a.md)\n",
            "![img](Note.md) `[code](Note.md)` [[Note.md]] [mail](mailto:x.md)\n",
            "```\n[fenced](Note.md)\n```\n",
            "[unused]: Other.md\n",
            "[proto](//host/a.md) [bare](.md)\n",
        );
        assert!(destinations(content).is_empty());
    }

    #[test]
    fn an_unclosed_backtick_makes_the_rest_of_the_line_code_as_for_wikilinks() {
        assert_eq!(
            destinations("[a](A.md) ` [b](B.md)\n[c](C.md)\n"),
            vec!["A.md", "C.md"]
        );
    }

    #[test]
    fn anchor_is_not_part_of_the_path() {
        let target = note_link_target("Install.md#First%20Run").expect("note link");
        assert_eq!(target.path, "Install.md");
        assert_eq!(target.raw_path_len, "Install.md".len());
        assert!(note_link_target("Install#x.md").is_none());
    }

    #[test]
    fn percent_decoding_keeps_a_bare_percent_literal() {
        assert_eq!(percent_decode("Save%2020%%20now.md"), "Save 20% now.md");
        assert_eq!(percent_decode("Caf%C3%A9.md"), "Café.md");
        assert_eq!(percent_decode("100%.md"), "100%.md");
        assert_eq!(percent_decode("%FF.md"), "%FF.md");
    }

    #[test]
    fn encoding_escapes_only_what_breaks_a_link() {
        assert_eq!(
            encode_link_path("../a b/Save 20% (v2) [x].md", false),
            "../a%20b/Save%2020%25%20%28v2%29%20%5Bx%5D.md"
        );
        assert_eq!(encode_link_path("Café/日本語.md", false), "Café/日本語.md");
        assert_eq!(encode_link_path("a b/50%.md", true), "a b/50%25.md");
    }

    #[test]
    fn rewrite_retargets_the_path_and_keeps_text_anchor_and_title() {
        let content = "x [see the plan](Old%20Name.md#Top \"t\") y\n";
        let out = rewrite_note_links(content, |_| NoteLinkEdit::Retarget("New Name.md".into()));
        assert_eq!(out, "x [see the plan](New%20Name.md#Top \"t\") y\n");
    }

    #[test]
    fn unlink_keeps_the_text_and_drops_reference_definitions() {
        let content = concat!(
            "see [the plan](../p/Plan.md) today\n",
            "and [again][p] or [p]\n",
            "[p]: ../p/Plan.md\n",
            "[keep](Other.md)\n",
        );
        let out = rewrite_note_links(content, |target| {
            if target.path.ends_with("Plan.md") {
                NoteLinkEdit::Unlink
            } else {
                NoteLinkEdit::Keep
            }
        });
        assert_eq!(
            out,
            "see the plan today\nand again or p\n[keep](Other.md)\n"
        );
    }

    #[test]
    fn relative_note_path_climbs_and_descends() {
        assert_eq!(
            relative_note_path("00-inbox", "20-projects/Beacon"),
            "../20-projects/Beacon.md"
        );
        assert_eq!(relative_note_path("a", "a/Plan"), "Plan.md");
        assert_eq!(relative_note_path("", "a/Plan"), "a/Plan.md");
    }
}
