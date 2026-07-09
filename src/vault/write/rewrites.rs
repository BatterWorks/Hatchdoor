use std::fs;
use std::io;
use std::path::Path;

use crate::vault::types::VaultIndex;

use super::assets::asset_reference_rewrite_plan;
use super::fs_ops::atomic_write;
use super::types::{TextRewrite, WriteError};

pub(super) fn backlink_rewrite_plan(
    index: &VaultIndex,
    moved_slug: &str,
    new_target: Option<&str>,
) -> Result<Vec<TextRewrite>, WriteError> {
    let mut rewrites = Vec::new();
    for entry in index.ordered_entries() {
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
            let should_change = index
                .resolve_wikilink(target)
                .is_some_and(|candidate| candidate.slug == moved_slug);
            if !should_change {
                return Some(target.to_string());
            }
            new_target.map(ToOwned::to_owned)
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

/// Apply a batch of note rewrites all-or-nothing. Each target's current content
/// is captured before it is overwritten; if any later write fails, every file
/// already written in this batch is restored to its captured content, so callers
/// never observe a partially-applied rewrite (which would leave dangling or
/// duplicated backlinks/asset references across the vault).
pub(super) fn apply_rewrites(
    rewrites: Vec<TextRewrite>,
) -> Result<Vec<std::path::PathBuf>, WriteError> {
    let mut written: Vec<(std::path::PathBuf, Option<String>)> = Vec::with_capacity(rewrites.len());
    for rewrite in &rewrites {
        // Capture the pre-write content so a later failure can restore it.
        // Rewrite targets are existing tracked notes, so a read normally
        // succeeds; None means the file was absent and there is nothing to
        // restore it to.
        let original = fs::read_to_string(&rewrite.path).ok();
        if let Err(error) = atomic_write(&rewrite.path, &rewrite.content) {
            for (path, prior) in written.iter().rev() {
                if let Some(prior) = prior {
                    let _ = atomic_write(path, prior);
                }
            }
            return Err(error);
        }
        written.push((rewrite.path.clone(), original));
    }
    Ok(written.into_iter().map(|(path, _)| path).collect())
}

pub(super) fn rollback_rewrites(
    vault_root: &Path,
    index: &VaultIndex,
    from_path: &Path,
    to_path: &Path,
) {
    if let Ok(rewrites) =
        asset_reference_rewrite_plan(vault_root, index, "", from_path, to_path, &[])
    {
        let _ = apply_rewrites(rewrites);
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn apply_rewrites_rolls_back_written_files_when_a_later_write_fails() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();

        // A real note that will be rewritten first (and must be restored), and a
        // second rewrite whose path is under a non-existent directory so its
        // atomic_write fails deterministically after the first already landed.
        let a = root.join("A.md");
        fs::write(&a, "orig A\n").unwrap();
        let b = root.join("missing_dir").join("B.md");

        let rewrites = vec![
            TextRewrite {
                path: a.clone(),
                content: "new A\n".to_string(),
            },
            TextRewrite {
                path: b.clone(),
                content: "new B\n".to_string(),
            },
        ];

        let err = apply_rewrites(rewrites).expect_err("second rewrite must fail");
        assert!(matches!(err, WriteError::Io(_)));

        // The first file must be rolled back to its original content, not left
        // half-applied.
        assert_eq!(
            fs::read_to_string(&a).unwrap(),
            "orig A\n",
            "A.md should be restored after the batch failed"
        );
    }
}
