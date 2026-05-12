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
use super::rewrites::{apply_rewrites, backlink_rewrite_plan, merge_rewrites};
use super::types::{WriteError, WriteOutcome};

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
