use std::ffi::CString;
use std::fs;
use std::io::Write;
use std::os::fd::{AsRawFd, FromRawFd};
use std::path::{Path, PathBuf};

use crate::cache::parse::content_hash;
use crate::vault::types::NoteEntry;

use super::types::{MutationPhase, TextRewrite, WriteError};

enum Compensation {
    MoveBack {
        phase: MutationPhase,
        source: PathBuf,
        destination: PathBuf,
    },
    RestoreRewrite {
        path: PathBuf,
        original: String,
        applied_hash: String,
    },
}

pub(super) struct MutationJournal {
    vault_root: PathBuf,
    completed: Vec<Compensation>,
}

impl MutationJournal {
    pub(super) fn new(vault_root: &Path) -> Self {
        Self {
            vault_root: vault_root.to_path_buf(),
            completed: Vec::new(),
        }
    }

    pub(super) fn move_file(
        &mut self,
        phase: MutationPhase,
        source: &Path,
        destination: &Path,
    ) -> Result<(), WriteError> {
        let result = move_file_no_follow(source, destination);
        self.retain_completed_move(phase, source, destination, &result);
        result
    }

    pub(super) fn move_note(
        &mut self,
        source: &Path,
        destination: &Path,
        expected_content_hash: &str,
    ) -> Result<(), WriteError> {
        let result = move_file_if_unchanged(source, destination, expected_content_hash);
        self.retain_completed_move(MutationPhase::Note, source, destination, &result);
        result
    }

    fn retain_completed_move(
        &mut self,
        phase: MutationPhase,
        source: &Path,
        destination: &Path,
        result: &Result<(), WriteError>,
    ) {
        // The R01 move primitive normally returns Ok only after removing its
        // source gate. If that final cleanup itself fails, the file has already
        // reached the destination. Retain that completed commit as well so an
        // outer transaction never mistakes it for a mutation-free error.
        let committed_despite_error = result.is_err()
            && is_regular_file_no_follow(destination)
            && !is_regular_file_no_follow(source);
        if result.is_ok() || committed_despite_error {
            self.completed.push(Compensation::MoveBack {
                phase,
                source: source.to_path_buf(),
                destination: destination.to_path_buf(),
            });
        }
    }

    pub(super) fn apply_rewrites(
        &mut self,
        rewrites: Vec<TextRewrite>,
    ) -> Result<Vec<PathBuf>, WriteError> {
        self.apply_rewrites_with_before_commit(rewrites, |_| {})
    }

    fn apply_rewrites_with_before_commit(
        &mut self,
        rewrites: Vec<TextRewrite>,
        mut before_commit: impl FnMut(&Path),
    ) -> Result<Vec<PathBuf>, WriteError> {
        let mut rewritten = Vec::with_capacity(rewrites.len());
        for rewrite in rewrites {
            let original = fs::read_to_string(&rewrite.path).map_err(|error| {
                WriteError::Io(format!(
                    "failed to retain original note '{}' before rewrite: {error}",
                    rewrite.path.display()
                ))
            })?;
            let original_hash = content_hash(&original);
            before_commit(&rewrite.path);
            let result = atomic_write_if_unchanged(&rewrite.path, &rewrite.content, &original_hash);
            let committed_despite_error = result.is_err()
                && fs::read_to_string(&rewrite.path)
                    .is_ok_and(|current| current == rewrite.content);
            if result.is_ok() || committed_despite_error {
                rewritten.push(rewrite.path.clone());
                self.completed.push(Compensation::RestoreRewrite {
                    path: rewrite.path,
                    original,
                    applied_hash: content_hash(&rewrite.content),
                });
            }
            result?;
        }
        Ok(rewritten)
    }

    pub(super) fn rollback(mut self, cause: WriteError) -> WriteError {
        self.rollback_with_observer(cause, |_| {})
    }

    #[cfg(test)]
    fn rollback_observing(
        mut self,
        cause: WriteError,
        observer: impl FnMut(&'static str),
    ) -> WriteError {
        self.rollback_with_observer(cause, observer)
    }

    fn rollback_with_observer(
        &mut self,
        cause: WriteError,
        mut observer: impl FnMut(&'static str),
    ) -> WriteError {
        if self.completed.is_empty() {
            return cause;
        }

        let mut failures = Vec::new();
        while let Some(compensation) = self.completed.pop() {
            let (label, paths, result) = match compensation {
                Compensation::MoveBack {
                    phase,
                    source,
                    destination,
                } => {
                    let label = match phase {
                        MutationPhase::Note => "restore moved note",
                        MutationPhase::Asset => "restore moved asset",
                        MutationPhase::Rewrite => unreachable!("rewrites are not file moves"),
                    };
                    let result = move_file_no_follow(&destination, &source);
                    (label, vec![source, destination], result)
                }
                Compensation::RestoreRewrite {
                    path,
                    original,
                    applied_hash,
                } => {
                    let result = atomic_write_if_unchanged(&path, &original, &applied_hash);
                    ("restore rewritten note", vec![path], result)
                }
            };
            observer(label);
            if let Err(error) = result {
                tracing::error!(
                    compensation = label,
                    error = ?error,
                    "vault mutation compensation failed"
                );
                failures.push((label, paths));
            }
        }

        if failures.is_empty() {
            return annotate_rollback_succeeded(cause);
        }

        tracing::error!(
            cause = ?cause,
            failed_compensations = failures.len(),
            "vault mutation requires manual recovery"
        );

        let details = failures
            .into_iter()
            .map(|(label, paths)| {
                let paths = paths
                    .iter()
                    .map(|path| bounded_vault_path(&self.vault_root, path))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{label} [{paths}]")
            })
            .collect::<Vec<_>>()
            .join("; ");
        WriteError::recovery_required(format!("vault mutation rollback was incomplete: {details}"))
    }
}

fn is_regular_file_no_follow(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_file())
}

fn bounded_vault_path(vault_root: &Path, path: &Path) -> String {
    path.strip_prefix(vault_root)
        .ok()
        .filter(|relative| !relative.as_os_str().is_empty())
        .map(|relative| relative.to_string_lossy().into_owned())
        .or_else(|| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "<vault path>".to_string())
}

fn annotate_rollback_succeeded(error: WriteError) -> WriteError {
    fn annotate(message: String) -> String {
        format!("{message}; rollback succeeded")
    }

    match error {
        WriteError::Conflict(message) => WriteError::Conflict(annotate(message)),
        WriteError::InvalidInput(message) => WriteError::InvalidInput(annotate(message)),
        WriteError::Io(message) => WriteError::Io(annotate(message)),
    }
}

pub(super) fn ensure_content_hash(entry: &NoteEntry, expected: &str) -> Result<(), WriteError> {
    ensure_path_content_hash(&entry.path, &entry.relative_path, expected)
}

fn ensure_path_content_hash(path: &Path, label: &str, expected: &str) -> Result<(), WriteError> {
    let expected = expected.trim();
    if expected.is_empty() {
        return Err(WriteError::InvalidInput(
            "expected_content_hash cannot be empty".to_string(),
        ));
    }
    ensure_safe_destination(path)?;
    let mut content = String::new();
    open_existing_file_no_follow(path)
        .and_then(|mut file| std::io::Read::read_to_string(&mut file, &mut content))
        .map_err(|error| WriteError::Io(format!("failed to read note '{}': {error}", label)))?;
    let actual = content_hash(&content);
    if actual != expected {
        return Err(WriteError::Conflict(format!(
            "note changed since it was read: expected {expected}, found {actual}"
        )));
    }
    Ok(())
}

pub(super) fn atomic_write(path: &Path, content: &str) -> Result<(), WriteError> {
    atomic_write_bytes(path, content.as_bytes())
}

pub(super) fn atomic_write_if_unchanged(
    path: &Path,
    content: &str,
    expected_content_hash: &str,
) -> Result<(), WriteError> {
    atomic_write_bytes_if_unchanged(path, content.as_bytes(), expected_content_hash)
}

pub(super) fn atomic_write_bytes(path: &Path, bytes: &[u8]) -> Result<(), WriteError> {
    atomic_write_inner(path, bytes, None, || {})
}

fn atomic_write_bytes_if_unchanged(
    path: &Path,
    bytes: &[u8],
    expected_content_hash: &str,
) -> Result<(), WriteError> {
    atomic_write_inner(path, bytes, Some(expected_content_hash), || {})
}

#[cfg(test)]
fn atomic_write_if_unchanged_with_before_exchange(
    path: &Path,
    content: &str,
    expected_content_hash: &str,
    before_exchange: impl FnOnce(),
) -> Result<(), WriteError> {
    atomic_write_inner(
        path,
        content.as_bytes(),
        Some(expected_content_hash),
        before_exchange,
    )
}

fn atomic_write_inner(
    path: &Path,
    bytes: &[u8],
    expected_content_hash: Option<&str>,
    before_commit: impl FnOnce(),
) -> Result<(), WriteError> {
    let (parent, filename) = open_parent_dir_no_follow(path)?;
    let (tmp_name, mut file) = create_unique_temporary_file(&parent, &filename)?;

    // Write and fsync the temp file so its bytes are durable BEFORE the rename.
    // Without the fsync, a crash just after the rename can leave the note file's
    // name pointing at data the OS never flushed (an empty or truncated file).
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| {
            let _ = unlink_at(&parent, &tmp_name);
            WriteError::Io(format!(
                "failed to write temporary file for '{}': {error}",
                path.display()
            ))
        })?;
    drop(file);
    before_commit();

    if let Some(expected) = expected_content_hash {
        // Exchange is the commit point: the displaced destination is held at
        // our private name, where we verify its identity. A concurrent atomic
        // save therefore becomes detectable and is swapped back, rather than
        // being silently overwritten in a check-then-rename gap.
        rename_exchange(&parent, &tmp_name, &parent, &filename)?;
        let prior = read_file_at_no_follow(&parent, &tmp_name).map_err(|error| {
            let _ = rename_exchange(&parent, &tmp_name, &parent, &filename);
            WriteError::Io(format!(
                "failed to inspect replaced note '{}': {error}",
                path.display()
            ))
        })?;
        if content_hash(&prior) != expected.trim() {
            rename_exchange(&parent, &tmp_name, &parent, &filename).map_err(|error| {
                WriteError::Io(format!(
                    "failed to restore concurrently changed note '{}': {error}",
                    path.display()
                ))
            })?;
            let _ = unlink_at(&parent, &tmp_name);
            return Err(WriteError::Conflict(format!(
                "note changed since it was read: expected {}, found {}",
                expected.trim(),
                content_hash(&prior)
            )));
        }
        unlink_at(&parent, &tmp_name)?;
    } else {
        ensure_safe_destination_at(&parent, &filename, path)?;
        rename_at(&parent, &tmp_name, &parent, &filename)?;
    }

    // fsync the parent directory so the rename itself (a directory metadata
    // change) survives a crash — otherwise the durable temp data can still be
    // lost if the directory entry update was not flushed.
    let _ = parent.sync_all();
    Ok(())
}

fn ensure_safe_destination(path: &Path) -> Result<(), WriteError> {
    let (parent, filename) = open_parent_dir_no_follow(path)?;
    ensure_safe_destination_at(&parent, &filename, path)
}

fn ensure_safe_destination_at(
    parent: &fs::File,
    filename: &CString,
    path: &Path,
) -> Result<(), WriteError> {
    match metadata_at_no_follow(parent, filename) {
        Ok(metadata) if metadata.st_mode & libc::S_IFMT == libc::S_IFLNK => {
            Err(WriteError::Conflict(format!(
                "refusing to replace symlink destination '{}'",
                path.display()
            )))
        }
        Ok(metadata) if metadata.st_mode & libc::S_IFMT != libc::S_IFREG => {
            Err(WriteError::Conflict(format!(
                "refusing to replace non-file destination '{}'",
                path.display()
            )))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(WriteError::Io(format!(
            "failed to inspect write destination '{}': {error}",
            path.display()
        ))),
    }
}

fn open_existing_file_no_follow(path: &Path) -> Result<fs::File, std::io::Error> {
    let (parent, filename) = open_parent_dir_no_follow_io(path)?;
    open_file_at_no_follow(&parent, &filename)
}

fn create_unique_temporary_file(
    parent: &fs::File,
    filename: &CString,
) -> Result<(CString, fs::File), WriteError> {
    let filename = filename.to_str().map_err(|_| {
        WriteError::InvalidInput("write destination filename must be UTF-8".to_string())
    })?;

    for _ in 0..32 {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).map_err(|error| {
            WriteError::Io(format!(
                "failed to generate temporary filename entropy: {error}"
            ))
        })?;
        let suffix: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let tmp = CString::new(format!(".{filename}.hatchdoor-tmp-{suffix}"))
            .expect("generated name has no NUL");
        match create_file_at_no_follow(parent, &tmp) {
            Ok(file) => return Ok((tmp, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(WriteError::Io(format!(
                    "failed to create a temporary file beside '{filename}': {error}"
                )));
            }
        }
    }

    Err(WriteError::Io(
        "failed to reserve a unique temporary file after 32 attempts".to_string(),
    ))
}

pub(super) fn move_file_if_unchanged(
    source: &Path,
    destination: &Path,
    expected_content_hash: &str,
) -> Result<(), WriteError> {
    move_file_if_unchanged_inner(source, destination, expected_content_hash, || {})
}

#[cfg(test)]
fn move_file_if_unchanged_with_after_exchange(
    source: &Path,
    destination: &Path,
    expected_content_hash: &str,
    after_exchange: impl FnOnce(),
) -> Result<(), WriteError> {
    move_file_if_unchanged_inner(source, destination, expected_content_hash, after_exchange)
}

fn move_file_if_unchanged_inner(
    source: &Path,
    destination: &Path,
    expected_content_hash: &str,
    after_exchange: impl FnOnce(),
) -> Result<(), WriteError> {
    let (source_parent, source_name) = open_parent_dir_no_follow(source)?;
    let (destination_parent, destination_name) = open_parent_dir_no_follow(destination)?;
    create_move_gate(&destination_parent, &destination_name).map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            WriteError::Conflict(format!(
                "destination already exists: {}",
                destination.display()
            ))
        } else {
            WriteError::Io(format!(
                "failed to reserve destination '{}': {error}",
                destination.display()
            ))
        }
    })?;
    if let Err(error) = rename_exchange(
        &source_parent,
        &source_name,
        &destination_parent,
        &destination_name,
    ) {
        let _ = remove_move_gate(&destination_parent, &destination_name);
        return Err(WriteError::Io(format!(
            "failed to atomically move '{}' to '{}': {error}",
            source.display(),
            destination.display()
        )));
    }
    after_exchange();
    let moved = read_regular_file_at(&destination_parent, &destination_name);
    let matches = moved
        .as_ref()
        .is_ok_and(|content| content_hash(content) == expected_content_hash.trim());
    if !matches {
        restore_move_from_gate(
            &source_parent,
            &source_name,
            &destination_parent,
            &destination_name,
            source,
        )?;
        return match moved {
            Ok(prior) => Err(WriteError::Conflict(format!(
                "note changed since it was read: expected {}, found {}",
                expected_content_hash.trim(),
                content_hash(&prior)
            ))),
            Err(error) => Err(WriteError::Conflict(format!(
                "refusing to move unsafe source '{}': {error}",
                source.display()
            ))),
        };
    }
    remove_move_gate(&source_parent, &source_name).map_err(|error| {
        WriteError::Io(format!(
            "failed to finalize move from '{}': {error}",
            source.display()
        ))
    })?;
    let _ = source_parent.sync_all();
    let _ = destination_parent.sync_all();
    Ok(())
}

pub(super) fn move_file_no_follow(source: &Path, destination: &Path) -> Result<(), WriteError> {
    let (source_parent, source_name) = open_parent_dir_no_follow(source)?;
    let (destination_parent, destination_name) = open_parent_dir_no_follow(destination)?;
    create_move_gate(&destination_parent, &destination_name).map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            WriteError::Conflict(format!(
                "destination already exists: {}",
                destination.display()
            ))
        } else {
            WriteError::Io(format!(
                "failed to reserve destination '{}': {error}",
                destination.display()
            ))
        }
    })?;
    if let Err(error) = rename_exchange(
        &source_parent,
        &source_name,
        &destination_parent,
        &destination_name,
    ) {
        let _ = remove_move_gate(&destination_parent, &destination_name);
        return Err(WriteError::Io(format!(
            "failed to atomically move '{}' to '{}': {error}",
            source.display(),
            destination.display()
        )));
    }
    if let Err(error) = assert_regular_file_at(&destination_parent, &destination_name) {
        restore_move_from_gate(
            &source_parent,
            &source_name,
            &destination_parent,
            &destination_name,
            source,
        )?;
        return Err(WriteError::Conflict(format!(
            "refusing to move unsafe source '{}': {error}",
            source.display()
        )));
    }
    remove_move_gate(&source_parent, &source_name).map_err(|error| {
        WriteError::Io(format!(
            "failed to finalize move from '{}': {error}",
            source.display()
        ))
    })?;
    let _ = source_parent.sync_all();
    let _ = destination_parent.sync_all();
    Ok(())
}

fn create_move_gate(parent: &fs::File, name: &CString) -> Result<(), std::io::Error> {
    let result = unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

fn remove_move_gate(parent: &fs::File, name: &CString) -> Result<(), std::io::Error> {
    let result = unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

fn restore_move_from_gate(
    source_parent: &fs::File,
    source_name: &CString,
    destination_parent: &fs::File,
    destination_name: &CString,
    source: &Path,
) -> Result<(), WriteError> {
    rename_exchange(
        source_parent,
        source_name,
        destination_parent,
        destination_name,
    )
    .map_err(|error| {
        WriteError::Io(format!(
            "failed to restore unsafe move from '{}': {error}",
            source.display()
        ))
    })?;
    remove_move_gate(destination_parent, destination_name).map_err(|error| {
        WriteError::Io(format!(
            "failed to remove move gate for '{}': {error}",
            source.display()
        ))
    })
}

fn open_parent_dir_no_follow(path: &Path) -> Result<(fs::File, CString), WriteError> {
    open_parent_dir_no_follow_io(path).map_err(|error| {
        WriteError::Io(format!(
            "failed to open verified parent for '{}': {error}",
            path.display()
        ))
    })
}

fn open_parent_dir_no_follow_io(path: &Path) -> Result<(fs::File, CString), std::io::Error> {
    let filename = path.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "write destination has no filename",
        )
    })?;
    let filename = CString::new(filename.as_encoded_bytes()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "write destination filename has NUL",
        )
    })?;
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "write destination has no parent",
        )
    })?;
    if !parent.is_absolute() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "write destination must be absolute",
        ));
    }

    let root = CString::new("/").expect("literal has no NUL");
    let mut current = open_dir_at(libc::AT_FDCWD, &root)?;
    for component in parent.components() {
        use std::path::Component;
        match component {
            Component::RootDir => {}
            Component::Normal(part) => {
                let part = CString::new(part.as_encoded_bytes()).map_err(|_| {
                    std::io::Error::new(std::io::ErrorKind::InvalidInput, "path component has NUL")
                })?;
                current = open_dir_at(current.as_raw_fd(), &part)?;
            }
            _ => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "write path has unsupported component",
                ));
            }
        }
    }
    Ok((current, filename))
}

fn open_dir_at(dirfd: libc::c_int, name: &CString) -> Result<fs::File, std::io::Error> {
    let fd = unsafe {
        libc::openat(
            dirfd,
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    file_from_fd(fd)
}

fn create_file_at_no_follow(parent: &fs::File, name: &CString) -> Result<fs::File, std::io::Error> {
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            0o600,
        )
    };
    file_from_fd(fd)
}

fn open_file_at_no_follow(parent: &fs::File, name: &CString) -> Result<fs::File, std::io::Error> {
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    file_from_fd(fd)
}

fn file_from_fd(fd: libc::c_int) -> Result<fs::File, std::io::Error> {
    if fd < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(unsafe { fs::File::from_raw_fd(fd) })
    }
}

fn metadata_at_no_follow(parent: &fs::File, name: &CString) -> Result<libc::stat, std::io::Error> {
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    let result = unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            name.as_ptr(),
            stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if result == 0 {
        Ok(unsafe { stat.assume_init() })
    } else {
        Err(std::io::Error::last_os_error())
    }
}

fn read_file_at_no_follow(parent: &fs::File, name: &CString) -> Result<String, std::io::Error> {
    let mut content = String::new();
    std::io::Read::read_to_string(&mut open_file_at_no_follow(parent, name)?, &mut content)?;
    Ok(content)
}

/// Assert the entry is an ordinary file: not a symlink, directory, device
/// node, or FIFO. This is the whole of the safety property the move guard
/// needs, and it says nothing about the bytes inside.
fn assert_regular_file_at(parent: &fs::File, name: &CString) -> Result<(), std::io::Error> {
    let metadata = metadata_at_no_follow(parent, name)?;
    if metadata.st_mode & libc::S_IFMT != libc::S_IFREG {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "source is not a regular file",
        ));
    }
    Ok(())
}

/// The regular-file assertion plus a text read, for the paths that go on to
/// hash the content as Markdown. Anything that only needs the file to be a
/// file must call [`assert_regular_file_at`] instead: an attachment is
/// arbitrary bytes and need not decode as UTF-8 (#220).
fn read_regular_file_at(parent: &fs::File, name: &CString) -> Result<String, std::io::Error> {
    assert_regular_file_at(parent, name)?;
    read_file_at_no_follow(parent, name)
}

fn unlink_at(parent: &fs::File, name: &CString) -> Result<(), std::io::Error> {
    let result = unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), 0) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

fn rename_at(
    from_parent: &fs::File,
    from: &CString,
    to_parent: &fs::File,
    to: &CString,
) -> Result<(), WriteError> {
    let result = unsafe {
        libc::renameat(
            from_parent.as_raw_fd(),
            from.as_ptr(),
            to_parent.as_raw_fd(),
            to.as_ptr(),
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(WriteError::Io(format!(
            "failed to commit descriptor-relative rename: {}",
            std::io::Error::last_os_error()
        )))
    }
}

fn rename_exchange(
    from_parent: &fs::File,
    from: &CString,
    to_parent: &fs::File,
    to: &CString,
) -> Result<(), std::io::Error> {
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            from_parent.as_raw_fd(),
            from.as_ptr(),
            to_parent.as_raw_fd(),
            to.as_ptr(),
            libc::RENAME_EXCHANGE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::write::tests::BINARY_ASSET;
    use crate::vault::write::types::AssetMove;
    use tempfile::tempdir;

    #[cfg(unix)]
    fn make_fifo(path: &Path) {
        use std::os::unix::ffi::OsStrExt;

        let name = CString::new(path.as_os_str().as_bytes()).expect("fifo path");
        let result = unsafe { libc::mkfifo(name.as_ptr(), 0o600) };
        assert_eq!(result, 0, "mkfifo: {}", std::io::Error::last_os_error());
    }

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

    #[cfg(unix)]
    #[test]
    fn atomic_write_avoids_a_planted_temporary_sidecar_without_touching_its_target() {
        use std::os::unix::fs::symlink;

        let dir = tempdir().expect("tempdir");
        let note = dir.path().join("Note.md");
        let sentinel = dir.path().join("outside.txt");
        fs::write(&sentinel, "do not change").expect("sentinel");
        symlink(&sentinel, note.with_extension("md.hatchdoor-tmp")).expect("sidecar link");

        atomic_write(&note, "# Note\nupdated\n")
            .expect("a random exclusive temporary file must avoid the planted sidecar");

        assert_eq!(fs::read_to_string(&sentinel).unwrap(), "do not change");
        assert_eq!(fs::read_to_string(&note).unwrap(), "# Note\nupdated\n");
        assert!(
            note.with_extension("md.hatchdoor-tmp").is_symlink(),
            "the planted sidecar itself must remain untouched"
        );
    }

    #[test]
    fn conditional_atomic_write_preserves_a_manual_edit_made_before_commit() {
        let dir = tempdir().expect("tempdir");
        let note = dir.path().join("Note.md");
        fs::write(&note, "original").expect("original note");
        let expected = content_hash("original");

        let error =
            atomic_write_if_unchanged_with_before_exchange(&note, "agent edit", &expected, || {
                fs::write(&note, "manual edit").expect("manual edit")
            })
            .expect_err("a final-window manual edit must be restored");

        assert!(matches!(error, WriteError::Conflict(_)));
        assert_eq!(fs::read_to_string(&note).unwrap(), "manual edit");
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_keeps_the_commit_in_the_opened_parent_after_a_path_swap() {
        use std::os::unix::fs::symlink;

        let dir = tempdir().expect("tempdir");
        let vault = dir.path().join("vault");
        let notes = vault.join("Notes");
        let external = dir.path().join("external");
        fs::create_dir_all(&notes).expect("notes");
        fs::create_dir(&external).expect("external");
        let note = notes.join("Note.md");
        let original_parent = vault.join("Notes-original");

        atomic_write_inner(&note, b"safe\n", None, || {
            fs::rename(&notes, &original_parent).expect("swap away opened parent");
            symlink(&external, &notes).expect("replace path with external symlink");
        })
        .expect("descriptor-relative commit");

        assert_eq!(
            fs::read_to_string(original_parent.join("Note.md")).unwrap(),
            "safe\n"
        );
        assert!(
            !external.join("Note.md").exists(),
            "the substituted external directory must never receive the write"
        );
    }

    #[test]
    fn conditional_move_keeps_a_manual_save_after_the_exchange() {
        let dir = tempdir().expect("tempdir");
        let source = dir.path().join("Source.md");
        let destination = dir.path().join("Moved.md");
        fs::write(&source, "original").expect("source");
        let expected = content_hash("original");

        // The directory gate installed at Source.md makes this simulated atomic
        // save fail until the move has released the name, so it cannot be
        // overwritten by cleanup.
        let result =
            move_file_if_unchanged_with_after_exchange(&source, &destination, &expected, || {
                assert!(
                    fs::write(&source, "manual edit").is_err(),
                    "the source gate must reject a save until cleanup finishes"
                );
            });

        assert!(result.is_ok());
        assert_eq!(fs::read_to_string(&destination).unwrap(), "original");
        fs::write(&source, "manual edit").expect("manual save after gate release");
        assert_eq!(fs::read_to_string(&source).unwrap(), "manual edit");
    }

    #[cfg(unix)]
    #[test]
    fn move_file_no_follow_rejects_a_symlink_source_and_restores_it() {
        use std::os::unix::fs::symlink;

        let dir = tempdir().expect("tempdir");
        let sentinel = dir.path().join("outside.txt");
        let source = dir.path().join("asset.png");
        let destination = dir.path().join("moved.png");
        fs::write(&sentinel, "sentinel").expect("sentinel");
        symlink(&sentinel, &source).expect("source link");

        let error = move_file_no_follow(&source, &destination)
            .expect_err("symlink sources must not be moved");

        assert!(matches!(error, WriteError::Conflict(_)));
        assert!(source.is_symlink());
        assert!(!destination.exists());
        assert_eq!(fs::read_to_string(&sentinel).unwrap(), "sentinel");
    }

    #[test]
    fn move_file_no_follow_moves_bytes_that_are_not_valid_utf8() {
        // issue #220: an attachment is arbitrary bytes. The post-move guard
        // exists to prove the moved thing is a regular file, not that it
        // decodes as text, so a real image or PDF must move unchanged.
        let dir = tempdir().expect("tempdir");
        let source = dir.path().join("photo.jpg");
        let destination = dir.path().join("moved.jpg");
        fs::write(&source, BINARY_ASSET).expect("source");

        move_file_no_follow(&source, &destination).expect("a binary asset must move");

        assert!(!source.exists());
        assert_eq!(fs::read(&destination).unwrap(), BINARY_ASSET);
    }

    #[cfg(unix)]
    #[test]
    fn move_file_no_follow_rejects_a_fifo_source_and_restores_it() {
        use std::os::unix::fs::FileTypeExt;

        let dir = tempdir().expect("tempdir");
        let source = dir.path().join("asset.png");
        let destination = dir.path().join("moved.png");
        make_fifo(&source);

        let error =
            move_file_no_follow(&source, &destination).expect_err("a FIFO must not be moved");

        assert!(matches!(error, WriteError::Conflict(_)));
        assert!(
            fs::symlink_metadata(&source)
                .expect("fifo metadata")
                .file_type()
                .is_fifo(),
            "the refused source must be restored as it was"
        );
        assert!(!destination.exists());
    }

    #[test]
    fn mutation_journal_restores_binary_asset_bytes_when_a_later_move_fails() {
        // issue #220: rollback replays the same move primitive, so the bytes
        // of an asset already moved by an earlier step must survive the round
        // trip when a later step of the same mutation fails.
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        let binary = root.join("photo.jpg");
        let sibling = root.join("notes.txt");
        fs::write(&binary, BINARY_ASSET).expect("binary asset");
        fs::write(&sibling, "text asset").expect("text asset");
        let dst_dir = root.join("dst");
        fs::create_dir(&dst_dir).expect("dst dir");

        let mut journal = MutationJournal::new(root);
        journal
            .move_file(MutationPhase::Asset, &binary, &dst_dir.join("photo.jpg"))
            .expect("binary asset move");
        // The second asset's destination parent does not exist, so its move
        // fails deterministically after the first has committed.
        let cause = journal
            .move_file(
                MutationPhase::Asset,
                &sibling,
                &root.join("missing_dir").join("notes.txt"),
            )
            .expect_err("the later move must fail");
        let error = journal.rollback(cause);

        assert!(matches!(error, WriteError::Io(_)));
        assert_eq!(
            fs::read(&binary).expect("restored asset"),
            BINARY_ASSET,
            "rollback must restore the original bytes"
        );
        assert!(!dst_dir.join("photo.jpg").exists());
        assert_eq!(fs::read_to_string(&sibling).unwrap(), "text asset");
    }

    #[test]
    fn mutation_journal_rolls_back_already_moved_assets_on_failure() {
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

        let moves = [
            AssetMove {
                source: src_a.clone(),
                destination: dst_a.clone(),
            },
            AssetMove {
                source: src_b.clone(),
                destination: dst_b.clone(),
            },
        ];

        let mut journal = MutationJournal::new(root);
        journal
            .move_file(
                MutationPhase::Asset,
                &moves[0].source,
                &moves[0].destination,
            )
            .expect("first move");
        let cause = journal
            .move_file(
                MutationPhase::Asset,
                &moves[1].source,
                &moves[1].destination,
            )
            .expect_err("second move must fail");
        let err = journal.rollback(cause);
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

    #[test]
    fn mutation_journal_restores_prior_rewrites_when_a_later_rewrite_fails() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        let first = root.join("First.md");
        fs::write(&first, "original first\n").expect("first");
        let missing = root.join("missing").join("Second.md");
        let rewrites = vec![
            TextRewrite {
                path: first.clone(),
                content: "rewritten first\n".to_string(),
            },
            TextRewrite {
                path: missing,
                content: "rewritten second\n".to_string(),
            },
        ];
        let mut journal = MutationJournal::new(root);

        let cause = journal
            .apply_rewrites(rewrites)
            .expect_err("the missing second rewrite must fail");
        let error = journal.rollback(cause);

        let WriteError::Io(message) = error else {
            panic!("expected I/O failure");
        };
        assert!(message.contains("rollback succeeded"));
        assert_eq!(
            fs::read_to_string(first).expect("restored first"),
            "original first\n"
        );
    }

    #[test]
    fn mutation_journal_compensates_rewrite_asset_and_note_in_reverse_order() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        let note_source = root.join("Note.md");
        let note_destination = root.join("Moved.md");
        let asset_source = root.join("asset.png");
        let asset_destination = root.join("moved.png");
        let backlink = root.join("Backlink.md");
        fs::write(&note_source, "note").expect("note");
        fs::write(&asset_source, "asset").expect("asset");
        fs::write(&backlink, "original backlink").expect("backlink");
        let mut journal = MutationJournal::new(root);
        journal
            .move_note(&note_source, &note_destination, &content_hash("note"))
            .expect("move note");
        journal
            .move_file(MutationPhase::Asset, &asset_source, &asset_destination)
            .expect("move asset");
        journal
            .apply_rewrites(vec![TextRewrite {
                path: backlink.clone(),
                content: "rewritten backlink".to_string(),
            }])
            .expect("rewrite");
        let mut compensated = Vec::new();

        let error = journal.rollback_observing(
            WriteError::Io("injected after rewrite".to_string()),
            |label| compensated.push(label),
        );

        let WriteError::Io(message) = error else {
            panic!("expected injected failure");
        };
        assert!(message.contains("rollback succeeded"));
        assert_eq!(
            compensated,
            vec![
                "restore rewritten note",
                "restore moved asset",
                "restore moved note"
            ]
        );
        assert_eq!(fs::read_to_string(note_source).unwrap(), "note");
        assert_eq!(fs::read_to_string(asset_source).unwrap(), "asset");
        assert_eq!(fs::read_to_string(backlink).unwrap(), "original backlink");
        assert!(!note_destination.exists());
        assert!(!asset_destination.exists());
    }

    #[test]
    fn mutation_journal_preserves_a_manual_edit_before_forward_rewrite_commit() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        let note_source = root.join("Note.md");
        let note_destination = root.join("Moved.md");
        let backlink = root.join("Backlink.md");
        fs::write(&note_source, "note").expect("note");
        fs::write(&backlink, "original backlink").expect("backlink");
        let mut journal = MutationJournal::new(root);
        journal
            .move_note(&note_source, &note_destination, &content_hash("note"))
            .expect("move note");

        let cause = journal
            .apply_rewrites_with_before_commit(
                vec![TextRewrite {
                    path: backlink.clone(),
                    content: "rewritten backlink".to_string(),
                }],
                |path| fs::write(path, "manual edit").expect("manual edit"),
            )
            .expect_err("the retained original must be bound to rewrite commit");
        let error = journal.rollback(cause);

        let WriteError::Conflict(message) = error else {
            panic!("expected rewrite conflict");
        };
        assert!(message.contains("rollback succeeded"));
        assert_eq!(fs::read_to_string(note_source).unwrap(), "note");
        assert_eq!(fs::read_to_string(backlink).unwrap(), "manual edit");
        assert!(!note_destination.exists());
    }
}
