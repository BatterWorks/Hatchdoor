use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::cache::parse::content_hash;

use super::paths::strip_md_extension;
use super::types::{NoteEntry, VaultIndex};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WriteOutcome {
    pub(crate) slug: Option<String>,
    pub(crate) relative_path: Option<String>,
    pub(crate) content_hash: Option<String>,
    pub(crate) rewritten_notes: usize,
    pub(crate) moved_assets: usize,
    pub(crate) trashed_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WriteError {
    Conflict(String),
    InvalidInput(String),
    Io(String),
}

#[derive(Debug, Clone)]
struct TextRewrite {
    path: PathBuf,
    content: String,
}

#[derive(Debug, Clone)]
struct AssetMove {
    source: PathBuf,
    destination: PathBuf,
}

pub(crate) fn create_note(
    vault_root: &Path,
    relative_path: &str,
    content: &str,
    overwrite: bool,
) -> Result<WriteOutcome, WriteError> {
    let path = resolve_new_note_path(vault_root, relative_path)?;
    if path.exists() && !overwrite {
        return Err(WriteError::Conflict(format!(
            "Note already exists: {}",
            normalize_note_relative_path(relative_path)?
        )));
    }

    create_parent_dir_inside_root(vault_root, &path, "note")?;

    atomic_write(&path, content)?;
    let normalized = normalize_note_relative_path(relative_path)?;
    Ok(WriteOutcome {
        slug: None,
        relative_path: Some(strip_md_extension(&normalized).to_string()),
        content_hash: Some(content_hash(content)),
        rewritten_notes: 0,
        moved_assets: 0,
        trashed_path: None,
    })
}

pub(crate) fn update_note(
    entry: &NoteEntry,
    content: &str,
    expected_content_hash: &str,
) -> Result<WriteOutcome, WriteError> {
    ensure_content_hash(entry, expected_content_hash)?;
    atomic_write(&entry.path, content)?;
    Ok(WriteOutcome {
        slug: Some(entry.slug.clone()),
        relative_path: Some(entry.relative_path.clone()),
        content_hash: Some(content_hash(content)),
        rewritten_notes: 0,
        moved_assets: 0,
        trashed_path: None,
    })
}

pub(crate) fn append_note(
    entry: &NoteEntry,
    content: &str,
    expected_content_hash: &str,
) -> Result<WriteOutcome, WriteError> {
    ensure_content_hash(entry, expected_content_hash)?;
    let mut current = fs::read_to_string(&entry.path).map_err(|error| {
        WriteError::Io(format!(
            "failed to read note '{}': {error}",
            entry.relative_path
        ))
    })?;
    if !current.ends_with('\n') {
        current.push('\n');
    }
    current.push_str(content);
    atomic_write(&entry.path, &current)?;
    Ok(WriteOutcome {
        slug: Some(entry.slug.clone()),
        relative_path: Some(entry.relative_path.clone()),
        content_hash: Some(content_hash(&current)),
        rewritten_notes: 0,
        moved_assets: 0,
        trashed_path: None,
    })
}

pub(crate) fn move_or_rename_note(
    vault_root: &Path,
    index: &VaultIndex,
    entry: &NoteEntry,
    target_relative_path: &str,
    expected_content_hash: &str,
) -> Result<WriteOutcome, WriteError> {
    ensure_content_hash(entry, expected_content_hash)?;
    let target_path = resolve_new_note_path(vault_root, target_relative_path)?;
    if target_path.exists() {
        return Err(WriteError::Conflict(format!(
            "Destination note already exists: {}",
            normalize_note_relative_path(target_relative_path)?
        )));
    }
    create_parent_dir_inside_root(vault_root, &target_path, "destination")?;

    let target_without_ext =
        strip_md_extension(&normalize_note_relative_path(target_relative_path)?).to_string();
    let backlink_rewrites = backlink_rewrite_plan(index, &entry.slug, Some(&target_without_ext))?;
    let (asset_moves, asset_rewrites) = asset_move_plan(
        vault_root,
        index,
        entry,
        &target_path,
        false,
        &backlink_rewrites,
    )?;
    fs::rename(&entry.path, &target_path).map_err(|error| {
        WriteError::Io(format!(
            "failed to move note '{}' to '{}': {error}",
            entry.path.display(),
            target_path.display()
        ))
    })?;
    let moved_assets = move_assets(&asset_moves)?;
    let rewritten_notes = apply_rewrites(merge_rewrites(backlink_rewrites, asset_rewrites))?;
    let moved_content = fs::read_to_string(&target_path).map_err(|error| {
        WriteError::Io(format!(
            "failed to read moved note '{}': {error}",
            target_path.display()
        ))
    })?;

    Ok(WriteOutcome {
        slug: None,
        relative_path: Some(target_without_ext),
        content_hash: Some(content_hash(&moved_content)),
        rewritten_notes,
        moved_assets,
        trashed_path: None,
    })
}

pub(crate) fn delete_note(
    vault_root: &Path,
    index: &VaultIndex,
    entry: &NoteEntry,
    expected_content_hash: &str,
) -> Result<WriteOutcome, WriteError> {
    ensure_content_hash(entry, expected_content_hash)?;
    let trash_relative = unique_trash_relative_path(vault_root, &entry.relative_path)?;
    let trash_path = vault_root.join(format!("{trash_relative}.md"));
    create_parent_dir_inside_root(vault_root, &trash_path, "trash")?;

    let backlink_rewrites = backlink_rewrite_plan(index, &entry.slug, None)?;
    let (asset_moves, asset_rewrites) = asset_move_plan(
        vault_root,
        index,
        entry,
        &trash_path,
        true,
        &backlink_rewrites,
    )?;
    let moved_assets = move_assets(&asset_moves)?;
    fs::rename(&entry.path, &trash_path).map_err(|error| {
        WriteError::Io(format!(
            "failed to move note '{}' to trash '{}': {error}",
            entry.path.display(),
            trash_path.display()
        ))
    })?;
    let rewritten_notes = apply_rewrites(merge_rewrites(backlink_rewrites, asset_rewrites))?;

    Ok(WriteOutcome {
        slug: Some(entry.slug.clone()),
        relative_path: Some(entry.relative_path.clone()),
        content_hash: None,
        rewritten_notes,
        moved_assets,
        trashed_path: Some(trash_relative),
    })
}

pub(crate) fn normalize_note_relative_path(input: &str) -> Result<String, WriteError> {
    let trimmed = input.trim().replace('\\', "/");
    if trimmed.is_empty() {
        return Err(WriteError::InvalidInput(
            "note path cannot be empty".to_string(),
        ));
    }
    let path = Path::new(&trimmed);
    if path.is_absolute() {
        return Err(WriteError::InvalidInput(
            "note path must be vault-relative".to_string(),
        ));
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => {
                let Some(part) = value.to_str() else {
                    return Err(WriteError::InvalidInput(
                        "note path must be valid UTF-8".to_string(),
                    ));
                };
                if part.is_empty() {
                    return Err(WriteError::InvalidInput(
                        "note path cannot contain empty segments".to_string(),
                    ));
                }
                parts.push(part.to_string());
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(WriteError::InvalidInput(
                    "note path cannot escape the vault".to_string(),
                ));
            }
        }
    }
    if parts.is_empty() {
        return Err(WriteError::InvalidInput(
            "note path cannot be empty".to_string(),
        ));
    }
    let mut normalized = parts.join("/");
    if !normalized.ends_with(".md") {
        normalized.push_str(".md");
    }
    if normalized == ".md" || normalized.ends_with("/.md") {
        return Err(WriteError::InvalidInput(
            "note filename cannot be empty".to_string(),
        ));
    }
    Ok(normalized)
}

fn resolve_new_note_path(vault_root: &Path, relative_path: &str) -> Result<PathBuf, WriteError> {
    let root = canonical_root(vault_root)?;
    let normalized = normalize_note_relative_path(relative_path)?;
    let path = vault_root.join(&normalized);
    if let Some(parent) = path.parent()
        && parent.exists()
    {
        ensure_existing_path_inside_root(&root, parent)?;
    }
    Ok(path)
}

fn canonical_root(root: &Path) -> Result<PathBuf, WriteError> {
    fs::canonicalize(root).map_err(|error| {
        WriteError::Io(format!(
            "failed to canonicalize vault root '{}': {error}",
            root.display()
        ))
    })
}

fn ensure_existing_path_inside_root(root: &Path, path: &Path) -> Result<(), WriteError> {
    let root = canonical_root(root)?;
    let path = fs::canonicalize(path).map_err(|error| {
        WriteError::Io(format!(
            "failed to canonicalize path '{}': {error}",
            path.display()
        ))
    })?;
    if !path.starts_with(&root) {
        return Err(WriteError::InvalidInput(
            "path cannot escape the vault".to_string(),
        ));
    }
    Ok(())
}

fn create_parent_dir_inside_root(
    vault_root: &Path,
    path: &Path,
    label: &str,
) -> Result<(), WriteError> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    let root = canonical_root(vault_root)?;
    let nearest_existing = nearest_existing_ancestor(parent);
    ensure_existing_path_inside_root(&root, &nearest_existing)?;
    fs::create_dir_all(parent).map_err(|error| {
        WriteError::Io(format!(
            "failed to create {label} directory '{}': {error}",
            parent.display()
        ))
    })?;
    ensure_existing_path_inside_root(&root, parent)
}

fn nearest_existing_ancestor(path: &Path) -> PathBuf {
    let mut current = path;
    while !current.exists() {
        let Some(parent) = current.parent() else {
            break;
        };
        current = parent;
    }
    current.to_path_buf()
}

fn ensure_content_hash(entry: &NoteEntry, expected: &str) -> Result<(), WriteError> {
    let expected = expected.trim();
    if expected.is_empty() {
        return Err(WriteError::InvalidInput(
            "expected_content_hash cannot be empty".to_string(),
        ));
    }
    let content = fs::read_to_string(&entry.path).map_err(|error| {
        WriteError::Io(format!(
            "failed to read note '{}': {error}",
            entry.relative_path
        ))
    })?;
    let actual = content_hash(&content);
    if actual != expected {
        return Err(WriteError::Conflict(format!(
            "note changed since it was read: expected {expected}, found {actual}"
        )));
    }
    Ok(())
}

fn atomic_write(path: &Path, content: &str) -> Result<(), WriteError> {
    let tmp = path.with_extension("md.hatchdoor-tmp");
    fs::write(&tmp, content).map_err(|error| {
        WriteError::Io(format!(
            "failed to write temporary note '{}': {error}",
            tmp.display()
        ))
    })?;
    fs::rename(&tmp, path).map_err(|error| {
        let _ = fs::remove_file(&tmp);
        WriteError::Io(format!(
            "failed to replace note '{}': {error}",
            path.display()
        ))
    })
}

fn backlink_rewrite_plan(
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

fn transform_wikilinks<F>(content: &str, transform_target: F) -> String
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

fn transform_wikilinks_in_line<F>(line: &str, transform_target: &F) -> String
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

fn parse_fence_marker(trimmed_line: &str) -> Option<(u8, usize)> {
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

fn asset_move_plan(
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
        let destination_asset = destination_dir.join(&relative_asset);
        create_parent_dir_inside_root(vault_root, &destination_asset, "asset")?;
        if destination_asset.exists() {
            if allow_trash_collision {
                return Err(WriteError::Conflict(format!(
                    "Destination asset already exists: {}",
                    destination_asset.display()
                )));
            }
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

fn referenced_assets(content: &str) -> Vec<PathBuf> {
    let mut assets = Vec::new();
    for_non_code_line(content, |line| {
        extract_markdown_assets(line, &mut assets);
        extract_wiki_assets(line, &mut assets);
    });
    assets
}

fn asset_reference_rewrite_plan(
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

fn move_assets(moves: &[AssetMove]) -> Result<usize, WriteError> {
    for asset in moves {
        fs::rename(&asset.source, &asset.destination).map_err(|error| {
            WriteError::Io(format!(
                "failed to move asset '{}' to '{}': {error}",
                asset.source.display(),
                asset.destination.display()
            ))
        })?;
    }
    Ok(moves.len())
}

fn apply_rewrites(rewrites: Vec<TextRewrite>) -> Result<usize, WriteError> {
    let mut changed = 0usize;
    for rewrite in rewrites {
        atomic_write(&rewrite.path, &rewrite.content)?;
        changed += 1;
    }
    Ok(changed)
}

fn merge_rewrites(left: Vec<TextRewrite>, right: Vec<TextRewrite>) -> Vec<TextRewrite> {
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

fn rewrite_content_or_read(path: &Path, rewrites: &[TextRewrite]) -> Result<String, io::Error> {
    rewrites
        .iter()
        .rev()
        .find(|rewrite| rewrite.path == path)
        .map(|rewrite| Ok(rewrite.content.clone()))
        .unwrap_or_else(|| fs::read_to_string(path))
}

fn same_existing_path(left: &Path, right: &Path) -> bool {
    match (fs::canonicalize(left), fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn relative_link_target(vault_root: &Path, from_note: &Path, target: &Path) -> Option<String> {
    let from_dir = from_note.parent().unwrap_or(vault_root);
    let from_relative = from_dir.strip_prefix(vault_root).ok()?;
    let target_relative = target.strip_prefix(vault_root).ok()?;
    let from_parts = path_parts(from_relative)?;
    let target_parts = path_parts(target_relative)?;
    let common_len = from_parts
        .iter()
        .zip(target_parts.iter())
        .take_while(|(left, right)| left == right)
        .count();
    let mut parts = Vec::new();
    parts.extend((common_len..from_parts.len()).map(|_| "..".to_string()));
    parts.extend(target_parts[common_len..].iter().cloned());
    if parts.is_empty() {
        target_parts.last().cloned()
    } else {
        Some(parts.join("/"))
    }
}

fn path_parts(path: &Path) -> Option<Vec<String>> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => parts.push(value.to_str()?.to_string()),
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(parts)
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

fn unique_trash_relative_path(
    vault_root: &Path,
    relative_path: &str,
) -> Result<String, WriteError> {
    let base = format!(".hatchdoor-trash/{relative_path}");
    let mut candidate = base.clone();
    let mut suffix = 2usize;
    while vault_root.join(format!("{candidate}.md")).exists() {
        candidate = format!("{base}-{suffix}");
        suffix += 1;
    }
    Ok(candidate)
}

impl From<io::Error> for WriteError {
    fn from(error: io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn build(root: &Path) -> VaultIndex {
        VaultIndex::build(root).expect("build index")
    }

    #[test]
    fn create_note_rejects_traversal_and_writes_markdown() {
        let tmp = TempDir::new().expect("tempdir");
        let root = tmp.path();
        assert!(matches!(
            create_note(root, "../Escape.md", "no", false),
            Err(WriteError::InvalidInput(_))
        ));
        create_note(root, "Projects/New", "# New", false).expect("create");
        assert_eq!(
            fs::read_to_string(root.join("Projects/New.md")).expect("read"),
            "# New"
        );
    }

    #[cfg(unix)]
    #[test]
    fn create_note_rejects_symlinked_parent_escape_before_creating_dirs() {
        use std::os::unix::fs::symlink;

        let tmp = TempDir::new().expect("tempdir");
        let root = tmp.path().join("vault");
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&root).expect("vault");
        fs::create_dir_all(&outside).expect("outside");
        symlink(&outside, root.join("link")).expect("symlink");

        assert!(matches!(
            create_note(&root, "link/Nested/Escape.md", "no", false),
            Err(WriteError::InvalidInput(_))
        ));
        assert!(!outside.join("Nested").exists());
    }

    #[test]
    fn update_note_requires_matching_hash() {
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("Home.md");
        fs::write(&path, "old").expect("write");
        let index = build(tmp.path());
        let entry = index.find_by_slug("home").expect("home");
        assert!(matches!(
            update_note(entry, "new", "fnv1a64:deadbeef"),
            Err(WriteError::Conflict(_))
        ));
        update_note(entry, "new", &content_hash("old")).expect("update");
        assert_eq!(fs::read_to_string(path).expect("read"), "new");
    }

    #[test]
    fn move_note_rewrites_backlinks_and_moves_referenced_assets() {
        let tmp = TempDir::new().expect("tempdir");
        let root = tmp.path();
        fs::create_dir_all(root.join("Notes")).expect("mkdir");
        fs::write(root.join("Notes/Target.md"), "body\n![](image.png)").expect("target");
        fs::write(root.join("Notes/image.png"), "png").expect("asset");
        fs::create_dir_all(root.join("Other")).expect("other dir");
        fs::write(
            root.join("Backlink.md"),
            "[[Target|Alias]] and ![](Notes/image.png) and `[[Target]]`\n```\n[[Target]]\n![](Notes/image.png)\n```",
        )
        .expect("backlink");
        fs::write(
            root.join("Other/Shared.md"),
            "shared ![](../Notes/image.png) and `![](../Notes/image.png)`",
        )
        .expect("shared");
        let index = build(root);
        let entry = index.find_by_slug("target").expect("target");

        let outcome = move_or_rename_note(
            root,
            &index,
            entry,
            "Archive/Renamed.md",
            &content_hash("body\n![](image.png)"),
        )
        .expect("move");

        assert_eq!(outcome.rewritten_notes, 2);
        assert_eq!(outcome.moved_assets, 1);
        assert!(root.join("Archive/Renamed.md").exists());
        assert!(root.join("Archive/image.png").exists());
        let backlink = fs::read_to_string(root.join("Backlink.md")).expect("read");
        assert!(backlink.contains("[[Archive/Renamed|Alias]]"));
        assert!(backlink.contains("![](Archive/image.png)"));
        assert!(backlink.contains("`[[Target]]`"));
        assert!(backlink.contains("```\n[[Target]]\n![](Notes/image.png)\n```"));
        let shared = fs::read_to_string(root.join("Other/Shared.md")).expect("shared");
        assert!(shared.contains("![](../Archive/image.png)"));
        assert!(shared.contains("`![](../Notes/image.png)`"));
    }

    #[test]
    fn delete_note_moves_note_and_assets_to_trash_and_removes_backlinks() {
        let tmp = TempDir::new().expect("tempdir");
        let root = tmp.path();
        fs::write(root.join("Target.md"), "body ![](asset.pdf)").expect("target");
        fs::write(root.join("asset.pdf"), "pdf").expect("asset");
        fs::write(
            root.join("Backlink.md"),
            "before [[Target]] after ![](asset.pdf)",
        )
        .expect("backlink");
        let index = build(root);
        let entry = index.find_by_slug("target").expect("target");

        let outcome =
            delete_note(root, &index, entry, &content_hash("body ![](asset.pdf)")).expect("delete");

        let trash = outcome.trashed_path.expect("trash path");
        assert!(!root.join("Target.md").exists());
        assert!(root.join(format!("{trash}.md")).exists());
        assert!(root.join(".hatchdoor-trash/asset.pdf").exists());
        let backlink = fs::read_to_string(root.join("Backlink.md")).expect("backlink");
        assert_eq!(backlink, "before  after ![](.hatchdoor-trash/asset.pdf)");
    }
}
