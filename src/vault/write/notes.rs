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

pub fn update_note(
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
    atomic_write(&entry.path, &updated)?;
    Ok(WriteOutcome {
        slug: Some(entry.slug.clone()),
        relative_path: Some(entry.relative_path.clone()),
        content_hash: Some(content_hash(&updated)),
        rewritten_notes: 0,
        moved_assets: 0,
        trashed_path: None,
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
    atomic_write(&entry.path, &updated)?;
    Ok(WriteOutcome {
        slug: Some(entry.slug.clone()),
        relative_path: Some(entry.relative_path.clone()),
        content_hash: Some(content_hash(&updated)),
        rewritten_notes: 0,
        moved_assets: 0,
        trashed_path: None,
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
