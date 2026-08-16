use std::collections::HashSet;
use std::fs;
use std::path::Path;

use crate::vault::LayerMap;
use crate::vault::types::{NoteEntry, VaultIndex};

use super::assets::{asset_reference_rewrite_plan, referenced_assets};
use super::fs_ops::{MutationJournal, atomic_write_bytes};
use super::paths::{
    create_parent_dir_inside_root, ensure_allowed_attachment_path,
    ensure_existing_path_inside_root, normalize_attachment_relative_path,
    normalize_staged_filename, resolve_existing_attachment_path, resolve_new_attachment_path,
    unique_trash_attachment_relative_path, vault_relative_file_path,
};
use super::types::{AttachmentInfo, AttachmentOutcome, MutationPhase, WriteError};

pub fn list_note_attachments(
    vault_root: &Path,
    layers: &LayerMap,
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
            attachments.push(attachment_info(vault_root, &path, layers)?);
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

    // Resolve markers before mutating the filesystem. A malformed marker must
    // fail this request atomically rather than leaving a persisted attachment
    // behind after the response reports an error.
    let layers = LayerMap::collect(vault_root).map_err(WriteError::Io)?;

    atomic_write_bytes(&target_path, bytes)?;

    // No caller-supplied index here (imports do not rebuild one), so use the
    // marker snapshot collected before the write to report the new asset's layer.
    Ok(AttachmentOutcome {
        attachment: attachment_info(vault_root, &target_path, &layers)?,
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
    move_attachment_with_hook(
        vault_root,
        index,
        source_relative_path,
        target_relative_path,
        |_| Ok(()),
    )
}

fn move_attachment_with_hook(
    vault_root: &Path,
    index: &VaultIndex,
    source_relative_path: &str,
    target_relative_path: &str,
    after_phase: impl FnMut(MutationPhase) -> Result<(), WriteError>,
) -> Result<AttachmentOutcome, WriteError> {
    let source_path = resolve_existing_attachment_path(vault_root, source_relative_path)?;
    let target_path = resolve_new_attachment_path(vault_root, target_relative_path)?;
    move_attachment_by_paths_with_hook(
        vault_root,
        index,
        &source_path,
        &target_path,
        None,
        after_phase,
    )
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
    move_attachment_by_paths_with_hook(vault_root, index, &source_path, &target_path, None, |_| {
        Ok(())
    })
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
    move_attachment_by_paths_with_hook(
        vault_root,
        index,
        &source_path,
        &trash_path,
        Some(trash_relative),
        |_| Ok(()),
    )
}

fn move_attachment_by_paths_with_hook(
    vault_root: &Path,
    index: &VaultIndex,
    source_path: &Path,
    target_path: &Path,
    trashed_path: Option<String>,
    mut after_phase: impl FnMut(MutationPhase) -> Result<(), WriteError>,
) -> Result<AttachmentOutcome, WriteError> {
    if target_path.exists() {
        let target = target_path
            .strip_prefix(vault_root)
            .unwrap_or(target_path)
            .display();
        return Err(WriteError::Conflict(format!(
            "Destination attachment already exists: {}",
            target
        )));
    }
    ensure_existing_path_inside_root(vault_root, source_path)?;
    ensure_allowed_attachment_path(source_path)?;
    ensure_allowed_attachment_path(target_path)?;
    create_parent_dir_inside_root(vault_root, target_path, "attachment")?;
    let rewrites =
        asset_reference_rewrite_plan(vault_root, index, "", source_path, target_path, &[])?;
    let mut journal = MutationJournal::new(vault_root);
    if let Err(error) = journal.move_file(MutationPhase::Asset, source_path, target_path) {
        return Err(journal.rollback(error));
    }
    if let Err(error) = after_phase(MutationPhase::Asset) {
        return Err(journal.rollback(error));
    }

    let had_rewrites = !rewrites.is_empty();
    let rewritten = match journal.apply_rewrites(rewrites) {
        Ok(rewritten) => rewritten,
        Err(error) => return Err(journal.rollback(error)),
    };
    if had_rewrites && let Err(error) = after_phase(MutationPhase::Rewrite) {
        return Err(journal.rollback(error));
    }
    let rewritten_notes = rewritten.len();
    let attachment = match attachment_info(vault_root, target_path, &index.layers) {
        Ok(attachment) => attachment,
        Err(error) => return Err(journal.rollback(error)),
    };
    let mut affected_paths = rewritten;
    affected_paths.push(source_path.to_path_buf());
    affected_paths.push(target_path.to_path_buf());
    Ok(AttachmentOutcome {
        attachment,
        rewritten_notes,
        trashed_path,
        cleanup_warning: None,
        affected_paths,
    })
}

#[cfg(test)]
pub(super) fn move_attachment_with_failure(
    vault_root: &Path,
    index: &VaultIndex,
    source_relative_path: &str,
    target_relative_path: &str,
    after_phase: impl FnMut(MutationPhase) -> Result<(), WriteError>,
) -> Result<AttachmentOutcome, WriteError> {
    move_attachment_with_hook(
        vault_root,
        index,
        source_relative_path,
        target_relative_path,
        after_phase,
    )
}

fn attachment_info(
    vault_root: &Path,
    path: &Path,
    layers: &LayerMap,
) -> Result<AttachmentInfo, WriteError> {
    let relative_path = vault_relative_file_path(vault_root, path)?.ok_or_else(|| {
        WriteError::InvalidInput("attachment path cannot escape the vault".to_string())
    })?;
    let bytes = fs::read(path).map_err(|error| {
        WriteError::Io(format!(
            "failed to read attachment '{}': {error}",
            path.display()
        ))
    })?;
    // An asset's layer is its containing folder's layer — the same longest-prefix
    // resolution the index uses for notes. Reported, never filtered on: an
    // embedded image in a demoted note must stay fetchable.
    let layer = layers.layer_for(&relative_path).map(str::to_string);
    Ok(AttachmentInfo {
        relative_path,
        size_bytes: bytes.len().min(u64::MAX as usize) as u64,
        content_hash: bytes_hash(&bytes),
        layer,
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn import_attachment_rejects_a_malformed_marker_before_writing() {
        let vault = tempdir().expect("vault");
        let sources = vault.path().join("sources");
        fs::create_dir_all(&sources).expect("sources directory");
        fs::write(sources.join(".hatchdoor-layer"), "name: all\n").expect("marker");

        let target = "sources/image.png";
        let error = import_attachment_bytes(vault.path(), target, b"image", 1024, false)
            .expect_err("malformed marker must reject the import");

        assert!(matches!(error, WriteError::Io(_)));
        assert!(
            !vault.path().join(target).exists(),
            "a failed import must not leave an attachment on disk"
        );
    }
}
