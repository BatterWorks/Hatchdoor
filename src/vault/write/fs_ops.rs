use std::fs;
use std::io::Write;
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

    // Write and fsync the temp file so its bytes are durable BEFORE the rename.
    // Without the fsync, a crash just after the rename can leave the note file's
    // name pointing at data the OS never flushed (an empty or truncated file).
    let mut file = fs::File::create(&tmp).map_err(|error| {
        WriteError::Io(format!(
            "failed to create temporary note '{}': {error}",
            tmp.display()
        ))
    })?;
    file.write_all(content.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|error| {
            let _ = fs::remove_file(&tmp);
            WriteError::Io(format!(
                "failed to write temporary note '{}': {error}",
                tmp.display()
            ))
        })?;
    drop(file);

    fs::rename(&tmp, path).map_err(|error| {
        let _ = fs::remove_file(&tmp);
        WriteError::Io(format!(
            "failed to replace note '{}': {error}",
            path.display()
        ))
    })?;

    // fsync the parent directory so the rename itself (a directory metadata
    // change) survives a crash — otherwise the durable temp data can still be
    // lost if the directory entry update was not flushed.
    if let Some(parent) = path.parent()
        && let Ok(dir) = fs::File::open(parent)
    {
        let _ = dir.sync_all();
    }
    Ok(())
}

/// Move a set of assets, all-or-nothing. If any move fails, the assets already
/// moved in this call are renamed back (best-effort) so callers never observe a
/// partially-moved set. Combined with rename-note-first ordering, this keeps a
/// note and its attachments consistent even when a move fails midway.
pub(super) fn move_assets(moves: &[AssetMove]) -> Result<usize, WriteError> {
    let mut moved: Vec<&AssetMove> = Vec::new();
    for asset in moves {
        if let Err(error) = fs::rename(&asset.source, &asset.destination) {
            // Roll back the assets already moved in this call, in reverse order.
            for done in moved.iter().rev() {
                let _ = fs::rename(&done.destination, &done.source);
            }
            return Err(WriteError::Io(format!(
                "failed to move asset '{}' to '{}': {error}",
                asset.source.display(),
                asset.destination.display()
            )));
        }
        moved.push(asset);
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn atomic_write_persists_content_and_leaves_no_temp_file() {
        let dir = tempdir().expect("tempdir");
        let note = dir.path().join("Note.md");
        atomic_write(&note, "# Note\nbody\n").expect("write");
        assert_eq!(fs::read_to_string(&note).unwrap(), "# Note\nbody\n");
        // The temp sidecar must be renamed away, not left behind.
        assert!(!note.with_extension("md.hatchdoor-tmp").exists());

        // Overwriting an existing note replaces content atomically.
        atomic_write(&note, "# Note\nupdated\n").expect("overwrite");
        assert_eq!(fs::read_to_string(&note).unwrap(), "# Note\nupdated\n");
        assert!(!note.with_extension("md.hatchdoor-tmp").exists());
    }

    #[test]
    fn move_assets_rolls_back_already_moved_on_failure() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        // Two source assets that exist; a valid destination dir for the first,
        // and a destination under a NON-existent dir for the second so its move
        // fails deterministically (ENOENT).
        let src_a = root.join("a.png");
        let src_b = root.join("b.png");
        fs::write(&src_a, "a").unwrap();
        fs::write(&src_b, "b").unwrap();
        let dst_dir = root.join("dst");
        fs::create_dir(&dst_dir).unwrap();
        let dst_a = dst_dir.join("a.png");
        let dst_b = root.join("missing_dir").join("b.png"); // parent does not exist

        let moves = vec![
            AssetMove {
                source: src_a.clone(),
                destination: dst_a.clone(),
            },
            AssetMove {
                source: src_b.clone(),
                destination: dst_b.clone(),
            },
        ];

        let err = move_assets(&moves).expect_err("second move must fail");
        assert!(matches!(err, WriteError::Io(_)));

        // The first move must have been rolled back: a.png back at its source,
        // and not left at the destination. b.png never moved.
        assert!(src_a.exists(), "a.png should be rolled back to its source");
        assert!(
            !dst_a.exists(),
            "a.png should not remain at the destination"
        );
        assert!(src_b.exists(), "b.png should still be at its source");
    }
}
