use std::fs;
use std::path::Path;

use crate::cache::parse::content_hash;
use crate::vault::paths::strip_md_extension;
use crate::vault::types::{NoteEntry, VaultIndex};

use super::assets::asset_move_plan;
use super::fs_ops::{atomic_write, ensure_content_hash, move_assets};
use super::paths::{
    create_parent_dir_inside_root, normalize_note_relative_path, resolve_new_note_path,
    unique_trash_relative_path,
};
use super::rewrites::{apply_rewrites, backlink_rewrite_plan, merge_rewrites, parse_fence_marker};
use super::types::{WriteError, WriteOutcome};

/// Where `replace_section` places the supplied content relative to the matched section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionMode {
    /// Replace the whole section (heading line through the body before the next same-or-higher heading).
    Replace,
    /// Insert the content immediately before the heading line, leaving the section intact.
    Before,
    /// Insert the content immediately after the section, leaving the section intact.
    After,
}

struct PreparedNoteContent {
    content: String,
    warnings: Vec<String>,
}

fn prepare_note_content(content: &str) -> Result<PreparedNoteContent, WriteError> {
    if content.contains('\0') {
        return Err(WriteError::InvalidInput(
            "note content cannot contain NUL bytes".to_string(),
        ));
    }

    let mut warnings = Vec::new();
    let mut normalized = content.replace("\r\n", "\n").replace('\r', "\n");
    if normalized != content {
        warnings.push("normalized CRLF/CR line endings to LF".to_string());
    }
    if !normalized.is_empty() && !normalized.ends_with('\n') {
        normalized.push('\n');
        warnings.push("added final newline".to_string());
    }
    warnings.extend(frontmatter_warnings(&normalized));

    Ok(PreparedNoteContent {
        content: normalized,
        warnings,
    })
}

fn frontmatter_warnings(content: &str) -> Vec<String> {
    let Some(rest) = content.strip_prefix("---\n") else {
        return Vec::new();
    };

    let mut warnings = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut closed = false;
    for line in rest.lines() {
        if line.trim() == "---" {
            closed = true;
            break;
        }
        if line.starts_with(char::is_whitespace) {
            continue;
        }
        let Some((key, _)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        if !seen.insert(key.to_string()) {
            warnings.push(format!("frontmatter has duplicate key: {key}"));
        }
    }
    if !closed {
        warnings.push("frontmatter opening marker has no closing marker".to_string());
    }
    warnings
}

pub fn create_note(
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

    let prepared = prepare_note_content(content)?;
    atomic_write(&path, &prepared.content)?;
    let normalized = normalize_note_relative_path(relative_path)?;
    Ok(WriteOutcome {
        slug: None,
        relative_path: Some(strip_md_extension(&normalized).to_string()),
        content_hash: Some(content_hash(&prepared.content)),
        quality_warnings: prepared.warnings,
        rewritten_notes: 0,
        moved_assets: 0,
        trashed_path: None,
        affected_paths: vec![path.clone()],
    })
}

pub fn update_note(
    entry: &NoteEntry,
    content: &str,
    expected_content_hash: &str,
) -> Result<WriteOutcome, WriteError> {
    ensure_content_hash(entry, expected_content_hash)?;
    let prepared = prepare_note_content(content)?;
    atomic_write(&entry.path, &prepared.content)?;
    Ok(WriteOutcome {
        slug: Some(entry.slug.clone()),
        relative_path: Some(entry.relative_path.clone()),
        content_hash: Some(content_hash(&prepared.content)),
        quality_warnings: prepared.warnings,
        rewritten_notes: 0,
        moved_assets: 0,
        trashed_path: None,
        affected_paths: vec![entry.path.clone()],
    })
}

pub fn append_note(
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
    let prepared = prepare_note_content(&current)?;
    atomic_write(&entry.path, &prepared.content)?;
    Ok(WriteOutcome {
        slug: Some(entry.slug.clone()),
        relative_path: Some(entry.relative_path.clone()),
        content_hash: Some(content_hash(&prepared.content)),
        quality_warnings: prepared.warnings,
        rewritten_notes: 0,
        moved_assets: 0,
        trashed_path: None,
        affected_paths: vec![entry.path.clone()],
    })
}

pub fn edit_note(
    entry: &NoteEntry,
    old_string: &str,
    new_string: &str,
    expected_content_hash: &str,
    replace_all: bool,
) -> Result<WriteOutcome, WriteError> {
    ensure_content_hash(entry, expected_content_hash)?;
    if old_string.is_empty() {
        return Err(WriteError::InvalidInput(
            "old_string cannot be empty".to_string(),
        ));
    }
    let current = read_note(entry)?;
    let matches = current.matches(old_string).count();
    match matches {
        0 => {
            return Err(WriteError::InvalidInput(format!(
                "old_string not found in note '{}'",
                entry.relative_path
            )));
        }
        count if count > 1 && !replace_all => {
            return Err(WriteError::Conflict(format!(
                "old_string is not unique in note '{}' ({count} matches); add surrounding context or pass replace_all",
                entry.relative_path
            )));
        }
        _ => {}
    }
    let updated = if replace_all {
        current.replace(old_string, new_string)
    } else {
        current.replacen(old_string, new_string, 1)
    };
    let prepared = prepare_note_content(&updated)?;
    atomic_write(&entry.path, &prepared.content)?;
    Ok(WriteOutcome {
        slug: Some(entry.slug.clone()),
        relative_path: Some(entry.relative_path.clone()),
        content_hash: Some(content_hash(&prepared.content)),
        quality_warnings: prepared.warnings,
        rewritten_notes: 0,
        moved_assets: 0,
        trashed_path: None,
        affected_paths: vec![entry.path.clone()],
    })
}

pub fn replace_section(
    entry: &NoteEntry,
    heading: &str,
    mode: SectionMode,
    content: &str,
    expected_content_hash: &str,
) -> Result<WriteOutcome, WriteError> {
    ensure_content_hash(entry, expected_content_hash)?;
    let requested = heading.trim();
    if !requested.starts_with('#') {
        return Err(WriteError::InvalidInput(
            "heading must start with one or more '#' characters".to_string(),
        ));
    }
    let current = read_note(entry)?;
    let (start, end) = section_span(&current, requested, &entry.relative_path)?;
    let updated = match mode {
        SectionMode::Replace => splice(&current[..start], content, &current[end..]),
        SectionMode::Before => splice(&current[..start], content, &current[start..]),
        SectionMode::After => splice(&current[..end], content, &current[end..]),
    };
    let prepared = prepare_note_content(&updated)?;
    atomic_write(&entry.path, &prepared.content)?;
    Ok(WriteOutcome {
        slug: Some(entry.slug.clone()),
        relative_path: Some(entry.relative_path.clone()),
        content_hash: Some(content_hash(&prepared.content)),
        quality_warnings: prepared.warnings,
        rewritten_notes: 0,
        moved_assets: 0,
        trashed_path: None,
        affected_paths: vec![entry.path.clone()],
    })
}

fn read_note(entry: &NoteEntry) -> Result<String, WriteError> {
    fs::read_to_string(&entry.path).map_err(|error| {
        WriteError::Io(format!(
            "failed to read note '{}': {error}",
            entry.relative_path
        ))
    })
}

/// All ATX headings in `content` that are not inside a fenced code block, as
/// `(byte offset of the heading line, heading level, trimmed heading line)`.
fn scan_headings(content: &str) -> Vec<(usize, usize, &str)> {
    let mut headings = Vec::new();
    let mut fenced_marker: Option<(u8, usize)> = None;
    let mut offset = 0usize;
    for line in content.split_inclusive('\n') {
        let body = line.strip_suffix('\n').unwrap_or(line);
        let trimmed = body.trim_start();
        if let Some((marker, min_len)) = fenced_marker {
            if let Some((close_marker, close_len)) = parse_fence_marker(trimmed)
                && close_marker == marker
                && close_len >= min_len
            {
                fenced_marker = None;
            }
        } else if let Some(marker) = parse_fence_marker(trimmed) {
            fenced_marker = Some(marker);
        } else {
            let level = trimmed.chars().take_while(|ch| *ch == '#').count();
            if (1..=6).contains(&level)
                && trimmed[level..]
                    .chars()
                    .next()
                    .is_some_and(char::is_whitespace)
            {
                headings.push((offset, level, trimmed.trim_end()));
            }
        }
        offset += line.len();
    }
    headings
}

/// Byte range `[start, end)` covering the requested section: the heading line
/// through the body that precedes the next same-or-higher heading (or EOF).
fn section_span(
    content: &str,
    requested: &str,
    relative_path: &str,
) -> Result<(usize, usize), WriteError> {
    let headings = scan_headings(content);
    let matched: Vec<usize> = headings
        .iter()
        .enumerate()
        .filter(|(_, (_, _, text))| *text == requested)
        .map(|(idx, _)| idx)
        .collect();
    match matched.as_slice() {
        [] => Err(WriteError::InvalidInput(format!(
            "heading '{requested}' not found in note '{relative_path}'"
        ))),
        [idx] => {
            let (start, level, _) = headings[*idx];
            let end = headings[idx + 1..]
                .iter()
                .find(|(_, candidate_level, _)| *candidate_level <= level)
                .map(|(offset, _, _)| *offset)
                .unwrap_or(content.len());
            Ok((start, end))
        }
        more => Err(WriteError::Conflict(format!(
            "heading '{requested}' is not unique in note '{relative_path}' ({} matches)",
            more.len()
        ))),
    }
}

/// Join `prefix + block + suffix`, guaranteeing newline separation so an
/// inserted block never glues onto adjacent lines.
fn splice(prefix: &str, block: &str, suffix: &str) -> String {
    let mut out = String::with_capacity(prefix.len() + block.len() + suffix.len() + 2);
    out.push_str(prefix);
    if !prefix.is_empty() && !prefix.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(block);
    if !suffix.is_empty() && !block.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(suffix);
    out
}

pub fn move_or_rename_note(
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
    let rewritten = apply_rewrites(merge_rewrites(backlink_rewrites, asset_rewrites))?;
    let rewritten_notes = rewritten.len();
    let moved_content = fs::read_to_string(&target_path).map_err(|error| {
        WriteError::Io(format!(
            "failed to read moved note '{}': {error}",
            target_path.display()
        ))
    })?;

    let mut affected_paths = rewritten;
    affected_paths.push(entry.path.clone());
    affected_paths.push(target_path.clone());
    for asset in &asset_moves {
        affected_paths.push(asset.source.clone());
        affected_paths.push(asset.destination.clone());
    }

    Ok(WriteOutcome {
        slug: None,
        relative_path: Some(target_without_ext),
        content_hash: Some(content_hash(&moved_content)),
        quality_warnings: Vec::new(),
        rewritten_notes,
        moved_assets,
        trashed_path: None,
        affected_paths,
    })
}

pub fn archive_note(
    vault_root: &Path,
    index: &VaultIndex,
    entry: &NoteEntry,
    archive_prefix: &str,
    expected_content_hash: &str,
) -> Result<WriteOutcome, WriteError> {
    let archive_folder = archive_prefix.trim().trim_matches('/');
    if archive_folder.is_empty() {
        return Err(WriteError::InvalidInput(
            "archive prefix cannot be empty".to_string(),
        ));
    }
    let file_name = entry
        .relative_path
        .rsplit('/')
        .next()
        .unwrap_or(&entry.relative_path);
    let target_relative_path = format!("{archive_folder}/{file_name}");
    if target_relative_path == entry.relative_path {
        return Err(WriteError::Conflict(format!(
            "Note is already archived: {}",
            entry.relative_path
        )));
    }
    move_or_rename_note(
        vault_root,
        index,
        entry,
        &target_relative_path,
        expected_content_hash,
    )
}

pub fn delete_note(
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
    // Trash the note FIRST, then move its assets (mirroring move_or_rename_note).
    // Doing assets first meant a failed note rename left a live note pointing at
    // attachments that had already been relocated into trash, with no rollback.
    fs::rename(&entry.path, &trash_path).map_err(|error| {
        WriteError::Io(format!(
            "failed to move note '{}' to trash '{}': {error}",
            entry.path.display(),
            trash_path.display()
        ))
    })?;
    let moved_assets = match move_assets(&asset_moves) {
        Ok(count) => count,
        Err(error) => {
            // move_assets is all-or-nothing, so no asset is stranded. Restore the
            // note out of trash so the whole delete is a no-op on failure.
            let _ = fs::rename(&trash_path, &entry.path);
            return Err(error);
        }
    };
    let rewritten = apply_rewrites(merge_rewrites(backlink_rewrites, asset_rewrites))?;
    let rewritten_notes = rewritten.len();

    let mut affected_paths = rewritten;
    affected_paths.push(entry.path.clone());
    affected_paths.push(trash_path.clone());
    for asset in &asset_moves {
        affected_paths.push(asset.source.clone());
        affected_paths.push(asset.destination.clone());
    }

    Ok(WriteOutcome {
        slug: Some(entry.slug.clone()),
        relative_path: Some(entry.relative_path.clone()),
        content_hash: None,
        quality_warnings: Vec::new(),
        rewritten_notes,
        moved_assets,
        trashed_path: Some(trash_relative),
        affected_paths,
    })
}
