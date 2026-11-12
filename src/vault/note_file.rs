//! Bounded reads of one file in a Vault (#636).
//!
//! A Vault's files arrive from Git pulls and sync tools, so a path the index
//! named a note, or `.obsidian/app.json`, may by now be a FIFO, a device, a
//! symbolic link or a file of any size. A plain `open` of a FIFO blocks until
//! a writer appears, and a plain read of `/dev/zero` never ends. Every read
//! here refuses anything but a regular file without blocking, and refuses a
//! file over the caller's bound before reading it.

use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;

/// The largest note an exact read, a download or a text match reads: the
/// largest reply an agent can receive over MCP (`MAX_JSONRPC_RESPONSE_BYTES`).
/// A note this size already cannot reach an agent whole.
pub const MAX_NOTE_BYTES: u64 = 8 * 1024 * 1024;

/// Why [`read_bounded`] read nothing.
#[derive(Debug)]
pub enum BoundedReadError {
    /// The path is a FIFO, a device, a directory, a socket or a symbolic
    /// link. The index follows no link, so a link here arrived after it.
    NotRegular,
    /// The file holds more than `limit` bytes.
    TooLarge { limit: u64 },
    /// The note's bytes are not UTF-8, so it has no text to return (#637).
    /// Only [`read_note_text`] reports this; [`read_bounded`] returns bytes.
    NotUtf8,
    /// Opening or reading failed; `NotFound` when nothing is there.
    Io(io::Error),
}

impl fmt::Display for BoundedReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotRegular => formatter.write_str("not a regular file"),
            Self::TooLarge { limit } => write!(formatter, "larger than {limit} bytes"),
            Self::NotUtf8 => formatter.write_str("not valid UTF-8"),
            Self::Io(error) => error.fmt(formatter),
        }
    }
}

impl From<io::Error> for BoundedReadError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// The bytes of the regular file at `path`, at most `limit` of them.
pub fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, BoundedReadError> {
    // Checked before opening, so a FIFO or device is never opened at all.
    if !fs::symlink_metadata(path)?.file_type().is_file() {
        return Err(BoundedReadError::NotRegular);
    }
    // Checked again on the open handle: the path may have changed between.
    read_open_bounded(open_without_blocking(path)?, limit)
}

/// The bytes of a file the caller already opened, at most `limit` of them.
/// For a caller that opens through its own path checks, such as the write
/// layer's walk that follows no symbolic link in any component.
pub fn read_open_bounded(file: File, limit: u64) -> Result<Vec<u8>, BoundedReadError> {
    let metadata = file.metadata()?;
    if !metadata.file_type().is_file() {
        return Err(BoundedReadError::NotRegular);
    }
    if metadata.len() > limit {
        return Err(BoundedReadError::TooLarge { limit });
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    // A file that grows while it is read stops at the bound, one byte over.
    file.take(limit.saturating_add(1)).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(BoundedReadError::TooLarge { limit });
    }
    Ok(bytes)
}

/// The note at `path` as text, bounded by [`MAX_NOTE_BYTES`]. A file that is
/// not UTF-8 fails as [`BoundedReadError::NotUtf8`], which every exact read
/// reports as the one non-retryable `note_unreadable` (#637).
pub fn read_note_text(path: &Path) -> Result<String, BoundedReadError> {
    String::from_utf8(read_bounded(path, MAX_NOTE_BYTES)?).map_err(|_| BoundedReadError::NotUtf8)
}

/// The Vault-relative file name of the note whose extension-free relative
/// path is `relative_path`, for a caller that holds only that path, such as a
/// published snapshot row. A note's extension is `.md` in any case (#637), so
/// when no `.md` file is there, the folder is searched for the one other case
/// the catalogue lists; with none, the `.md` name comes back, and reading it
/// reports the note missing.
pub(crate) fn note_file_name(root: &Path, relative_path: &str) -> String {
    let lower = format!("{relative_path}.md");
    if fs::symlink_metadata(root.join(&lower)).is_ok() {
        return lower;
    }
    let (folder, stem) = match relative_path.rsplit_once('/') {
        Some((folder, stem)) => (Some(folder), stem),
        None => (None, relative_path),
    };
    let Ok(entries) = fs::read_dir(folder.map_or_else(|| root.to_path_buf(), |f| root.join(f)))
    else {
        return lower;
    };
    let mut names: Vec<String> = entries
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| {
            name.len() == stem.len() + 3
                && name.starts_with(stem)
                && super::paths::is_note_path(Path::new(name))
        })
        .collect();
    // The catalogue's own choice among case twins: the first in path order.
    names.sort();
    match (names.into_iter().next(), folder) {
        (Some(name), Some(folder)) => format!("{folder}/{name}"),
        (Some(name), None) => name,
        (None, _) => lower,
    }
}

/// Opens `path` for reading without following a final symbolic link and
/// without waiting for a FIFO's writer. A final symbolic link is
/// [`BoundedReadError::NotRegular`]; the caller still checks the handle's
/// type before it reads.
#[cfg(unix)]
pub(crate) fn open_without_blocking(path: &Path) -> Result<File, BoundedReadError> {
    use std::os::unix::fs::OpenOptionsExt;

    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(path)
        .map_err(|error| match error.raw_os_error() {
            Some(libc::ELOOP) => BoundedReadError::NotRegular,
            _ => BoundedReadError::Io(error),
        })
}

#[cfg(not(unix))]
pub(crate) fn open_without_blocking(path: &Path) -> Result<File, BoundedReadError> {
    Ok(File::open(path)?)
}

/// Grows the file at `path` one byte past [`MAX_NOTE_BYTES`] without writing
/// it: the file is sparse, so a test of the bound costs no disk.
#[cfg(test)]
pub(crate) fn grow_past_note_bound(path: &Path) {
    fs::File::options()
        .write(true)
        .open(path)
        .expect("open")
        .set_len(MAX_NOTE_BYTES + 1)
        .expect("grow the note past the bound");
}

/// Makes a FIFO at `path`, for tests of the readers that must not block on one.
#[cfg(all(test, unix))]
pub(crate) fn make_fifo(path: &Path) {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let name = CString::new(path.as_os_str().as_bytes()).expect("fifo path");
    // SAFETY: `name` is a valid NUL-terminated path that outlives the call.
    let result = unsafe { libc::mkfifo(name.as_ptr(), 0o600) };
    assert_eq!(result, 0, "mkfifo: {}", io::Error::last_os_error());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_regular_file_within_the_bound_is_read_whole() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("Note.md");
        fs::write(&path, "twelve bytes").expect("write");
        assert_eq!(read_bounded(&path, 12).expect("read"), b"twelve bytes");
        assert!(matches!(
            read_bounded(&path, 11),
            Err(BoundedReadError::TooLarge { limit: 11 })
        ));
    }

    #[test]
    fn a_missing_file_is_not_found() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(matches!(
            read_bounded(&dir.path().join("Gone.md"), 10),
            Err(BoundedReadError::Io(error)) if error.kind() == io::ErrorKind::NotFound
        ));
    }

    #[cfg(unix)]
    #[test]
    fn fifos_symlinks_and_directories_are_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let fifo = dir.path().join("Fifo.md");
        make_fifo(&fifo);
        let target = dir.path().join("Target.md");
        fs::write(&target, "text").expect("write");
        let link = dir.path().join("Link.md");
        std::os::unix::fs::symlink(&target, &link).expect("link");
        for path in [fifo, link, dir.path().to_path_buf()] {
            assert!(
                matches!(read_bounded(&path, 1024), Err(BoundedReadError::NotRegular)),
                "{}",
                path.display()
            );
        }
    }
}
