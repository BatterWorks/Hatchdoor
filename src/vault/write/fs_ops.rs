use std::fs;
use std::path::Path;

use crate::cache::parse::content_hash;
use crate::vault::types::NoteEntry;

use super::types::{AssetMove, WriteError};

pub(super) fn ensure_content_hash(entry: &NoteEntry, expected: &str) -> Result<(), WriteError> {
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

pub(super) fn atomic_write(path: &Path, content: &str) -> Result<(), WriteError> {
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

pub(super) fn move_assets(moves: &[AssetMove]) -> Result<usize, WriteError> {
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

pub(super) fn import_attachment_file(
    source_path: &Path,
    target_path: &Path,
) -> Result<Option<String>, WriteError> {
    match fs::rename(source_path, target_path) {
        Ok(()) => return Ok(None),
        Err(rename_error) => {
            fs::copy(source_path, target_path).map_err(|copy_error| {
                WriteError::Io(format!(
                    "failed to import attachment '{}' to '{}': rename failed: {rename_error}; copy failed: {copy_error}",
                    source_path.display(),
                    target_path.display()
                ))
            })?;
        }
    }

    match fs::remove_file(source_path) {
        Ok(()) => Ok(None),
        Err(error) => Ok(Some(format!(
            "import succeeded but failed to remove staged attachment '{}': {error}",
            source_path.display()
        ))),
    }
}
