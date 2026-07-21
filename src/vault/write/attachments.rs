use std::collections::HashSet;
use std::fs;
use std::path::Path;

use crate::vault::types::{NoteEntry, VaultIndex};

use super::assets::{asset_reference_rewrite_plan, referenced_assets};
use super::paths::{
    create_parent_dir_inside_root, ensure_allowed_attachment_path,
    ensure_existing_path_inside_root, normalize_attachment_relative_path,
    normalize_staged_filename, resolve_existing_attachment_path, resolve_new_attachment_path,
    unique_trash_attachment_relative_path, vault_relative_file_path,
};
use super::rewrites::{apply_rewrites, rollback_rewrites};
use super::types::{AttachmentInfo, AttachmentOutcome, WriteError};

pub fn list_note_attachments(
    vault_root: &Path,
    entry: &NoteEntry,
) -> Result<Vec<AttachmentInfo>, WriteError> {
    let content = fs::read_to_string(&entry.path).map_err(|error| {
        WriteError::Io(format!(
            "failed to read note '{}' for attachments: {error}",
            entry.relative_path
        ))
    })?;
    let note_dir = entry.path.parent().unwrap_or(vault_root);
    let mut attachments = Vec::new();
    let mut seen = HashSet::new();
    for relative_asset in referenced_assets(&content) {
        let path = note_dir.join(relative_asset);
        if !path.exists() || !path.is_file() {
            continue;
        }
        ensure_existing_path_inside_root(vault_root, &path)?;
        let Some(relative_path) = vault_relative_file_path(vault_root, &path)? else {
            continue;
        };
        if seen.insert(relative_path.clone()) {
            attachments.push(attachment_info(vault_root, &path)?);
        }
    }
    Ok(attachments)
}

pub fn import_attachment_bytes(
    vault_root: &Path,
    target_relative_path: &str,
    bytes: &[u8],
    max_bytes: u64,
    overwrite: bool,
) -> Result<AttachmentOutcome, WriteError> {
    let target_path = resolve_new_attachment_path(vault_root, target_relative_path)?;
    ensure_allowed_attachment_path(&target_path)?;
    create_parent_dir_inside_root(vault_root, &target_path, "attachment")?;

    let size = bytes.len().min(u64::MAX as usize) as u64;
    if size > max_bytes {
        return Err(WriteError::InvalidInput(format!(
            "attachment exceeds max size: {size} > {max_bytes}",
        )));
    }
    if target_path.exists() && !overwrite {
        return Err(WriteError::Conflict(format!(
            "Attachment already exists: {}",
            normalize_attachment_relative_path(target_relative_path)?
        )));
    }

    fs::write(&target_path, bytes).map_err(|error| {
        WriteError::Io(format!(
            "failed to write attachment '{}': {error}",
            target_path.display()
        ))
    })?;

    Ok(AttachmentOutcome {
        attachment: attachment_info(vault_root, &target_path)?,
        rewritten_notes: 0,
        trashed_path: None,
        cleanup_warning: None,
        affected_paths: vec![target_path],
    })
}

pub fn move_attachment(
    vault_root: &Path,
    index: &VaultIndex,
    source_relative_path: &str,
    target_relative_path: &str,
) -> Result<AttachmentOutcome, WriteError> {
    let source_path = resolve_existing_attachment_path(vault_root, source_relative_path)?;
    let target_path = resolve_new_attachment_path(vault_root, target_relative_path)?;
    if target_path.exists() {
        return Err(WriteError::Conflict(format!(
            "Destination attachment already exists: {}",
            normalize_attachment_relative_path(target_relative_path)?
        )));
    }
    ensure_allowed_attachment_path(&source_path)?;
    ensure_allowed_attachment_path(&target_path)?;
    create_parent_dir_inside_root(vault_root, &target_path, "attachment")?;

    let rewrites =
        asset_reference_rewrite_plan(vault_root, index, "", &source_path, &target_path, &[])?;
    let rewritten = apply_rewrites(rewrites)?;
    let rewritten_notes = rewritten.len();
    fs::rename(&source_path, &target_path).map_err(|error| {
        rollback_rewrites(vault_root, index, &target_path, &source_path);
        WriteError::Io(format!(
            "failed to move attachment '{}' to '{}': {error}",
            source_path.display(),
            target_path.display()
        ))
    })?;
    let mut affected_paths = rewritten;
    affected_paths.push(source_path.clone());
    affected_paths.push(target_path.clone());
    Ok(AttachmentOutcome {
        attachment: attachment_info(vault_root, &target_path)?,
        rewritten_notes,
        trashed_path: None,
        cleanup_warning: None,
        affected_paths,
    })
}

pub fn rename_attachment(
    vault_root: &Path,
    index: &VaultIndex,
    source_relative_path: &str,
    new_filename: &str,
) -> Result<AttachmentOutcome, WriteError> {
    let source_path = resolve_existing_attachment_path(vault_root, source_relative_path)?;
    let filename = normalize_staged_filename(new_filename)?;
    let target_path = source_path.parent().unwrap_or(vault_root).join(filename);
    move_attachment_by_paths(vault_root, index, &source_path, &target_path)
}

pub fn delete_attachment(
    vault_root: &Path,
    index: &VaultIndex,
    source_relative_path: &str,
) -> Result<AttachmentOutcome, WriteError> {
    let source_path = resolve_existing_attachment_path(vault_root, source_relative_path)?;
    ensure_allowed_attachment_path(&source_path)?;
    let trash_relative = unique_trash_attachment_relative_path(vault_root, source_relative_path)?;
    let trash_path = vault_root.join(&trash_relative);
    ensure_allowed_attachment_path(&trash_path)?;
    create_parent_dir_inside_root(vault_root, &trash_path, "trash")?;
    let rewrites =
        asset_reference_rewrite_plan(vault_root, index, "", &source_path, &trash_path, &[])?;

    let rewritten = apply_rewrites(rewrites)?;
    let rewritten_notes = rewritten.len();
    fs::rename(&source_path, &trash_path).map_err(|error| {
        rollback_rewrites(vault_root, index, &trash_path, &source_path);
        WriteError::Io(format!(
            "failed to trash attachment '{}' to '{}': {error}",
            source_path.display(),
            trash_path.display()
        ))
    })?;
    let mut affected_paths = rewritten;
    affected_paths.push(source_path.clone());
    affected_paths.push(trash_path.clone());
    Ok(AttachmentOutcome {
        attachment: attachment_info(vault_root, &trash_path)?,
        rewritten_notes,
        trashed_path: Some(trash_relative),
        cleanup_warning: None,
        affected_paths,
    })
}

fn move_attachment_by_paths(
    vault_root: &Path,
    index: &VaultIndex,
    source_path: &Path,
    target_path: &Path,
) -> Result<AttachmentOutcome, WriteError> {
    if target_path.exists() {
        return Err(WriteError::Conflict(format!(
            "Destination attachment already exists: {}",
            target_path.display()
        )));
    }
    ensure_existing_path_inside_root(vault_root, source_path)?;
    ensure_allowed_attachment_path(source_path)?;
    ensure_allowed_attachment_path(target_path)?;
    create_parent_dir_inside_root(vault_root, target_path, "attachment")?;
    let rewrites =
        asset_reference_rewrite_plan(vault_root, index, "", source_path, target_path, &[])?;
    let rewritten = apply_rewrites(rewrites)?;
    let rewritten_notes = rewritten.len();
    fs::rename(source_path, target_path).map_err(|error| {
        rollback_rewrites(vault_root, index, target_path, source_path);
        WriteError::Io(format!(
            "failed to move attachment '{}' to '{}': {error}",
            source_path.display(),
            target_path.display()
        ))
    })?;
    let mut affected_paths = rewritten;
    affected_paths.push(source_path.to_path_buf());
    affected_paths.push(target_path.to_path_buf());
    Ok(AttachmentOutcome {
        attachment: attachment_info(vault_root, target_path)?,
        rewritten_notes,
        trashed_path: None,
        cleanup_warning: None,
        affected_paths,
    })
}

fn attachment_info(vault_root: &Path, path: &Path) -> Result<AttachmentInfo, WriteError> {
    let relative_path = vault_relative_file_path(vault_root, path)?.ok_or_else(|| {
        WriteError::InvalidInput("attachment path cannot escape the vault".to_string())
    })?;
    let bytes = fs::read(path).map_err(|error| {
        WriteError::Io(format!(
            "failed to read attachment '{}': {error}",
            path.display()
        ))
    })?;
    Ok(AttachmentInfo {
        relative_path,
        size_bytes: bytes.len().min(u64::MAX as usize) as u64,
        content_hash: bytes_hash(&bytes),
    })
}

fn bytes_hash(bytes: &[u8]) -> String {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut hash = FNV_OFFSET;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }

    format!("fnv1a64:{hash:016x}")
}
