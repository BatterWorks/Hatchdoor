use std::fs;
use std::io;
use std::path::Path;

use crate::vault::paths::{normalize_link_target, normalize_title};
use crate::vault::types::{NoteEntry, VaultIndex};

use super::types::{TextRewrite, WriteError};

/// Retarget every backlink to the moved note, keeping the form its author wrote.
///
/// A link written as a bare title stays a bare title, a path-qualified link
/// gets the new full path, and `new_target: None` removes the link entirely
/// (delete). The bare form is only safe while the new title names exactly one
/// note, so a title another note already carries falls back to the full path:
/// a link that resolved before the move must still resolve after it (#235).
pub(super) fn backlink_rewrite_plan(
    index: &VaultIndex,
    moved_slug: &str,
    new_target: Option<&str>,
) -> Result<Vec<TextRewrite>, WriteError> {
    let entries = index.ordered_entries();
    let bare_new_target =
        new_target.and_then(|target| unambiguous_bare_title(&entries, moved_slug, target));
    let mut rewrites = Vec::new();
    for entry in entries {
        if entry.slug == moved_slug {
            continue;
        }
        let content = fs::read_to_string(&entry.path).map_err(|error| {
            WriteError::Io(format!(
                "failed to read note '{}' for backlink rewrite: {error}",
                entry.relative_path
            ))
        })?;
        let rewritten = transform_wikilinks(&content, |target| {
            let Some(candidate) = index.resolve_wikilink(target) else {
                return Some(target.to_string());
            };
            if candidate.slug != moved_slug {
                return Some(target.to_string());
            }
            match bare_new_target.as_deref() {
                Some(bare) if target_is_the_moved_notes_bare_title(target, &candidate.title) => {
                    Some(bare.to_string())
                }
                _ => new_target.map(ToOwned::to_owned),
            }
        });
        if rewritten != content {
            rewrites.push(TextRewrite {
                path: entry.path,
                content: rewritten,
            });
        }
    }
    Ok(rewrites)
}

/// The moved note's new bare title, when no *other* note in the pre-move index
/// already carries it.
///
/// Only the moved note changes name, so the pre-move index answers the
/// post-move question. `resolve_wikilink` tries `by_path_title` before
/// `by_title`, but a bare title holds no `/` while a nested note's path key
/// always does, so the only note a bare title can capture on the path pass is
/// a root-level note whose relative path is its own title — which carries that
/// title too, and so is already caught here.
fn unambiguous_bare_title(
    entries: &[NoteEntry],
    moved_slug: &str,
    new_target: &str,
) -> Option<String> {
    let bare = new_target.rsplit('/').next().unwrap_or(new_target);
    let normalized = normalize_title(bare);
    let taken_by_another_note = entries
        .iter()
        .any(|entry| entry.slug != moved_slug && normalize_title(&entry.title) == normalized);
    (!taken_by_another_note).then(|| bare.to_string())
}

/// Whether this target is the moved note's own title, written bare.
///
/// A slug-form target (`[[some-note]]` for "Some Note") is machine-authored
/// and takes the full path like any other non-title form; for a single-word
/// title the two forms normalize alike, so the distinction only ever arises
/// for multi-word titles.
fn target_is_the_moved_notes_bare_title(target: &str, moved_title: &str) -> bool {
    let normalized = normalize_link_target(target);
    !normalized.contains('/') && normalize_title(&normalized) == normalize_title(moved_title)
}

pub(super) fn transform_wikilinks<F>(content: &str, transform_target: F) -> String
where
    F: Fn(&str) -> Option<String>,
{
    let mut out = String::with_capacity(content.len());
    let mut fenced_marker: Option<(u8, usize)> = None;
    for line in content.split_inclusive('\n') {
        let (line_body, line_ending) = line
            .strip_suffix('\n')
            .map(|body| (body, "\n"))
            .unwrap_or((line, ""));
        let trimmed = line_body.trim_start();
        if let Some((marker, min_len)) = fenced_marker {
            if let Some((close_marker, close_len)) = parse_fence_marker(trimmed)
                && close_marker == marker
                && close_len >= min_len
            {
                fenced_marker = None;
            }
            out.push_str(line_body);
            out.push_str(line_ending);
            continue;
        }
        if let Some(marker) = parse_fence_marker(trimmed) {
            fenced_marker = Some(marker);
            out.push_str(line_body);
            out.push_str(line_ending);
            continue;
        }
        out.push_str(&transform_wikilinks_in_line(line_body, &transform_target));
        out.push_str(line_ending);
    }
    out
}

pub(super) fn transform_wikilinks_in_line<F>(line: &str, transform_target: &F) -> String
where
    F: Fn(&str) -> Option<String>,
{
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut idx = 0usize;
    let mut inline_marker_len = 0usize;
    while idx < chars.len() {
        if chars[idx] == '`' {
            let mut marker_len = 1usize;
            while idx + marker_len < chars.len() && chars[idx + marker_len] == '`' {
                marker_len += 1;
            }
            for _ in 0..marker_len {
                out.push('`');
            }
            if inline_marker_len == 0 {
                inline_marker_len = marker_len;
            } else if marker_len == inline_marker_len {
                inline_marker_len = 0;
            }
            idx += marker_len;
            continue;
        }
        let wiki_start = inline_marker_len == 0
            && ((idx + 1 < chars.len() && chars[idx] == '[' && chars[idx + 1] == '[')
                || (idx + 2 < chars.len()
                    && chars[idx] == '!'
                    && chars[idx + 1] == '['
                    && chars[idx + 2] == '['));
        if wiki_start {
            let is_embed = chars[idx] == '!';
            let body_start = if is_embed { idx + 3 } else { idx + 2 };
            let mut end = body_start;
            while end + 1 < chars.len() {
                if chars[end] == ']' && chars[end + 1] == ']' {
                    break;
                }
                end += 1;
            }
            if end + 1 < chars.len() {
                let body: String = chars[body_start..end].iter().collect();
                if let Some(rewritten_body) = transform_wikilink_body(&body, transform_target) {
                    if is_embed {
                        out.push('!');
                    }
                    out.push_str("[[");
                    out.push_str(&rewritten_body);
                    out.push_str("]]");
                }
                idx = end + 2;
                continue;
            }
        }
        out.push(chars[idx]);
        idx += 1;
    }
    out
}

fn transform_wikilink_body<F>(body: &str, transform_target: &F) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    let target_end = body.find(['|', '#', '^']).unwrap_or(body.len());
    let target = body[..target_end].trim();
    if target.is_empty() {
        return Some(body.to_string());
    }
    let suffix = &body[target_end..];
    transform_target(target).map(|new_target| format!("{new_target}{suffix}"))
}

pub(super) fn parse_fence_marker(trimmed_line: &str) -> Option<(u8, usize)> {
    let bytes = trimmed_line.as_bytes();
    let marker = *bytes.first()?;
    if marker != b'`' && marker != b'~' {
        return None;
    }
    let mut len = 1usize;
    while len < bytes.len() && bytes[len] == marker {
        len += 1;
    }
    if len >= 3 { Some((marker, len)) } else { None }
}

pub(super) fn merge_rewrites(left: Vec<TextRewrite>, right: Vec<TextRewrite>) -> Vec<TextRewrite> {
    let mut merged: Vec<TextRewrite> = Vec::new();
    for rewrite in left.into_iter().chain(right) {
        if let Some(existing) = merged
            .iter_mut()
            .find(|existing| existing.path == rewrite.path)
        {
            existing.content = rewrite.content;
        } else {
            merged.push(rewrite);
        }
    }
    merged
}

pub(super) fn rewrite_content_or_read(
    path: &Path,
    rewrites: &[TextRewrite],
) -> Result<String, io::Error> {
    rewrites
        .iter()
        .rev()
        .find(|rewrite| rewrite.path == path)
        .map(|rewrite| Ok(rewrite.content.clone()))
        .unwrap_or_else(|| fs::read_to_string(path))
}
