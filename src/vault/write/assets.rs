use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::vault::types::{NoteEntry, VaultIndex};

use super::paths::{
    create_parent_dir_inside_root, ensure_existing_path_inside_root, is_trashed_path,
    relative_link_target, same_existing_path, unique_trash_attachment_relative_path,
};
use super::rewrites::{parse_fence_marker, rewrite_content_or_read};
use super::types::{AssetMove, TextRewrite, WriteError};

pub(super) fn asset_move_plan(
    vault_root: &Path,
    index: &VaultIndex,
    moved_entry: &NoteEntry,
    destination_note: &Path,
    allow_trash_collision: bool,
    baseline_rewrites: &[TextRewrite],
) -> Result<(Vec<AssetMove>, Vec<TextRewrite>), WriteError> {
    let content = fs::read_to_string(&moved_entry.path).map_err(|error| {
        WriteError::Io(format!(
            "failed to read note '{}' for asset moves: {error}",
            moved_entry.path.display()
        ))
    })?;
    let source_dir = moved_entry.path.parent().unwrap_or(vault_root);
    let destination_dir = destination_note.parent().unwrap_or(vault_root);
    let mut moves = Vec::new();
    let mut rewrites = Vec::new();
    let mut seen = HashSet::new();
    for relative_asset in referenced_assets(&content) {
        if !seen.insert(relative_asset.clone()) {
            continue;
        }
        let source_asset = source_dir.join(&relative_asset);
        if !source_asset.exists() || !source_asset.is_file() {
            continue;
        }
        ensure_existing_path_inside_root(vault_root, &source_asset)?;
        if is_trashed_path(vault_root, &source_asset)? {
            continue;
        }
        let destination_asset = if allow_trash_collision {
            // Trashing a note: the destination lives under .hatchdoor-trash and
            // may already hold an asset of the same relative path from an earlier
            // delete. Relocate to a unique name rather than failing the delete.
            let relative_str = relative_asset.to_string_lossy();
            vault_root.join(unique_trash_attachment_relative_path(
                vault_root,
                &relative_str,
            )?)
        } else {
            destination_dir.join(&relative_asset)
        };
        create_parent_dir_inside_root(vault_root, &destination_asset, "asset")?;
        if !allow_trash_collision && destination_asset.exists() {
            return Err(WriteError::Conflict(format!(
                "Destination asset already exists: {}",
                destination_asset.display()
            )));
        }
        moves.push(AssetMove {
            source: source_asset.clone(),
            destination: destination_asset.clone(),
        });
        let mut baseline = baseline_rewrites.to_vec();
        baseline.extend(rewrites.clone());
        rewrites.extend(asset_reference_rewrite_plan(
            vault_root,
            index,
            &moved_entry.slug,
            &source_asset,
            &destination_asset,
            &baseline,
        )?);
    }
    Ok((moves, rewrites))
}

pub(super) fn referenced_assets(content: &str) -> Vec<PathBuf> {
    let mut assets = Vec::new();
    for_non_code_line(content, |line| {
        extract_markdown_assets(line, &mut assets);
        extract_wiki_assets(line, &mut assets);
    });
    assets
}

pub(super) fn asset_reference_rewrite_plan(
    vault_root: &Path,
    index: &VaultIndex,
    moved_slug: &str,
    source_asset: &Path,
    destination_asset: &Path,
    baseline_rewrites: &[TextRewrite],
) -> Result<Vec<TextRewrite>, WriteError> {
    let mut rewrites = Vec::new();
    for entry in index.ordered_entries() {
        if entry.slug == moved_slug {
            continue;
        }
        let content = rewrite_content_or_read(&entry.path, baseline_rewrites).map_err(|error| {
            WriteError::Io(format!(
                "failed to read note '{}' for asset reference rewrite: {error}",
                entry.relative_path
            ))
        })?;
        let rewritten = transform_asset_references(&content, |target| {
            let note_dir = entry.path.parent().unwrap_or(vault_root);
            let resolved = note_dir.join(target);
            if !same_existing_path(&resolved, source_asset) {
                return target.to_string_lossy().into_owned();
            }
            relative_link_target(vault_root, &entry.path, destination_asset)
                .unwrap_or_else(|| target.to_string_lossy().into_owned())
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

fn transform_asset_references<F>(content: &str, transform_target: F) -> String
where
    F: Fn(&Path) -> String,
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
        out.push_str(&transform_asset_references_in_line(
            line_body,
            &transform_target,
        ));
        out.push_str(line_ending);
    }
    out
}

fn transform_asset_references_in_line<F>(line: &str, transform_target: &F) -> String
where
    F: Fn(&Path) -> String,
{
    transform_non_inline_code_segments(line, |plain| {
        transform_asset_references_in_plain_line(plain, transform_target)
    })
}

fn transform_asset_references_in_plain_line<F>(line: &str, transform_target: &F) -> String
where
    F: Fn(&Path) -> String,
{
    let markdown_rewritten = transform_markdown_asset_references_in_line(line, transform_target);
    transform_wiki_asset_references_in_line(&markdown_rewritten, transform_target)
}

fn transform_non_inline_code_segments<F>(line: &str, transform_plain: F) -> String
where
    F: Fn(&str) -> String,
{
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut plain = String::new();
    let mut idx = 0usize;
    let mut inline_marker_len = 0usize;
    while idx < chars.len() {
        if chars[idx] == '`' {
            if inline_marker_len == 0 && !plain.is_empty() {
                out.push_str(&transform_plain(&plain));
                plain.clear();
            }
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
        if inline_marker_len == 0 {
            plain.push(chars[idx]);
        } else {
            out.push(chars[idx]);
        }
        idx += 1;
    }
    if !plain.is_empty() {
        out.push_str(&transform_plain(&plain));
    }
    out
}

fn transform_markdown_asset_references_in_line<F>(line: &str, transform_target: &F) -> String
where
    F: Fn(&Path) -> String,
{
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(start) = rest.find("](") {
        out.push_str(&rest[..start + 2]);
        rest = &rest[start + 2..];
        let Some(end) = rest.find(')') else {
            out.push_str(rest);
            return out;
        };
        let body = &rest[..end];
        out.push_str(&transform_markdown_asset_body(body, transform_target));
        out.push(')');
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

fn transform_markdown_asset_body<F>(body: &str, transform_target: &F) -> String
where
    F: Fn(&Path) -> String,
{
    let leading = body.len() - body.trim_start().len();
    let trimmed_start = &body[leading..];
    let target_len = trimmed_start
        .find(char::is_whitespace)
        .unwrap_or(trimmed_start.len());
    let target = &trimmed_start[..target_len];
    let Some(asset) = asset_path_from_target(target) else {
        return body.to_string();
    };
    let mut out = String::new();
    out.push_str(&body[..leading]);
    out.push_str(&transform_target(&asset));
    out.push_str(&trimmed_start[target_len..]);
    out
}

fn transform_wiki_asset_references_in_line<F>(line: &str, transform_target: &F) -> String
where
    F: Fn(&Path) -> String,
{
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(start) = rest.find("[[") {
        out.push_str(&rest[..start + 2]);
        rest = &rest[start + 2..];
        let Some(end) = rest.find("]]") else {
            out.push_str(rest);
            return out;
        };
        let body = &rest[..end];
        out.push_str(&transform_wiki_asset_body(body, transform_target));
        out.push_str("]]");
        rest = &rest[end + 2..];
    }
    out.push_str(rest);
    out
}

fn transform_wiki_asset_body<F>(body: &str, transform_target: &F) -> String
where
    F: Fn(&Path) -> String,
{
    let target_end = body.find('|').unwrap_or(body.len());
    let target = body[..target_end].trim();
    let Some(asset) = asset_path_from_target(target) else {
        return body.to_string();
    };
    format!("{}{}", transform_target(&asset), &body[target_end..])
}

fn for_non_code_line<F>(content: &str, mut visit: F)
where
    F: FnMut(&str),
{
    let mut fenced_marker: Option<(u8, usize)> = None;
    for line in content.lines() {
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
        let no_inline_code = strip_inline_code_segments(line);
        visit(&no_inline_code);
    }
}

fn strip_inline_code_segments(line: &str) -> String {
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
            if inline_marker_len == 0 {
                inline_marker_len = marker_len;
            } else if marker_len == inline_marker_len {
                inline_marker_len = 0;
            }
            idx += marker_len;
            continue;
        }
        if inline_marker_len == 0 {
            out.push(chars[idx]);
        }
        idx += 1;
    }
    out
}

fn extract_markdown_assets(line: &str, assets: &mut Vec<PathBuf>) {
    let mut rest = line;
    while let Some(start) = rest.find("](") {
        rest = &rest[start + 2..];
        let Some(end) = rest.find(')') else {
            break;
        };
        let target = rest[..end].split_whitespace().next().unwrap_or("").trim();
        if let Some(asset) = asset_path_from_target(target) {
            assets.push(asset);
        }
        rest = &rest[end + 1..];
    }
}

fn extract_wiki_assets(line: &str, assets: &mut Vec<PathBuf>) {
    let mut rest = line;
    while let Some(start) = rest.find("[[") {
        rest = &rest[start + 2..];
        let Some(end) = rest.find("]]") else {
            break;
        };
        let target = rest[..end].split('|').next().unwrap_or("").trim();
        if let Some(asset) = asset_path_from_target(target) {
            assets.push(asset);
        }
        rest = &rest[end + 2..];
    }
}

fn asset_path_from_target(target: &str) -> Option<PathBuf> {
    let target = target.trim();
    if target.is_empty()
        || target.starts_with("http://")
        || target.starts_with("https://")
        || target.starts_with('#')
        || Path::new(target).is_absolute()
    {
        return None;
    }
    let path = Path::new(target);
    if path.components().any(|component| {
        !matches!(
            component,
            Component::Normal(_) | Component::CurDir | Component::ParentDir
        )
    }) {
        return None;
    }
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "avif" | "bmp" | "pdf"
    ) {
        Some(path.to_path_buf())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn trashing_an_asset_whose_name_already_exists_in_trash_picks_a_unique_name() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();

        // A note referencing an attachment, and the attachment itself.
        fs::write(
            root.join("Note.md"),
            "# Note\n![img](Attachments/foo.png)\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("Attachments")).unwrap();
        fs::write(root.join("Attachments/foo.png"), "img").unwrap();

        // A previous delete already trashed an asset of the same relative path.
        fs::create_dir_all(root.join(".hatchdoor-trash/Attachments")).unwrap();
        fs::write(root.join(".hatchdoor-trash/Attachments/foo.png"), "older").unwrap();

        let index = VaultIndex::build(root).expect("index");
        let entry = index
            .ordered_entries()
            .into_iter()
            .find(|e| e.slug == "note")
            .expect("note entry");
        let trash_note = root.join(".hatchdoor-trash/Note.md");

        // Deleting to trash must not fail just because the asset name already
        // exists in trash; it should relocate to a unique name instead.
        let (moves, _rewrites) = asset_move_plan(root, &index, &entry, &trash_note, true, &[])
            .expect("plan must succeed");
        assert_eq!(moves.len(), 1);
        assert_eq!(
            moves[0].destination,
            root.join(".hatchdoor-trash/Attachments/foo-2.png"),
            "colliding trashed asset should get a unique suffix"
        );
    }
}
