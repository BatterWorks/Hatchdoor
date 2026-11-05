//! The durable record of a multi-note write in progress, and the repair a
//! Vault gets when it is activated after a process died part-way through one
//! (#631, ADR-51).
//!
//! A rename, move, archive or delete of a note, an attachment move, and a
//! Vault-wide tag rename or delete each change many files one at a time. The
//! [`MutationJournal`](super::fs_ops::MutationJournal) undoes the steps it
//! completed when a later one fails, but that only holds while the process
//! lives. So before the first step the journal writes the whole plan here, in
//! the Vault's own file under Hatchdoor's state folder, and removes it once
//! the operation has finished or been undone. A record still present when a
//! Vault is activated is one a dead process left, and
//! [`repair_interrupted_writes`] finishes it.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::cache::parse::content_hash;

use super::fs_ops::{
    atomic_write_if_unchanged_keeping_modified, captured_modified_time, move_file_no_follow,
};
use super::types::{TextRewrite, WriteError};

/// The record format this build writes and reads. A record from a newer
/// build is left alone, never acted on or removed.
const INTENT_SCHEMA_VERSION: u32 = 1;

/// The folder, beside the Vault registry, that holds one record per Vault.
const INTENT_DIRECTORY: &str = "write-intents";

/// Where one Vault keeps the record of its multi-note write in progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntentLog {
    path: PathBuf,
}

impl IntentLog {
    /// The record for the Vault `vault_id`, in `state_directory`, the folder
    /// the Vault registry lives in.
    pub fn for_vault(state_directory: &Path, vault_id: impl std::fmt::Display) -> Self {
        Self {
            path: state_directory
                .join(INTENT_DIRECTORY)
                .join(format!("{vault_id}.json")),
        }
    }

    #[cfg(test)]
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Where the repair puts the temporary files it takes out of the Vault:
    /// `<vault-id>.leftovers/` beside the record.
    fn leftovers(&self) -> PathBuf {
        self.path.with_extension("leftovers")
    }

    /// Write the plan durably. A record already there is never replaced:
    /// the mutation lock admits one multi-note write per Vault at a time, so
    /// one still present is a plan the repair kept to retry, or could not
    /// read, or one whose removal failed. Replacing it would lose that plan,
    /// or let it be replayed over this write later, so the write is refused.
    pub(super) fn record(&self, record: &IntentRecord) -> Result<(), WriteError> {
        let refused = |error: std::io::Error| {
            WriteError::Io(format!(
                "could not record this change before starting it, so nothing was changed: {error}"
            ))
        };
        if fs::symlink_metadata(&self.path).is_ok() {
            return Err(refused(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "an earlier interrupted change in this Vault is still waiting to be finished; \
                 restart Hatchdoor to finish it, and see its log if that does not",
            )));
        }
        let directory = self
            .path
            .parent()
            .expect("an intent record sits in a folder");
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(directory)
            .map_err(refused)?;
        let bytes = serde_json::to_vec(record).map_err(|error| refused(error.into()))?;
        let staged = directory.join(format!(
            ".{}.staged",
            self.path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
        ));
        let mut file = fs::File::create(&staged).map_err(refused)?;
        let written = file
            .write_all(&bytes)
            .and_then(|()| file.sync_all())
            .and_then(|()| fs::rename(&staged, &self.path))
            .and_then(|()| fs::File::open(directory)?.sync_all());
        if let Err(error) = written {
            let _ = fs::remove_file(&staged);
            return Err(refused(error));
        }
        Ok(())
    }

    /// Remove the record once the operation it describes has finished or
    /// been undone. A record that cannot be removed is logged: the next
    /// activation finds every step done, or undone, and only removes it.
    pub(super) fn clear(&self) {
        match fs::remove_file(&self.path) {
            Ok(()) => {
                if let Some(directory) = self.path.parent()
                    && let Ok(directory) = fs::File::open(directory)
                {
                    let _ = directory.sync_all();
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => tracing::error!(
                record = %self.path.display(),
                error = %error,
                "could not remove the record of a finished Vault write"
            ),
        }
    }
}

/// One multi-note write, planned in full: what moves, then what is rewritten.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct IntentRecord {
    schema_version: u32,
    /// What the operation was, for the log: `move note`, `rename tag`, ...
    operation: String,
    /// In the order they run.
    moves: Vec<RecordedMove>,
    /// In the order they run, after every move.
    rewrites: Vec<RecordedRewrite>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct RecordedMove {
    /// Vault-relative, in [`encode_path`]'s form.
    source: String,
    destination: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct RecordedRewrite {
    /// Vault-relative, in [`encode_path`]'s form.
    path: String,
    /// The note's hash when the plan read it: the only text a rewrite may
    /// replace.
    original_hash: String,
    /// The hash of `content`, which says the rewrite already landed.
    applied_hash: String,
    content: String,
}

impl IntentRecord {
    pub(super) fn new(
        vault_root: &Path,
        operation: &str,
        moves: &[(&Path, &Path)],
        rewrites: &[TextRewrite],
    ) -> Result<Self, WriteError> {
        let moves = moves
            .iter()
            .map(|(source, destination)| {
                Ok(RecordedMove {
                    source: encode_path(vault_root, source)?,
                    destination: encode_path(vault_root, destination)?,
                })
            })
            .collect::<Result<_, WriteError>>()?;
        let rewrites = rewrites
            .iter()
            .map(|rewrite| {
                Ok(RecordedRewrite {
                    path: encode_path(vault_root, &rewrite.path)?,
                    original_hash: rewrite.original_hash.clone(),
                    applied_hash: content_hash(&rewrite.content),
                    content: rewrite.content.clone(),
                })
            })
            .collect::<Result<_, WriteError>>()?;
        Ok(Self {
            schema_version: INTENT_SCHEMA_VERSION,
            operation: operation.to_string(),
            moves,
            rewrites,
        })
    }
}

/// A Vault-relative path as a JSON string that gives back the same bytes: a
/// `%` and every byte that is not part of valid UTF-8 are written `%XX`, so a
/// filename that is not UTF-8 survives the round trip.
fn encode_path(vault_root: &Path, path: &Path) -> Result<String, WriteError> {
    let relative = path.strip_prefix(vault_root).map_err(|_| {
        WriteError::Io(format!(
            "could not record '{}': it is outside the Vault",
            path.display()
        ))
    })?;
    let mut encoded = String::new();
    for chunk in relative.as_os_str().as_bytes().utf8_chunks() {
        for character in chunk.valid().chars() {
            if character == '%' {
                encoded.push_str("%25");
            } else {
                encoded.push(character);
            }
        }
        for byte in chunk.invalid() {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    Ok(encoded)
}

/// The path [`encode_path`] wrote, under `vault_root`, or `None` for one that
/// is malformed or would leave the Vault.
fn decode_path(vault_root: &Path, encoded: &str) -> Option<PathBuf> {
    let mut bytes = Vec::with_capacity(encoded.len());
    let mut rest = encoded.as_bytes();
    while let Some((&byte, tail)) = rest.split_first() {
        if byte == b'%' {
            let hex = tail.get(..2)?;
            let hex = std::str::from_utf8(hex).ok()?;
            bytes.push(u8::from_str_radix(hex, 16).ok()?);
            rest = &tail[2..];
        } else {
            bytes.push(byte);
            rest = tail;
        }
    }
    let relative = PathBuf::from(OsString::from_vec(bytes));
    let inside = relative.components().next().is_some()
        && relative
            .components()
            .all(|component| matches!(component, Component::Normal(_)));
    inside.then(|| vault_root.join(relative))
}

/// What one activation's repair did, for its tests. Everything here is also
/// logged as it happens.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct WriteRepair {
    /// The interrupted operation, when a record was found and acted on.
    pub(crate) operation: Option<String>,
    /// Whether the operation had changed anything before it was cut off.
    /// When it had not, nothing is rolled forward and the Vault is left as
    /// it was before the operation began.
    pub(crate) rolled_forward: bool,
    /// Vault-relative paths this repair moved or rewrote.
    pub(crate) completed: Vec<String>,
    /// Vault-relative paths the repair left alone because they no longer
    /// look the way the plan expects. Each one needs a person's attention.
    pub(crate) needs_attention: Vec<String>,
    /// Whether a step failed for a reason that may pass, a folder that
    /// could not be written for example, so the record was kept and the
    /// next start tries again.
    pub(crate) kept_for_retry: bool,
    /// Vault-relative paths of what this repair took out of the Vault: the
    /// empty folders an interrupted move left as its reservation, which are
    /// removed, and leftover temporary files, which are moved to the
    /// leftovers folder beside the record.
    pub(crate) removed_leftovers: Vec<String>,
}

/// One repair at a time, whichever Vault, so two activations of the same
/// Vault (a reconcile that builds and then discards a control block) can
/// never repair it concurrently.
static REPAIR: Mutex<()> = Mutex::new(());

/// Finish the multi-note write a dead process left recorded in `intent_log`,
/// if any, then take every temporary file Hatchdoor's writes leave beside a
/// note out of the Vault, logging each step.
///
/// Called once per Vault per process, when the Vault is first activated and
/// before any write can reach it.
pub(crate) fn repair_interrupted_writes(vault_root: &Path, intent_log: &IntentLog) -> WriteRepair {
    let _serialized = REPAIR
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut repair = WriteRepair::default();
    roll_forward(vault_root, intent_log, &mut repair);
    sweep_temporary_files(vault_root, &intent_log.leftovers(), &mut repair);
    repair
}

/// What is at one path of a planned move.
#[derive(Debug, PartialEq, Eq)]
enum FoundAtPath {
    Missing,
    File,
    /// An empty folder: at a move's destination, the gate that reserves the
    /// name before the exchange; at its source, the gate the exchange swapped
    /// there, which the move removes as its last step.
    EmptyFolder,
    Other,
}

fn found_at(path: &Path) -> FoundAtPath {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => FoundAtPath::Missing,
        Ok(metadata) if metadata.file_type().is_file() => FoundAtPath::File,
        Ok(metadata)
            if metadata.file_type().is_dir()
                && fs::read_dir(path).is_ok_and(|mut entries| entries.next().is_none()) =>
        {
            FoundAtPath::EmptyFolder
        }
        _ => FoundAtPath::Other,
    }
}

fn vault_relative(vault_root: &Path, path: &Path) -> String {
    path.strip_prefix(vault_root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

/// The record `intent_log` holds, or `None` when there is none, or one this
/// build must not act on, which is logged and left in place.
fn read_record(intent_log: &IntentLog) -> Option<IntentRecord> {
    let bytes = match fs::read(&intent_log.path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            tracing::error!(
                record = %intent_log.path.display(),
                error = %error,
                "could not read the record of an interrupted Vault write; it was left in place"
            );
            return None;
        }
    };
    match serde_json::from_slice::<IntentRecord>(&bytes) {
        Ok(record) if record.schema_version <= INTENT_SCHEMA_VERSION => Some(record),
        Ok(record) => {
            tracing::error!(
                record = %intent_log.path.display(),
                schema_version = record.schema_version,
                "the record of an interrupted Vault write comes from a newer Hatchdoor; it was \
                 left in place and not acted on"
            );
            None
        }
        Err(error) => {
            tracing::error!(
                record = %intent_log.path.display(),
                error = %error,
                "the record of an interrupted Vault write cannot be read; it was left in place"
            );
            None
        }
    }
}

/// Sort a failed repair step: a refusal says the file no longer looks the
/// way the plan expects, which a retry cannot change, so it needs a
/// person; an I/O failure may pass, so the record is kept for the next start.
fn step_failed(
    vault_root: &Path,
    path: &Path,
    error: &WriteError,
    what: &str,
    repair: &mut WriteRepair,
) {
    tracing::warn!(
        path = %vault_relative(vault_root, path),
        error = ?error,
        "could not finish {what} an interrupted Vault write had planned"
    );
    match error {
        WriteError::Io(_) => repair.kept_for_retry = true,
        _ => repair
            .needs_attention
            .push(vault_relative(vault_root, path)),
    }
}

fn roll_forward(vault_root: &Path, intent_log: &IntentLog, repair: &mut WriteRepair) {
    let Some(record) = read_record(intent_log) else {
        return;
    };
    repair.operation = Some(record.operation.clone());

    let mut moves = Vec::with_capacity(record.moves.len());
    let mut unreadable_moves = false;
    for planned in &record.moves {
        match (
            decode_path(vault_root, &planned.source),
            decode_path(vault_root, &planned.destination),
        ) {
            (Some(source), Some(destination)) => moves.push((source, destination)),
            _ => {
                unreadable_moves = true;
                repair.needs_attention.push(planned.source.clone());
            }
        }
    }
    let mut rewrites = Vec::with_capacity(record.rewrites.len());
    for planned in &record.rewrites {
        match decode_path(vault_root, &planned.path) {
            Some(path) => rewrites.push((path, planned)),
            None => repair.needs_attention.push(planned.path.clone()),
        }
    }

    // Whether any step landed before the process died. One that had not
    // changed anything is not finished: the person who asked for it was
    // never told it happened, and the Vault is still whole as it was.
    let move_landed = moves.iter().any(|(source, destination)| {
        matches!(
            found_at(source),
            FoundAtPath::Missing | FoundAtPath::EmptyFolder
        ) && found_at(destination) == FoundAtPath::File
    });
    let rewrite_landed = rewrites.iter().any(|(path, planned)| {
        fs::read_to_string(path).is_ok_and(|current| content_hash(&current) == planned.applied_hash)
    });
    repair.rolled_forward = move_landed || rewrite_landed;

    // Every move must stand before a backlink is pointed at where it went:
    // a backlink rewritten towards a move that could not be finished would
    // name a note that is not there.
    let mut moves_complete = !unreadable_moves;
    for (source, destination) in &moves {
        match (found_at(source), found_at(destination)) {
            (
                FoundAtPath::File,
                destination_found @ (FoundAtPath::Missing | FoundAtPath::EmptyFolder),
            ) => {
                if destination_found == FoundAtPath::EmptyFolder {
                    remove_gate(vault_root, destination, repair);
                }
                if !repair.rolled_forward {
                    continue;
                }
                match move_file_no_follow(source, destination) {
                    Ok(()) => repair
                        .completed
                        .push(vault_relative(vault_root, destination)),
                    Err(error) => {
                        moves_complete = false;
                        step_failed(vault_root, source, &error, "moving a file", repair);
                    }
                }
            }
            (
                source_found @ (FoundAtPath::Missing | FoundAtPath::EmptyFolder),
                FoundAtPath::File,
            ) => {
                if source_found == FoundAtPath::EmptyFolder {
                    remove_gate(vault_root, source, repair);
                }
            }
            _ => {
                moves_complete = false;
                repair
                    .needs_attention
                    .push(vault_relative(vault_root, source));
            }
        }
    }

    if repair.rolled_forward && moves_complete {
        for (path, planned) in &rewrites {
            let current = match fs::read_to_string(path) {
                Ok(current) => content_hash(&current),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    repair
                        .needs_attention
                        .push(vault_relative(vault_root, path));
                    continue;
                }
                Err(error) => {
                    step_failed(
                        vault_root,
                        path,
                        &WriteError::Io(error.to_string()),
                        "rewriting a note",
                        repair,
                    );
                    continue;
                }
            };
            if current == planned.applied_hash {
                continue;
            }
            if current != planned.original_hash {
                // Edited since the plan read it. Rewriting it would replace
                // that edit with text built from the older copy.
                repair
                    .needs_attention
                    .push(vault_relative(vault_root, path));
                continue;
            }
            let modified = captured_modified_time(path);
            match atomic_write_if_unchanged_keeping_modified(
                path,
                &planned.content,
                &planned.original_hash,
                modified,
            ) {
                Ok(()) => repair.completed.push(vault_relative(vault_root, path)),
                Err(error) => step_failed(vault_root, path, &error, "rewriting a note", repair),
            }
        }
    } else if repair.rolled_forward {
        tracing::warn!(
            vault = %vault_root.display(),
            operation = %record.operation,
            "a move an interrupted Vault write had planned could not be finished, so its \
             backlinks were left as they were"
        );
    }

    if repair.rolled_forward {
        tracing::warn!(
            vault = %vault_root.display(),
            operation = %record.operation,
            completed = repair.completed.len(),
            needs_attention = repair.needs_attention.len(),
            "finished a Vault write that was interrupted when Hatchdoor stopped"
        );
    } else {
        tracing::warn!(
            vault = %vault_root.display(),
            operation = %record.operation,
            "a Vault write was interrupted before it changed anything; the Vault was left as it was"
        );
    }
    for path in &repair.needs_attention {
        tracing::warn!(
            vault = %vault_root.display(),
            operation = %record.operation,
            path = %path,
            "needs attention: an interrupted Vault write could not be finished here, because the \
             file no longer looks the way the write planned"
        );
    }
    if repair.kept_for_retry {
        // Kept, so the next start carries on from here: every step is
        // idempotent. Until then a new multi-note write in this Vault is
        // refused rather than replacing this record.
        tracing::error!(
            vault = %vault_root.display(),
            operation = %record.operation,
            record = %intent_log.path.display(),
            "an interrupted Vault write could not be finished because of a filesystem error; its \
             record was kept and the next start tries again"
        );
    } else {
        intent_log.clear();
    }
}

fn remove_gate(vault_root: &Path, gate: &Path, repair: &mut WriteRepair) {
    match fs::remove_dir(gate) {
        Ok(()) => {
            tracing::warn!(
                path = %vault_relative(vault_root, gate),
                "removed the empty folder an interrupted move left as its reservation"
            );
            repair
                .removed_leftovers
                .push(vault_relative(vault_root, gate));
        }
        Err(error) => tracing::warn!(
            path = %vault_relative(vault_root, gate),
            error = %error,
            "could not remove the empty folder an interrupted move left as its reservation"
        ),
    }
}

/// Whether `name` is a temporary file a Hatchdoor write parks its bytes in:
/// `.<file name>.hatchdoor-tmp-<32 hex digits>`.
fn is_temporary_file_name(name: &[u8]) -> bool {
    const MARKER: &[u8] = b".hatchdoor-tmp-";
    let Some(rest) = name.strip_prefix(b".") else {
        return false;
    };
    let Some(suffix) = rest.len().checked_sub(32) else {
        return false;
    };
    let (front, random) = rest.split_at(suffix);
    random.iter().all(u8::is_ascii_hexdigit)
        && front.len() > MARKER.len()
        && front.ends_with(MARKER)
}

/// Move every temporary file a write left in the Vault to `leftovers`, at
/// the same Vault-relative path. No write is in flight when a Vault is first
/// activated, so each one belongs to a write that never finished. It is
/// moved rather than deleted because it can hold the only copy of an edit:
/// a save made in another editor, which the exchange displaced into it and
/// a crash, or a failed undo, kept from being put back. The Vault's Git
/// folder is not entered, and neither is any symlink.
fn sweep_temporary_files(vault_root: &Path, leftovers: &Path, repair: &mut WriteRepair) {
    let mut folders = vec![vault_root.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let Ok(entries) = fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let name = entry.file_name();
            if file_type.is_dir() {
                if !(folder == vault_root && name == ".git") {
                    folders.push(entry.path());
                }
            } else if file_type.is_file() && is_temporary_file_name(name.as_bytes()) {
                let path = entry.path();
                let relative = vault_relative(vault_root, &path);
                let kept = leftovers.join(path.strip_prefix(vault_root).unwrap_or(&path));
                match move_out_of_vault(&path, &kept) {
                    Ok(()) => {
                        tracing::warn!(
                            path = %relative,
                            kept = %kept.display(),
                            "moved a temporary file an interrupted Vault write left behind out \
                             of the Vault; it may hold an edit that was never saved to its note"
                        );
                        repair.removed_leftovers.push(relative);
                    }
                    Err(error) => tracing::warn!(
                        path = %relative,
                        error = %error,
                        "could not move a temporary file an interrupted Vault write left behind \
                         out of the Vault"
                    ),
                }
            }
        }
    }
}

/// Move `path` to `kept`, copying when the state folder is on another
/// filesystem. `kept` is never replaced: the random part of a temporary
/// file's name makes a clash a second leftover of the same write, which is
/// left where it is.
fn move_out_of_vault(path: &Path, kept: &Path) -> std::io::Result<()> {
    if let Some(parent) = kept.parent() {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)?;
    }
    if fs::symlink_metadata(kept).is_ok() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("'{}' already exists", kept.display()),
        ));
    }
    match fs::rename(path, kept) {
        Ok(()) => Ok(()),
        Err(error) if error.raw_os_error() == Some(libc::EXDEV) => {
            fs::copy(path, kept)?;
            fs::File::open(kept)?.sync_all()?;
            fs::remove_file(path)
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_survives_the_record_byte_for_byte() {
        let root = Path::new("/vault");
        let name = OsString::from_vec(b"Notes/caf\xe9 100%.md".to_vec());
        let encoded = encode_path(root, &root.join(&name)).expect("encode");
        assert_eq!(encoded, "Notes/caf%E9 100%25.md");
        assert_eq!(decode_path(root, &encoded), Some(root.join(name)));
    }

    #[test]
    fn a_recorded_path_cannot_leave_the_vault() {
        let root = Path::new("/vault");
        assert_eq!(decode_path(root, "../etc/passwd"), None);
        assert_eq!(decode_path(root, "/etc/passwd"), None);
        assert_eq!(decode_path(root, ""), None);
        assert_eq!(decode_path(root, "bad%G1"), None);
    }

    #[test]
    fn only_a_hatchdoor_temporary_name_is_recognised() {
        let random = "0123456789abcdef0123456789abcdef";
        assert!(is_temporary_file_name(
            format!(".Hub.md.hatchdoor-tmp-{random}").as_bytes()
        ));
        assert!(!is_temporary_file_name(
            format!("Hub.md.hatchdoor-tmp-{random}").as_bytes()
        ));
        assert!(!is_temporary_file_name(
            format!(".Hub.md.hatchdoor-tmp-{}", &random[1..]).as_bytes()
        ));
        assert!(!is_temporary_file_name(
            format!(".hatchdoor-tmp-{random}").as_bytes()
        ));
        assert!(!is_temporary_file_name(b".Hub.md"));
    }
}
