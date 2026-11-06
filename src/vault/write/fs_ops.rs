use std::cell::Cell;
use std::ffi::CString;
use std::fs::{self, FileTimes};
use std::io::Write;
use std::os::fd::{AsRawFd, FromRawFd};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::cache::parse::content_hash;
use crate::rename_flags::{RenameFlag, rename_flagged_at};
use crate::vault::types::NoteEntry;

use super::intent::{IntentLog, IntentRecord};
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
        /// The note's modification time before the rewrite, so restoring
        /// its content does not also stamp it with the time of the rollback.
        original_modified: Option<SystemTime>,
    },
}

/// The steps of one multi-note write, so a failure part-way can undo them.
///
/// The undo lives in memory and lasts as long as the process. What outlives
/// it is the plan, which [`MutationJournal::begin`] writes to the Vault's
/// [`IntentLog`] before the first step and which [`MutationJournal::finish`]
/// and [`MutationJournal::rollback`] remove, so a process that dies part-way
/// leaves the record a later activation finishes (#631, ADR-51).
pub(super) struct MutationJournal {
    vault_root: PathBuf,
    /// The Vault's intent log, once [`MutationJournal::begin`] has written
    /// this journal's plan to it. Only a journal that wrote the record
    /// removes it, so a write refused because an earlier record is still
    /// waiting leaves that record alone.
    recorded: Option<IntentLog>,
    intent_log: Option<IntentLog>,
    completed: Vec<Compensation>,
    /// Step boundaries passed, for the tests that stop the write at each.
    #[cfg(test)]
    steps: usize,
}

#[cfg(test)]
thread_local! {
    /// The step boundary at which the tests' simulated crash happens: the
    /// journal panics there, which unwinds past every rollback and leaves
    /// the record behind exactly as a dead process would.
    static CRASH_AFTER_STEPS: Cell<Option<usize>> = const { Cell::new(None) };
}

/// Make this thread's next journals panic once they have passed `steps` step
/// boundaries: 0 is just after the record is written, and each completed
/// move or rewrite is one more.
#[cfg(test)]
pub(super) fn crash_after_steps(steps: Option<usize>) {
    CRASH_AFTER_STEPS.set(steps);
}

impl MutationJournal {
    /// `intent_log` is `None` only for a caller with no state folder, which
    /// is a test: every Vault the runtime activates has one.
    pub(super) fn new(vault_root: &Path, intent_log: Option<&IntentLog>) -> Self {
        Self {
            vault_root: vault_root.to_path_buf(),
            recorded: None,
            intent_log: intent_log.cloned(),
            completed: Vec::new(),
            #[cfg(test)]
            steps: 0,
        }
    }

    /// Record the whole plan durably before its first step. A plan that
    /// cannot be recorded is refused, and nothing has changed yet.
    pub(super) fn begin(
        &mut self,
        operation: &str,
        moves: &[(&Path, &Path)],
        rewrites: &[TextRewrite],
    ) -> Result<(), WriteError> {
        if let Some(intent_log) = self.intent_log.take() {
            intent_log.record(&IntentRecord::new(
                &self.vault_root,
                operation,
                moves,
                rewrites,
            )?)?;
            self.recorded = Some(intent_log);
        }
        self.step_boundary();
        Ok(())
    }

    /// The operation finished: its record goes.
    pub(super) fn finish(self) {
        if let Some(intent_log) = &self.recorded {
            intent_log.clear();
        }
    }

    fn step_boundary(&mut self) {
        #[cfg(test)]
        {
            let passed = self.steps;
            self.steps += 1;
            if CRASH_AFTER_STEPS.get() == Some(passed) {
                panic!("simulated crash after {passed} steps");
            }
        }
    }

    pub(super) fn move_file(
        &mut self,
        phase: MutationPhase,
        source: &Path,
        destination: &Path,
    ) -> Result<(), WriteError> {
        let result = try_move_file_no_follow(source, destination);
        self.retain_completed_move(phase, source, destination, result)
    }

    pub(super) fn move_note(
        &mut self,
        source: &Path,
        destination: &Path,
        expected_content_hash: &str,
    ) -> Result<(), WriteError> {
        self.move_note_with_after_exchange(source, destination, expected_content_hash, || {})
    }

    fn move_note_with_after_exchange(
        &mut self,
        source: &Path,
        destination: &Path,
        expected_content_hash: &str,
        after_exchange: impl FnOnce(),
    ) -> Result<(), WriteError> {
        let result =
            try_move_file_if_unchanged(source, destination, expected_content_hash, after_exchange);
        self.retain_completed_move(MutationPhase::Note, source, destination, result)
    }

    /// Record a move the rollback must undo: one that succeeded, and one the
    /// primitive itself reports as landed although a step after its commit
    /// failed. The primitive says so from where the failure happened rather
    /// than this inferring it from what the paths look like afterwards: an
    /// occupied destination and a vanished source are also what a move
    /// refused at its gate looks like when something outside Hatchdoor
    /// deleted the source meanwhile, and undoing that "move" would carry an
    /// unrelated file off the destination.
    fn retain_completed_move(
        &mut self,
        phase: MutationPhase,
        source: &Path,
        destination: &Path,
        result: Result<(), MoveError>,
    ) -> Result<(), WriteError> {
        let landed = match &result {
            Ok(()) => true,
            Err(MoveError::Landed(_)) => true,
            Err(MoveError::NotMoved(_)) => false,
        };
        if landed {
            self.completed.push(Compensation::MoveBack {
                phase,
                source: source.to_path_buf(),
                destination: destination.to_path_buf(),
            });
            self.step_boundary();
        }
        result.map_err(MoveError::into_write_error)
    }

    pub(super) fn apply_rewrites(
        &mut self,
        rewrites: Vec<TextRewrite>,
    ) -> Result<Vec<PathBuf>, WriteError> {
        self.apply_rewrites_with_before_commit(rewrites, |_| {})
    }

    /// Commit each planned rewrite against the hash its plan was built from.
    ///
    /// The expectation is [`TextRewrite::original_hash`], carried from plan
    /// time, not a hash of the read below: the read exists to retain the text
    /// a rollback would restore, and using it as the CAS expectation would
    /// only detect a concurrent save that landed in the microseconds between
    /// the read and the rename, not one that landed while earlier notes in
    /// the same loop were being written. A note that moved on since the plan
    /// read it is refused with [`WriteError::Conflict`] and nothing is
    /// written to it (#321).
    ///
    /// A rewrite here is bookkeeping, not authorship: the note changes only
    /// because a name it carries changed elsewhere, a note or attachment it
    /// links to or a tag renamed across the Vault. So the committed file
    /// keeps the modification time the note had before, and the Vault's
    /// writing history survives a rename that touches hundreds of notes
    /// (#299).
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
            let original_modified = captured_modified_time(&rewrite.path);
            if content_hash(&original) != rewrite.original_hash {
                // Vault-relative, because this message reaches an MCP client
                // verbatim and the absolute host path is nobody's business
                // there.
                return Err(WriteError::Conflict(format!(
                    "Note '{}' changed after this change was planned; nothing was rewritten in it",
                    bounded_vault_path(&self.vault_root, &rewrite.path)
                )));
            }
            before_commit(&rewrite.path);
            let result = atomic_write_if_unchanged_keeping_modified(
                &rewrite.path,
                &rewrite.content,
                &rewrite.original_hash,
                original_modified,
            );
            let committed_despite_error = result.is_err()
                && fs::read_to_string(&rewrite.path)
                    .is_ok_and(|current| current == rewrite.content);
            if result.is_ok() || committed_despite_error {
                rewritten.push(rewrite.path.clone());
                self.completed.push(Compensation::RestoreRewrite {
                    path: rewrite.path,
                    original,
                    applied_hash: content_hash(&rewrite.content),
                    original_modified,
                });
                self.step_boundary();
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

    /// Undo every completed step, then remove the record: the caller is
    /// told the outcome, so a later activation must not finish what this
    /// one reported as failed, even when the undo was incomplete and the
    /// failure says so.
    fn rollback_with_observer(
        &mut self,
        cause: WriteError,
        observer: impl FnMut(&'static str),
    ) -> WriteError {
        let error = self.undo_completed(cause, observer);
        if let Some(intent_log) = self.recorded.take() {
            intent_log.clear();
        }
        error
    }

    fn undo_completed(
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
                    original_modified,
                } => {
                    let result = atomic_write_if_unchanged_keeping_modified(
                        &path,
                        &original,
                        &applied_hash,
                        original_modified,
                    );
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

fn bounded_vault_path(vault_root: &Path, path: &Path) -> String {
    path.strip_prefix(vault_root)
        .ok()
        .filter(|relative| !relative.as_os_str().is_empty())
        .map(|relative| relative.to_string_lossy().into_owned())
        .unwrap_or_else(|| bounded_write_name(path))
}

/// The note's own name, with the directories it sits under left off. A
/// recovery message is the one write failure that reaches an API client
/// unsanitized, so it carries this rather than the absolute host path the
/// rest of this layer puts in its logs.
fn bounded_write_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
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
        // Raised while planning, before anything exists to roll back.
        refused @ (WriteError::LinkRewriteUnsupported(_) | WriteError::HeadingNotFound(_)) => {
            refused
        }
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
    let actual = content_hash(&read_existing_note(path, label)?);
    if actual != expected {
        return Err(WriteError::Conflict(format!(
            "note changed since it was read: expected {expected}, found {actual}"
        )));
    }
    Ok(())
}

/// The text of the note at `path`, opened without following a symbolic link
/// in any component and read under `MAX_NOTE_BYTES` (#670). A note over the
/// bound is refused unread: Hatchdoor reads no note that large, so it cannot
/// edit one either.
pub(super) fn read_existing_note(path: &Path, label: &str) -> Result<String, WriteError> {
    use crate::vault::{BoundedReadError, MAX_NOTE_BYTES, read_open_bounded};

    let io_error = |error: &dyn std::fmt::Display| {
        WriteError::Io(format!("failed to read note '{label}': {error}"))
    };
    let file = open_existing_file_no_follow(path).map_err(|error| io_error(&error))?;
    let bytes = read_open_bounded(file, MAX_NOTE_BYTES).map_err(|error| match error {
        BoundedReadError::TooLarge { limit } => WriteError::InvalidInput(format!(
            "note '{label}' is larger than {} MiB, the most Hatchdoor reads or edits",
            limit / (1024 * 1024)
        )),
        BoundedReadError::NotRegular => {
            WriteError::InvalidInput(format!("note '{label}' is not a regular file"))
        }
        BoundedReadError::Io(error) => io_error(&error),
    })?;
    String::from_utf8(bytes).map_err(|error| io_error(&error))
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

/// Write a new file at a name that must still be free when the write
/// commits. A file that appeared there after the caller checked, from
/// anything outside Hatchdoor, is refused with [`WriteError::Conflict`] and
/// left untouched rather than replaced.
pub(super) fn atomic_create(path: &Path, content: &str) -> Result<(), WriteError> {
    atomic_write_inner(
        path,
        content.as_bytes(),
        Commit::CreateNew,
        None,
        || {},
        || {},
    )
}

/// [`atomic_create`] for bytes, with a hook that runs once they are prepared
/// and before the name is taken: the window in which something outside
/// Hatchdoor can create the file the caller found free.
pub(super) fn atomic_create_bytes_with_before_commit(
    path: &Path,
    bytes: &[u8],
    before_commit: impl FnOnce(),
) -> Result<(), WriteError> {
    atomic_write_inner(path, bytes, Commit::CreateNew, None, before_commit, || {})
}

pub(super) fn atomic_write_bytes(path: &Path, bytes: &[u8]) -> Result<(), WriteError> {
    atomic_write_inner(path, bytes, Commit::Replace, None, || {}, || {})
}

fn atomic_write_bytes_if_unchanged(
    path: &Path,
    bytes: &[u8],
    expected_content_hash: &str,
) -> Result<(), WriteError> {
    atomic_write_inner(
        path,
        bytes,
        Commit::IfUnchanged(expected_content_hash),
        None,
        || {},
        || {},
    )
}

/// A conditional write that leaves the note carrying `modified` rather than
/// the time of the write. `None` means the original time could not be read,
/// and the note takes a fresh one as any other write would.
pub(super) fn atomic_write_if_unchanged_keeping_modified(
    path: &Path,
    content: &str,
    expected_content_hash: &str,
    modified: Option<SystemTime>,
) -> Result<(), WriteError> {
    atomic_write_inner(
        path,
        content.as_bytes(),
        Commit::IfUnchanged(expected_content_hash),
        modified,
        || {},
        || {},
    )
}

/// Read the modification time a bookkeeping rewrite should keep. A failure
/// only costs the note its old timestamp, so it is logged, not raised.
pub(super) fn captured_modified_time(path: &Path) -> Option<SystemTime> {
    match fs::symlink_metadata(path).and_then(|metadata| metadata.modified()) {
        Ok(modified) => Some(modified),
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                error = %error,
                "could not read a note's modification time; the rewrite will refresh it"
            );
            None
        }
    }
}

/// Stamp the prepared file with the time the note had before. This runs on
/// the temporary file ahead of the commit, so the note is never visible
/// under its real name carrying the wrong time. The content is what the
/// caller asked for; a timestamp the filesystem refuses is logged and the
/// write goes ahead, since a fresh time is exactly what every write did
/// before #299.
fn keep_modified_time(file: &fs::File, path: &Path, modified: SystemTime) {
    if let Err(error) = file.set_times(FileTimes::new().set_modified(modified)) {
        tracing::warn!(
            path = %path.display(),
            error = %error,
            "could not preserve a note's modification time; it now carries the time of this write"
        );
    }
}

#[cfg(test)]
fn atomic_create_with_before_commit(
    path: &Path,
    content: &str,
    before_commit: impl FnOnce(),
) -> Result<(), WriteError> {
    atomic_write_inner(
        path,
        content.as_bytes(),
        Commit::CreateNew,
        None,
        before_commit,
        || {},
    )
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
        Commit::IfUnchanged(expected_content_hash),
        None,
        before_exchange,
        || {},
    )
}

/// Drives the window between the commit exchange and the read-back that
/// verifies what the exchange displaced. Only a test needs to reach in there,
/// to stand in for an external process touching the Vault directory mid-write.
#[cfg(test)]
fn atomic_write_if_unchanged_with_after_exchange(
    path: &Path,
    content: &str,
    expected_content_hash: &str,
    after_exchange: impl FnOnce(),
) -> Result<(), WriteError> {
    atomic_write_inner(
        path,
        content.as_bytes(),
        Commit::IfUnchanged(expected_content_hash),
        None,
        || {},
        after_exchange,
    )
}

/// How a prepared write takes the destination name.
enum Commit<'a> {
    /// Whatever is there is replaced.
    Replace,
    /// Only the content with this hash is replaced; anything else is a
    /// conflict and is put back.
    IfUnchanged(&'a str),
    /// Only a free name is taken. Anything that appeared at it since the
    /// caller looked is a conflict and is left alone.
    CreateNew,
}

fn atomic_write_inner(
    path: &Path,
    bytes: &[u8],
    commit: Commit<'_>,
    preserved_modified: Option<SystemTime>,
    before_commit: impl FnOnce(),
    after_exchange: impl FnOnce(),
) -> Result<(), WriteError> {
    let (parent, filename) = open_parent_dir_no_follow(path)?;
    // The file being replaced, when there is one, so the new file can take
    // over its mode and owner. A rename swaps in a whole new inode, which
    // otherwise keeps whatever this process created it with (#630).
    let replaced = metadata_at_no_follow(&parent, &filename)
        .ok()
        .filter(|metadata| metadata.st_mode & libc::S_IFMT == libc::S_IFREG);
    let (tmp_name, mut file) = create_unique_temporary_file(
        &parent,
        &filename,
        // A new file asks for 0666 and lets the umask, or the folder's
        // default ACL, decide, like any other program creating one. A
        // replacement starts private and is given the old file's mode
        // below, so its bytes are never readable more widely than the
        // note's were.
        if replaced.is_some() { 0o600 } else { 0o666 },
    )?;
    // From here every early return removes the sidecar, so a refused or
    // failed write leaves nothing behind for Git to pick up (#630).
    let sidecar = PendingSidecar {
        parent: &parent,
        name: tmp_name,
        armed: Cell::new(true),
    };
    let tmp_name = &sidecar.name;

    // Write and fsync the temp file so its bytes are durable BEFORE the rename.
    // Without the fsync, a crash just after the rename can leave the note file's
    // name pointing at data the OS never flushed (an empty or truncated file).
    file.write_all(bytes)
        .and_then(|()| {
            if let Some(replaced) = &replaced {
                let kept = keep_mode_and_owner(&file, path, replaced);
                keep_access_list(&file, path, replaced, kept, &parent, &filename);
            }
            // Stamped after the last byte is written, since any later write
            // would refresh it, and before the fsync makes it durable.
            if let Some(modified) = preserved_modified {
                keep_modified_time(&file, path, modified);
            }
            file.sync_all()
        })
        .map_err(|error| {
            WriteError::Io(format!(
                "failed to write temporary file for '{}': {error}",
                path.display()
            ))
        })?;
    drop(file);
    before_commit();

    match commit {
        Commit::IfUnchanged(expected) => {
            // Exchange is the commit point: the displaced destination is held at
            // our private name, where we verify its identity. A concurrent atomic
            // save therefore becomes detectable and is swapped back, rather than
            // being silently overwritten in a check-then-rename gap.
            if let Err(error) = rename_exchange(&parent, tmp_name, &parent, &filename) {
                if !exchange_unavailable(commit_directory(path), &error) {
                    return Err(WriteError::from(error));
                }
                // The hook stands in for an external process touching the Vault
                // directory at the commit point. On this path that point is the
                // instant before the rename rather than just after an exchange,
                // so it is handed over to fire there.
                commit_checked_without_exchange(
                    &parent,
                    tmp_name,
                    &filename,
                    path,
                    expected,
                    after_exchange,
                )?;
                sidecar.disarm();
                let _ = parent.sync_all();
                return Ok(());
            }
            // The sidecar's name now holds the displaced note, which the
            // checks below either remove or deliberately keep for recovery.
            sidecar.disarm();
            after_exchange();
            // The exchange is the commit point. From here the destination already
            // holds the new bytes, so every failure below turns on whether the
            // undo put them back: reporting one as a plain failure told a caller
            // its write had not landed while the new content sat committed.
            //
            // A recovery message reaches an API client unsanitized, so it names
            // the note and the sidecar without their directories.
            let note = bounded_write_name(path);
            let sidecar_name = tmp_name.to_string_lossy().into_owned();
            let prior = match read_file_at_no_follow(&parent, tmp_name) {
                Ok(prior) => prior,
                Err(read_error) => {
                    return Err(
                        match rename_exchange(&parent, tmp_name, &parent, &filename) {
                            Ok(()) => {
                                let _ = unlink_at(&parent, tmp_name);
                                WriteError::Io(format!(
                                    "failed to inspect replaced note '{}': {read_error}",
                                    path.display()
                                ))
                            }
                            Err(undo_error) => WriteError::recovery_required(format!(
                                "note '{note}' was replaced, its prior content could not be read \
                             back ({read_error}), and the replacement could not be undone \
                             ({undo_error}). The new content is committed and was never \
                             checked against the expected hash; the prior content is in \
                             '{sidecar_name}' beside it if that file still exists."
                            )),
                        },
                    );
                }
            };
            if content_hash(&prior) != expected.trim() {
                if let Err(undo_error) = rename_exchange(&parent, tmp_name, &parent, &filename) {
                    return Err(WriteError::recovery_required(format!(
                        "note '{note}' changed since it was read, and the replacement could not be \
                     undone ({undo_error}). The new content is committed over that change, \
                     whose content is in '{sidecar_name}' beside it if that file still exists."
                    )));
                }
                let _ = unlink_at(&parent, tmp_name);
                return Err(WriteError::Conflict(format!(
                    "note changed since it was read: expected {}, found {}",
                    expected.trim(),
                    content_hash(&prior)
                )));
            }
            // Committed and verified. A sidecar that will not unlink is
            // dot-prefixed litter the Vault already excludes as noise, and
            // failing here would report a correct write as a failure.
            let _ = unlink_at(&parent, tmp_name);
        }
        Commit::Replace => {
            ensure_safe_destination_at(&parent, &filename, path)?;
            rename_at(&parent, tmp_name, &parent, &filename)?;
            sidecar.disarm();
        }
        Commit::CreateNew => {
            if let Err(error) =
                rename_flagged_at(&parent, tmp_name, &parent, &filename, RenameFlag::NoReplace)
            {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    return Err(WriteError::Conflict(format!(
                        "'{}' already exists",
                        bounded_write_name(path)
                    )));
                }
                if !crate::rename_flags::flag_unavailable(
                    commit_directory(path),
                    RenameFlag::NoReplace,
                    &error,
                ) {
                    return Err(WriteError::Io(format!(
                        "failed to create '{}': {error}",
                        path.display()
                    )));
                }
                // A filesystem that cannot refuse an occupied name keeps the
                // plain rename, and with it the caller's own existence check
                // as the only guard: the same race every write on such a
                // filesystem accepts (ADR-26).
                ensure_safe_destination_at(&parent, &filename, path)?;
                rename_at(&parent, tmp_name, &parent, &filename)?;
            }
            sidecar.disarm();
        }
    }

    // fsync the parent directory so the rename itself (a directory metadata
    // change) survives a crash — otherwise the durable temp data can still be
    // lost if the directory entry update was not flushed.
    let _ = parent.sync_all();
    Ok(())
}

/// Commit a conditional write on a filesystem that cannot exchange two names.
///
/// This is the pre-2.5.0 shape: read the destination, compare it against the
/// expected hash, then rename the prepared temporary file over it. Every
/// precondition the exchange enforced still holds and every error a caller can
/// see keeps its code and its wording — a stale hash is still
/// [`WriteError::Conflict`] with the same sentence. What is gone is the
/// atomicity: a writer outside Hatchdoor that saves the note between the check
/// and the rename has its save overwritten instead of detected. That race is
/// the price of writing at all on such a filesystem, and the Vault reports it
/// once when its runtime is established rather than per write (ADR-26).
///
/// On failure nothing was committed, and the caller's [`PendingSidecar`]
/// removes the prepared bytes.
fn commit_checked_without_exchange(
    parent: &fs::File,
    tmp_name: &CString,
    filename: &CString,
    path: &Path,
    expected: &str,
    in_commit_window: impl FnOnce(),
) -> Result<(), WriteError> {
    // The destination's shape is checked before it is read, not after.
    // The exchange path learns what it displaced by reading it back and
    // can undo the swap if it dislikes the answer. Here the read is the
    // check, so it must not be the first thing to touch the destination:
    // opening a FIFO blocks until someone writes to it, and a blocked
    // write is worse than a refused one.
    ensure_safe_destination_at(parent, filename, path)?;
    let prior = read_file_at_no_follow(parent, filename).map_err(|error| {
        WriteError::Io(format!(
            "failed to read note '{}' before replacing it: {error}",
            path.display()
        ))
    })?;
    if content_hash(&prior) != expected.trim() {
        return Err(WriteError::Conflict(format!(
            "note changed since it was read: expected {}, found {}",
            expected.trim(),
            content_hash(&prior)
        )));
    }
    in_commit_window();
    rename_at(parent, tmp_name, parent, filename)
}

/// A prepared temporary file that has not yet taken the destination's name.
/// Dropping it while still armed unlinks it, so every early return between
/// its creation and the commit cleans up after itself rather than each one
/// having to remember to (#630).
struct PendingSidecar<'a> {
    parent: &'a fs::File,
    name: CString,
    armed: Cell<bool>,
}

impl PendingSidecar<'_> {
    /// The name no longer holds the prepared bytes, either because they were
    /// committed or because an exchange put the displaced note there, which
    /// its caller handles explicitly.
    fn disarm(&self) {
        self.armed.set(false);
    }
}

impl Drop for PendingSidecar<'_> {
    fn drop(&mut self) {
        if self.armed.get() {
            let _ = unlink_at(self.parent, &self.name);
        }
    }
}

/// Which of the replaced file's owner and group the prepared file ended up
/// with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Kept {
    owner: bool,
    group: bool,
}

/// Give the prepared file the mode and owner of the file it replaces. The
/// owner is changed only where this process may, which in practice means as
/// root; anywhere else the file keeps the old group when this process belongs
/// to it, and the mode is still carried over. Neither failure stops the
/// write: the content is what the caller asked for, and before #630 every
/// write lost both.
///
/// Set-ID and sticky bits are not carried over. The bytes are the caller's,
/// so a setuid file replaced as root would otherwise run the caller's
/// content with its owner's rights.
fn keep_mode_and_owner(file: &fs::File, path: &Path, replaced: &libc::stat) -> Kept {
    let fd = file.as_raw_fd();
    // What the prepared file was actually created with: a setgid folder
    // gives it the folder's group rather than this process's.
    let kept = |file_stat: &libc::stat| Kept {
        owner: file_stat.st_uid == replaced.st_uid,
        group: file_stat.st_gid == replaced.st_gid,
    };
    let created = fstat_fd(fd).map_or(
        Kept {
            owner: true,
            group: true,
        },
        |created| kept(&created),
    );
    if (!created.owner || !created.group)
        && unsafe { libc::fchown(fd, replaced.st_uid, replaced.st_gid) } != 0
    {
        let owner_error = std::io::Error::last_os_error();
        // Keeping the group alone needs only membership in it.
        let kept_group =
            created.group || unsafe { libc::fchown(fd, libc::uid_t::MAX, replaced.st_gid) } == 0;
        tracing::debug!(
            path = %path.display(),
            error = %owner_error,
            kept_group,
            "could not keep a note's owner; it now belongs to Hatchdoor's user"
        );
    }
    if unsafe { libc::fchmod(fd, replaced.st_mode & 0o777) } != 0 {
        tracing::warn!(
            path = %path.display(),
            error = %std::io::Error::last_os_error(),
            "could not keep a note's permissions; it now carries Hatchdoor's defaults"
        );
    }
    fstat_fd(fd).map_or(created, |result| kept(&result))
}

fn fstat_fd(fd: libc::c_int) -> Option<libc::stat> {
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    (unsafe { libc::fstat(fd, stat.as_mut_ptr()) } == 0).then(|| unsafe { stat.assume_init() })
}

/// Where the owner or group could not be kept, they keep their access
/// through ACL entries naming them instead (#661): a note owned by the user
/// who granted Hatchdoor access with `setfacl` would otherwise come back
/// owned by Hatchdoor, leaving that user only "other" rights. The replaced
/// file's own access ACL is carried over too, which is what keeps those
/// entries through every later write, once the note already belongs to
/// Hatchdoor. A filesystem without ACLs keeps only the mode.
fn keep_access_list(
    file: &fs::File,
    path: &Path,
    replaced: &libc::stat,
    kept: Kept,
    parent: &fs::File,
    filename: &CString,
) {
    let old_acl = read_access_acl_at(parent, filename, replaced);
    let Some(acl) = carried_access_acl(replaced, old_acl.as_deref(), kept) else {
        return;
    };
    let value = encode_acl(&acl);
    let result = unsafe {
        libc::fsetxattr(
            file.as_raw_fd(),
            ACL_ACCESS_XATTR.as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
        )
    };
    if result != 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::EOPNOTSUPP) {
            tracing::debug!(
                path = %path.display(),
                "the filesystem has no ACLs, so a note's former owner keeps only its mode"
            );
        } else {
            tracing::warn!(
                path = %path.display(),
                error = %error,
                "could not keep a note's access list; its former owner may have lost access"
            );
        }
    }
}

/// The extended attribute holding a file's access ACL on Linux.
const ACL_ACCESS_XATTR: &std::ffi::CStr = c"system.posix_acl_access";

// Linux's ACL extended-attribute layout, from `linux/posix_acl_xattr.h` and
// `linux/posix_acl.h`: a little-endian version word, then one
// `{tag: u16, perm: u16, id: u32}` per entry.
const ACL_XATTR_VERSION: u32 = 2;
const ACL_USER_OBJ: u16 = 0x01;
const ACL_USER: u16 = 0x02;
const ACL_GROUP_OBJ: u16 = 0x04;
const ACL_GROUP: u16 = 0x08;
const ACL_MASK: u16 = 0x10;
const ACL_OTHER: u16 = 0x20;
const ACL_UNDEFINED_ID: u32 = u32::MAX;

/// The id the kernel reports for a user or group this process's user
/// namespace cannot see (the default `overflowuid` and `overflowgid`), as
/// in rootless Podman for a note owned by another host user. Naming it in
/// an ACL would grant access to whoever 65534 maps to, not to the owner.
const OVERFLOW_ID: u32 = 65534;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AclEntry {
    tag: u16,
    perm: u16,
    id: u32,
}

/// The entries the mask limits.
fn masked(entry: &AclEntry) -> bool {
    matches!(entry.tag, ACL_USER | ACL_GROUP_OBJ | ACL_GROUP)
}

/// The replaced file's access ACL, or `None` when it has none beyond its
/// mode, cannot be found, is no longer the file `replaced` describes, or
/// lives on a filesystem without ACLs.
///
/// Reading an ACL needs no read permission on the file, so the file is
/// opened only as a path and its ACL read through `/proc/self/fd`, which
/// every Linux system and container has.
fn read_access_acl_at(
    parent: &fs::File,
    filename: &CString,
    replaced: &libc::stat,
) -> Option<Vec<AclEntry>> {
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            filename.as_ptr(),
            libc::O_PATH | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    let file = file_from_fd(fd).ok()?;
    let current = fstat_fd(file.as_raw_fd())?;
    if (current.st_dev, current.st_ino) != (replaced.st_dev, replaced.st_ino) {
        return None;
    }
    let proc_path = CString::new(format!("/proc/self/fd/{}", file.as_raw_fd())).ok()?;
    let mut buffer = Vec::new();
    // The size can change between asking and reading; ERANGE asks again.
    for _ in 0..4 {
        let size = unsafe {
            libc::getxattr(
                proc_path.as_ptr(),
                ACL_ACCESS_XATTR.as_ptr(),
                std::ptr::null_mut(),
                0,
            )
        };
        buffer.resize(usize::try_from(size).ok()?, 0);
        let length = unsafe {
            libc::getxattr(
                proc_path.as_ptr(),
                ACL_ACCESS_XATTR.as_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
            )
        };
        if let Ok(length) = usize::try_from(length) {
            return decode_acl(&buffer[..length]);
        }
        if std::io::Error::last_os_error().raw_os_error() != Some(libc::ERANGE) {
            return None;
        }
    }
    None
}

fn decode_acl(bytes: &[u8]) -> Option<Vec<AclEntry>> {
    let (version, entries) = bytes.split_first_chunk::<4>()?;
    if u32::from_le_bytes(*version) != ACL_XATTR_VERSION || entries.len() % 8 != 0 {
        return None;
    }
    Some(
        entries
            .chunks_exact(8)
            .map(|entry| AclEntry {
                tag: u16::from_le_bytes([entry[0], entry[1]]),
                perm: u16::from_le_bytes([entry[2], entry[3]]),
                id: u32::from_le_bytes([entry[4], entry[5], entry[6], entry[7]]),
            })
            .collect(),
    )
}

fn encode_acl(entries: &[AclEntry]) -> Vec<u8> {
    let mut bytes = ACL_XATTR_VERSION.to_le_bytes().to_vec();
    for entry in entries {
        bytes.extend_from_slice(&entry.tag.to_le_bytes());
        bytes.extend_from_slice(&entry.perm.to_le_bytes());
        bytes.extend_from_slice(&entry.id.to_le_bytes());
    }
    bytes
}

/// The access ACL the replacement needs, or `None` when the rename alone
/// already gives everyone the access they had: the owner and group were
/// kept and the old file had no ACL to carry over.
///
/// Built from the old file's ACL, or from its mode when it had none. Every
/// entry the old mask limited keeps exactly what the mask let it have, so
/// recomputing the mask grants nobody more than before. A former owner the
/// replacement could not keep is named with the old owner's rights, and a
/// former group with the old group's when it had any; an entry already
/// naming them is replaced. An id this process cannot see is never named.
fn carried_access_acl(
    replaced: &libc::stat,
    old: Option<&[AclEntry]>,
    kept: Kept,
) -> Option<Vec<AclEntry>> {
    if kept.owner && kept.group && old.is_none() {
        return None;
    }
    let mode = replaced.st_mode;
    let bits = |shift: u32| ((mode >> shift) & 0o7) as u16;
    let base = [
        AclEntry {
            tag: ACL_USER_OBJ,
            perm: bits(6),
            id: ACL_UNDEFINED_ID,
        },
        AclEntry {
            tag: ACL_GROUP_OBJ,
            perm: bits(3),
            id: ACL_UNDEFINED_ID,
        },
        AclEntry {
            tag: ACL_OTHER,
            perm: bits(0),
            id: ACL_UNDEFINED_ID,
        },
    ];
    let old = old.unwrap_or(&base);
    let old_mask = old
        .iter()
        .find(|entry| entry.tag == ACL_MASK)
        .map_or(0o7, |entry| entry.perm);
    let perm_of = |tag: u16| {
        old.iter()
            .find(|entry| entry.tag == tag)
            .map_or(0, |entry| entry.perm)
    };
    let owner_perm = perm_of(ACL_USER_OBJ);
    let group_perm = perm_of(ACL_GROUP_OBJ) & old_mask;
    let visible = |id: u32| id != ACL_UNDEFINED_ID && id != OVERFLOW_ID;

    let mut acl: Vec<AclEntry> = old
        .iter()
        .filter(|entry| entry.tag != ACL_MASK)
        .filter(|entry| !matches!(entry.tag, ACL_USER | ACL_GROUP) || visible(entry.id))
        .filter(|entry| !(entry.tag == ACL_USER && !kept.owner && entry.id == replaced.st_uid))
        .filter(|entry| !(entry.tag == ACL_GROUP && !kept.group && entry.id == replaced.st_gid))
        .map(|entry| {
            if masked(entry) {
                AclEntry {
                    perm: entry.perm & old_mask,
                    ..*entry
                }
            } else {
                *entry
            }
        })
        .collect();
    if !kept.owner && visible(replaced.st_uid) {
        acl.push(AclEntry {
            tag: ACL_USER,
            perm: owner_perm,
            id: replaced.st_uid,
        });
    }
    if !kept.group && group_perm != 0 && visible(replaced.st_gid) {
        acl.push(AclEntry {
            tag: ACL_GROUP,
            perm: group_perm,
            id: replaced.st_gid,
        });
    }
    if acl
        .iter()
        .any(|entry| entry.tag == ACL_USER || entry.tag == ACL_GROUP)
    {
        let mask = acl
            .iter()
            .filter(|entry| masked(entry))
            .fold(0, |mask, entry| mask | entry.perm);
        acl.push(AclEntry {
            tag: ACL_MASK,
            perm: mask,
            id: ACL_UNDEFINED_ID,
        });
    }
    // The kernel refuses an ACL out of this order.
    acl.sort_by_key(|entry| (entry.tag, entry.id));
    Some(acl)
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
    // Invalid input rather than a conflict: a conflict tells the caller to
    // read again and retry, and a retry meets the same symlink or folder
    // every time (#630).
    match metadata_at_no_follow(parent, filename) {
        Ok(metadata) if metadata.st_mode & libc::S_IFMT == libc::S_IFLNK => {
            Err(WriteError::InvalidInput(format!(
                "refusing to replace symlink destination '{}'",
                bounded_write_name(path)
            )))
        }
        Ok(metadata) if metadata.st_mode & libc::S_IFMT != libc::S_IFREG => {
            Err(WriteError::InvalidInput(format!(
                "refusing to replace non-file destination '{}'",
                bounded_write_name(path)
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

/// Every in-flight write parks its bytes at this prefix beside the
/// destination, so one name covers creating them and recognising them.
fn temporary_sidecar_prefix(filename: &str) -> String {
    format!(".{filename}.hatchdoor-tmp-")
}

fn create_unique_temporary_file(
    parent: &fs::File,
    filename: &CString,
    mode: libc::mode_t,
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
        let tmp = CString::new(format!("{}{suffix}", temporary_sidecar_prefix(filename)))
            .expect("generated name has no NUL");
        match create_file_at_no_follow(parent, &tmp, mode) {
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

/// Why a move primitive failed, split by whether the file had already
/// reached its destination when it did.
enum MoveError {
    /// Nothing moved, or the move was undone: the source is where it was.
    NotMoved(WriteError),
    /// The move committed and stays committed; a step after the commit
    /// failed. The caller's rollback owns moving it back.
    Landed(WriteError),
}

impl MoveError {
    fn into_write_error(self) -> WriteError {
        match self {
            Self::NotMoved(error) | Self::Landed(error) => error,
        }
    }
}

impl From<WriteError> for MoveError {
    fn from(error: WriteError) -> Self {
        Self::NotMoved(error)
    }
}

#[cfg(test)]
fn move_file_if_unchanged_with_after_exchange(
    source: &Path,
    destination: &Path,
    expected_content_hash: &str,
    after_exchange: impl FnOnce(),
) -> Result<(), WriteError> {
    try_move_file_if_unchanged(source, destination, expected_content_hash, after_exchange)
        .map_err(MoveError::into_write_error)
}

fn try_move_file_if_unchanged(
    source: &Path,
    destination: &Path,
    expected_content_hash: &str,
    after_exchange: impl FnOnce(),
) -> Result<(), MoveError> {
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
        if exchange_unavailable(commit_directory(destination), &error) {
            return move_checked_without_exchange(
                &source_parent,
                &source_name,
                &destination_parent,
                &destination_name,
                source,
                Some(expected_content_hash),
                after_exchange,
            )
            .map_err(MoveError::NotMoved);
        }
        let _ = remove_move_gate(&destination_parent, &destination_name);
        return Err(MoveError::NotMoved(WriteError::Io(format!(
            "failed to atomically move '{}' to '{}': {error}",
            source.display(),
            destination.display()
        ))));
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
        return Err(MoveError::NotMoved(match moved {
            Ok(prior) => WriteError::Conflict(format!(
                "note changed since it was read: expected {}, found {}",
                expected_content_hash.trim(),
                content_hash(&prior)
            )),
            Err(error) => WriteError::Conflict(format!(
                "refusing to move unsafe source '{}': {error}",
                source.display()
            )),
        }));
    }
    remove_move_gate(&source_parent, &source_name).map_err(|error| {
        MoveError::Landed(WriteError::Io(format!(
            "failed to finalize move from '{}': {error}",
            source.display()
        )))
    })?;
    let _ = source_parent.sync_all();
    let _ = destination_parent.sync_all();
    Ok(())
}

pub(super) fn move_file_no_follow(source: &Path, destination: &Path) -> Result<(), WriteError> {
    try_move_file_no_follow(source, destination).map_err(MoveError::into_write_error)
}

fn try_move_file_no_follow(source: &Path, destination: &Path) -> Result<(), MoveError> {
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
        if exchange_unavailable(commit_directory(destination), &error) {
            return move_checked_without_exchange(
                &source_parent,
                &source_name,
                &destination_parent,
                &destination_name,
                source,
                None,
                || {},
            )
            .map_err(MoveError::NotMoved);
        }
        let _ = remove_move_gate(&destination_parent, &destination_name);
        return Err(MoveError::NotMoved(WriteError::Io(format!(
            "failed to atomically move '{}' to '{}': {error}",
            source.display(),
            destination.display()
        ))));
    }
    if let Err(error) = assert_regular_file_at(&destination_parent, &destination_name) {
        restore_move_from_gate(
            &source_parent,
            &source_name,
            &destination_parent,
            &destination_name,
            source,
        )?;
        return Err(MoveError::NotMoved(WriteError::Conflict(format!(
            "refusing to move unsafe source '{}': {error}",
            source.display()
        ))));
    }
    remove_move_gate(&source_parent, &source_name).map_err(|error| {
        MoveError::Landed(WriteError::Io(format!(
            "failed to finalize move from '{}': {error}",
            source.display()
        )))
    })?;
    let _ = source_parent.sync_all();
    let _ = destination_parent.sync_all();
    Ok(())
}

/// Move a file on a filesystem that cannot exchange two names, with the
/// destination name already reserved by its gate directory.
///
/// The exchange-based move commits first and inspects afterwards, because an
/// exchange can always be undone. Without one there is nothing to undo with,
/// so the order reverses: verify the source, release the reservation, rename.
/// The caller sees the same conflicts it always did — an occupied destination
/// was already refused when the gate was created, a stale expected hash and an
/// irregular source are refused here with their existing wording. Between
/// releasing the gate and the rename the destination name is briefly
/// unreserved, which is this path's share of the race the exchange closed
/// (ADR-26).
fn move_checked_without_exchange(
    source_parent: &fs::File,
    source_name: &CString,
    destination_parent: &fs::File,
    destination_name: &CString,
    source: &Path,
    expected_content_hash: Option<&str>,
    in_commit_window: impl FnOnce(),
) -> Result<(), WriteError> {
    let verified = match expected_content_hash {
        Some(expected) => match read_regular_file_at(source_parent, source_name) {
            Ok(content) if content_hash(&content) == expected.trim() => Ok(()),
            Ok(content) => Err(WriteError::Conflict(format!(
                "note changed since it was read: expected {}, found {}",
                expected.trim(),
                content_hash(&content)
            ))),
            Err(error) => Err(WriteError::Conflict(format!(
                "refusing to move unsafe source '{}': {error}",
                source.display()
            ))),
        },
        None => assert_regular_file_at(source_parent, source_name).map_err(|error| {
            WriteError::Conflict(format!(
                "refusing to move unsafe source '{}': {error}",
                source.display()
            ))
        }),
    };
    if let Err(error) = verified {
        let _ = remove_move_gate(destination_parent, destination_name);
        return Err(error);
    }
    remove_move_gate(destination_parent, destination_name).map_err(|error| {
        WriteError::Io(format!(
            "failed to release the reserved destination for '{}': {error}",
            source.display()
        ))
    })?;
    in_commit_window();
    rename_at(
        source_parent,
        source_name,
        destination_parent,
        destination_name,
    )?;
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

/// Undo an exchange that must not stand. When the undo itself fails, the
/// file is still at the destination, so that failure is [`MoveError::Landed`];
/// once it is back at its source, a gate that will not clear is not.
fn restore_move_from_gate(
    source_parent: &fs::File,
    source_name: &CString,
    destination_parent: &fs::File,
    destination_name: &CString,
    source: &Path,
) -> Result<(), MoveError> {
    rename_exchange(
        source_parent,
        source_name,
        destination_parent,
        destination_name,
    )
    .map_err(|error| {
        MoveError::Landed(WriteError::Io(format!(
            "failed to restore unsafe move from '{}': {error}",
            source.display()
        )))
    })?;
    remove_move_gate(destination_parent, destination_name).map_err(|error| {
        MoveError::NotMoved(WriteError::Io(format!(
            "failed to remove move gate for '{}': {error}",
            source.display()
        )))
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

    // Each directory on the way is opened as a bare path handle, which needs
    // only permission to pass through it, not to list it. A Vault under a
    // home folder at 0711 owned by someone else otherwise failed every write
    // while reporting itself writable (#630). Only the parent the write
    // happens in is opened for reading, since its handle is synced.
    let root = CString::new("/").expect("literal has no NUL");
    let mut current = open_path_dir_at(libc::AT_FDCWD, &root)?;
    for component in parent.components() {
        use std::path::Component;
        match component {
            Component::RootDir => {}
            Component::Normal(part) => {
                let part = CString::new(part.as_encoded_bytes()).map_err(|_| {
                    std::io::Error::new(std::io::ErrorKind::InvalidInput, "path component has NUL")
                })?;
                current = open_path_dir_at(current.as_raw_fd(), &part)?;
            }
            _ => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "write path has unsupported component",
                ));
            }
        }
    }
    let itself = CString::new(".").expect("literal has no NUL");
    Ok((open_dir_at(current.as_raw_fd(), &itself)?, filename))
}

fn open_dir_at(dirfd: libc::c_int, name: &CString) -> Result<fs::File, std::io::Error> {
    open_dir_with(dirfd, name, libc::O_RDONLY)
}

/// A handle that can only be walked through, never read: enough to resolve
/// the next name relative to it.
fn open_path_dir_at(dirfd: libc::c_int, name: &CString) -> Result<fs::File, std::io::Error> {
    open_dir_with(dirfd, name, libc::O_PATH)
}

fn open_dir_with(
    dirfd: libc::c_int,
    name: &CString,
    access: libc::c_int,
) -> Result<fs::File, std::io::Error> {
    let fd = unsafe {
        libc::openat(
            dirfd,
            name.as_ptr(),
            access | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    file_from_fd(fd)
}

/// Create `name` exclusively, with `mode` as the permissions requested of
/// the kernel, before the umask or a default ACL narrows them.
fn create_file_at_no_follow(
    parent: &fs::File,
    name: &CString,
    mode: libc::mode_t,
) -> Result<fs::File, std::io::Error> {
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            libc::c_uint::from(mode),
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
    rename_flagged_at(from_parent, from, to_parent, to, RenameFlag::Exchange)
}

/// Whether this failed exchange failed because the filesystem cannot exchange
/// at all, which is the one case that earns the non-atomic path below.
///
/// `directory` is where the exchange was attempted; a bare `EINVAL` is not
/// enough on its own, because misuse of the syscall reports the same errno
/// (ADR-26).
fn exchange_unavailable(directory: &Path, error: &std::io::Error) -> bool {
    crate::rename_flags::flag_unavailable(directory, RenameFlag::Exchange, error)
}

/// The directory a write's commit happens in, for the capability question
/// above. Every caller here has already proved the path has a parent.
fn commit_directory(path: &Path) -> &Path {
    path.parent().unwrap_or(path)
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

    /// Every write sidecar left in `directory`, for any destination.
    fn sidecars_in(directory: &Path) -> Vec<String> {
        fs::read_dir(directory)
            .expect("read directory")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with('.') && name.contains(".hatchdoor-tmp-"))
            .collect()
    }

    /// #630: the sidecar was written before the destination was checked, and
    /// a refusal left it behind, where Git committed it. A retry, which the
    /// old `Conflict` invited, left another.
    #[cfg(unix)]
    #[test]
    fn a_refused_replace_leaves_no_sidecar_and_is_not_retryable() {
        use std::os::unix::fs::symlink;

        let dir = tempdir().expect("tempdir");
        let outside = tempdir().expect("outside");
        let target = outside.path().join("target.md");
        fs::write(&target, "outside").expect("outside file");
        symlink(&target, dir.path().join("Link.md")).expect("symlink");
        fs::create_dir(dir.path().join("Folder.md")).expect("folder");

        for name in ["Link.md", "Folder.md"] {
            let error = atomic_write(&dir.path().join(name), "LEAKED")
                .expect_err("a non-file destination must be refused");
            assert!(
                matches!(&error, WriteError::InvalidInput(message) if !message.contains(&*dir.path().to_string_lossy())),
                "{name}: a refusal a retry cannot fix is invalid input, named without the host path: {error:?}"
            );
        }
        assert_eq!(sidecars_in(dir.path()), Vec::<String>::new());
        assert_eq!(fs::read_to_string(&target).unwrap(), "outside");
    }

    /// A rename gives the note a new inode, which kept the 0600 the sidecar
    /// was created with until #630.
    #[cfg(unix)]
    #[test]
    fn a_replaced_note_keeps_its_mode_and_a_new_one_follows_the_umask() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().expect("tempdir");
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o7777;

        let shared = dir.path().join("Shared.md");
        fs::write(&shared, "original").expect("note");
        fs::set_permissions(&shared, fs::Permissions::from_mode(0o664)).expect("chmod");
        atomic_write(&shared, "replaced").expect("replace");
        assert_eq!(mode(&shared), 0o664, "a plain replace keeps the mode");
        atomic_write_if_unchanged(&shared, "edited", &content_hash("replaced"))
            .expect("conditional write");
        assert_eq!(mode(&shared), 0o664, "a conditional write keeps the mode");

        // What any other program creating a file here gets, umask and all.
        let reference = dir.path().join("reference");
        fs::File::create(&reference).expect("reference file");
        let created = dir.path().join("Created.md");
        atomic_create(&created, "new").expect("create");
        assert_eq!(mode(&created), mode(&reference));
        let written = dir.path().join("Written.md");
        atomic_write(&written, "new").expect("write a new note");
        assert_eq!(mode(&written), mode(&reference));
    }

    /// A setuid file replaced with a caller's bytes must not run them with
    /// its owner's rights.
    #[cfg(unix)]
    #[test]
    fn a_replaced_file_does_not_keep_set_id_bits() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().expect("tempdir");
        let tool = dir.path().join("tool.png");
        fs::write(&tool, "original").expect("file");
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o6755)).expect("chmod");

        atomic_write(&tool, "caller bytes").expect("replace");

        assert_eq!(
            fs::metadata(&tool).unwrap().permissions().mode() & 0o7777,
            0o755
        );
    }

    /// Without the right to change the owner, a note still keeps a group
    /// this process belongs to, so a folder shared through a group stays
    /// writable by it.
    #[cfg(unix)]
    #[test]
    fn a_replaced_note_keeps_a_group_the_process_belongs_to() {
        use std::os::unix::fs::MetadataExt;

        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let primary = unsafe { libc::getegid() };
        let mut groups = vec![0 as libc::gid_t; 64];
        let count = unsafe { libc::getgroups(64, groups.as_mut_ptr()) };
        let Some(other) = groups[..count.max(0) as usize]
            .iter()
            .copied()
            .find(|group| *group != primary)
        else {
            return; // No second group to keep.
        };
        let dir = tempdir().expect("tempdir");
        let note = dir.path().join("Shared.md");
        fs::write(&note, "original").expect("note");
        std::os::unix::fs::chown(&note, None, Some(other)).expect("chgrp");

        atomic_write(&note, "edited").expect("write");

        assert_eq!(fs::metadata(&note).unwrap().gid(), other);
    }

    /// The walk to the note's folder needs only permission to pass through
    /// the folders above it, as any other program writing there does. Root
    /// passes through everything, so there is nothing to show as root.
    #[cfg(unix)]
    #[test]
    fn a_write_reaches_a_folder_under_one_it_may_not_list() {
        use std::os::unix::fs::PermissionsExt;

        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let dir = tempdir().expect("tempdir");
        let home = dir.path().join("home");
        let vault = home.join("vault");
        fs::create_dir_all(&vault).expect("vault");
        fs::set_permissions(&home, fs::Permissions::from_mode(0o311)).expect("seal home");

        let outcome = atomic_write(&vault.join("Note.md"), "written");
        fs::set_permissions(&home, fs::Permissions::from_mode(0o755)).expect("reopen home");

        outcome.expect("a write under a folder that can be passed through");
        assert_eq!(
            fs::read_to_string(vault.join("Note.md")).unwrap(),
            "written"
        );
    }

    /// Run as root, the write can hand the note back to its owner. As
    /// anyone else it cannot, and there is nothing to check.
    #[cfg(unix)]
    #[test]
    fn a_replaced_note_keeps_its_owner_where_the_process_may_set_it() {
        use std::os::unix::fs::MetadataExt;

        if unsafe { libc::geteuid() } != 0 {
            return;
        }
        let dir = tempdir().expect("tempdir");
        let note = dir.path().join("Owned.md");
        fs::write(&note, "original").expect("note");
        std::os::unix::fs::chown(&note, Some(1000), Some(1000)).expect("chown");

        atomic_write_if_unchanged(&note, "edited", &content_hash("original")).expect("write");

        let metadata = fs::metadata(&note).unwrap();
        assert_eq!((metadata.uid(), metadata.gid()), (1000, 1000));
    }

    fn stat_of(uid: u32, gid: u32, mode: libc::mode_t) -> libc::stat {
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        stat.st_uid = uid;
        stat.st_gid = gid;
        stat.st_mode = libc::S_IFREG | mode;
        stat
    }

    fn acl_entry(tag: u16, perm: u16, id: u32) -> AclEntry {
        AclEntry { tag, perm, id }
    }

    const KEPT: Kept = Kept {
        owner: true,
        group: true,
    };
    const LOST: Kept = Kept {
        owner: false,
        group: false,
    };

    /// #661: a note owned by uid 1000, replaced by Hatchdoor's uid 65532,
    /// names 1000 and its group with the rights the owner bits and group
    /// bits gave them, so they keep that access.
    #[test]
    fn a_former_owner_and_group_are_named_with_their_old_rights() {
        let u = ACL_UNDEFINED_ID;
        assert_eq!(
            carried_access_acl(&stat_of(1000, 1000, 0o644), None, LOST),
            Some(vec![
                acl_entry(ACL_USER_OBJ, 6, u),
                acl_entry(ACL_USER, 6, 1000),
                acl_entry(ACL_GROUP_OBJ, 4, u),
                acl_entry(ACL_GROUP, 4, 1000),
                acl_entry(ACL_MASK, 6, u),
                acl_entry(ACL_OTHER, 4, u),
            ])
        );
        // A group with no rights needs no entry naming it.
        assert_eq!(
            carried_access_acl(&stat_of(1000, 1000, 0o600), None, LOST),
            Some(vec![
                acl_entry(ACL_USER_OBJ, 6, u),
                acl_entry(ACL_USER, 6, 1000),
                acl_entry(ACL_GROUP_OBJ, 0, u),
                acl_entry(ACL_MASK, 6, u),
                acl_entry(ACL_OTHER, 0, u),
            ])
        );
    }

    /// In a user namespace, as under rootless Podman, a note owned by a host
    /// user the container cannot see reports 65534, and an ACL entry naming
    /// one reads back as 65534 or as no id at all. Naming 65534 would grant
    /// whoever it maps to, and a second unnamed entry makes the kernel
    /// refuse the whole list.
    #[test]
    fn an_id_the_process_cannot_see_is_never_named() {
        let u = ACL_UNDEFINED_ID;
        let old = vec![
            acl_entry(ACL_USER_OBJ, 6, u),
            acl_entry(ACL_USER, 6, 2000),
            acl_entry(ACL_USER, 6, OVERFLOW_ID),
            acl_entry(ACL_USER, 4, u),
            acl_entry(ACL_GROUP_OBJ, 4, u),
            acl_entry(ACL_MASK, 6, u),
            acl_entry(ACL_OTHER, 0, u),
        ];
        assert_eq!(
            carried_access_acl(&stat_of(OVERFLOW_ID, OVERFLOW_ID, 0o640), Some(&old), LOST),
            Some(vec![
                acl_entry(ACL_USER_OBJ, 6, u),
                acl_entry(ACL_USER, 6, 2000),
                acl_entry(ACL_GROUP_OBJ, 4, u),
                acl_entry(ACL_MASK, 6, u),
                acl_entry(ACL_OTHER, 0, u),
            ])
        );
    }

    /// Kept as root or as the note's own owner, a note with no ACL gets none
    /// written, as after #630.
    #[test]
    fn a_kept_owner_with_no_old_acl_needs_no_acl() {
        assert_eq!(
            carried_access_acl(&stat_of(1000, 1000, 0o644), None, KEPT),
            None
        );
    }

    /// Once the note belongs to Hatchdoor, the next write keeps its owner and
    /// must carry the old ACL over, or the former owner's entry is lost.
    #[test]
    fn an_old_acl_is_carried_over_when_the_owner_was_kept() {
        let u = ACL_UNDEFINED_ID;
        let old = vec![
            acl_entry(ACL_USER_OBJ, 6, u),
            acl_entry(ACL_USER, 6, 1000),
            acl_entry(ACL_GROUP_OBJ, 4, u),
            acl_entry(ACL_MASK, 6, u),
            acl_entry(ACL_OTHER, 4, u),
        ];
        assert_eq!(
            carried_access_acl(&stat_of(65532, 65532, 0o664), Some(&old), KEPT),
            Some(old)
        );
    }

    /// An entry the old mask limited keeps only what it had in effect, the
    /// former owner's existing entry is replaced rather than duplicated, and
    /// the kept group gets no named entry.
    #[test]
    fn carried_entries_keep_their_effective_rights_only() {
        let u = ACL_UNDEFINED_ID;
        let old = vec![
            acl_entry(ACL_USER_OBJ, 6, u),
            acl_entry(ACL_USER, 7, 1000),
            acl_entry(ACL_USER, 7, 2000),
            acl_entry(ACL_GROUP_OBJ, 6, u),
            acl_entry(ACL_MASK, 4, u),
            acl_entry(ACL_OTHER, 0, u),
        ];
        assert_eq!(
            carried_access_acl(
                &stat_of(1000, 1000, 0o640),
                Some(&old),
                Kept {
                    owner: false,
                    group: true
                }
            ),
            Some(vec![
                acl_entry(ACL_USER_OBJ, 6, u),
                acl_entry(ACL_USER, 6, 1000),
                acl_entry(ACL_USER, 4, 2000),
                acl_entry(ACL_GROUP_OBJ, 4, u),
                acl_entry(ACL_MASK, 6, u),
                acl_entry(ACL_OTHER, 0, u),
            ])
        );
    }

    #[test]
    fn an_acl_survives_encoding_and_a_foreign_version_is_refused() {
        let acl = vec![
            acl_entry(ACL_USER_OBJ, 6, ACL_UNDEFINED_ID),
            acl_entry(ACL_USER, 6, 1000),
        ];
        assert_eq!(decode_acl(&encode_acl(&acl)), Some(acl));
        let mut foreign = encode_acl(&[]);
        foreign[0] = 1;
        assert_eq!(decode_acl(&foreign), None);
        assert_eq!(decode_acl(&[2, 0, 0]), None);
    }

    /// On a filesystem with ACLs, an entry on a note survives a replace.
    /// Without them there is nothing to check: the write itself is covered
    /// by every other test.
    #[test]
    fn a_replaced_note_keeps_its_access_acl() {
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().expect("tempdir");
        let note = dir.path().join("Shared.md");
        fs::write(&note, "original").expect("note");
        let name = CString::new(note.as_os_str().as_bytes()).unwrap();
        let u = ACL_UNDEFINED_ID;
        let acl = vec![
            acl_entry(ACL_USER_OBJ, 6, u),
            acl_entry(ACL_USER, 6, 4242),
            acl_entry(ACL_GROUP_OBJ, 4, u),
            acl_entry(ACL_MASK, 6, u),
            acl_entry(ACL_OTHER, 4, u),
        ];
        let value = encode_acl(&acl);
        let set = unsafe {
            libc::setxattr(
                name.as_ptr(),
                ACL_ACCESS_XATTR.as_ptr(),
                value.as_ptr().cast(),
                value.len(),
                0,
            )
        };
        if set != 0 {
            return; // No ACLs here.
        }

        atomic_write_if_unchanged(&note, "edited", &content_hash("original")).expect("write");
        assert_eq!(fs::read_to_string(&note).unwrap(), "edited");
        let (parent, filename) = open_parent_dir_no_follow(&note).unwrap();
        let replaced = metadata_at_no_follow(&parent, &filename).unwrap();
        assert_eq!(read_access_acl_at(&parent, &filename, &replaced), Some(acl));

        // Reading the list needs no read permission on the note, as when
        // Hatchdoor may replace an attachment it may not read.
        fs::set_permissions(&note, fs::Permissions::from_mode(0o200)).expect("chmod");
        let unreadable_acl = vec![
            acl_entry(ACL_USER_OBJ, 2, u),
            acl_entry(ACL_USER, 6, 4242),
            acl_entry(ACL_GROUP_OBJ, 0, u),
            acl_entry(ACL_MASK, 6, u),
            acl_entry(ACL_OTHER, 0, u),
        ];
        let set = unsafe {
            libc::setxattr(
                name.as_ptr(),
                ACL_ACCESS_XATTR.as_ptr(),
                encode_acl(&unreadable_acl).as_ptr().cast(),
                encode_acl(&unreadable_acl).len(),
                0,
            )
        };
        assert_eq!(set, 0, "{}", std::io::Error::last_os_error());
        atomic_write(&note, "replaced unread").expect("replace");

        let replaced = metadata_at_no_follow(&parent, &filename).unwrap();
        assert_eq!(
            read_access_acl_at(&parent, &filename, &replaced),
            Some(unreadable_acl)
        );
        fs::set_permissions(&note, fs::Permissions::from_mode(0o600)).expect("chmod");
        assert_eq!(fs::read_to_string(&note).unwrap(), "replaced unread");
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

    #[test]
    fn a_create_refuses_a_file_that_appeared_after_the_existence_check() {
        let dir = tempdir().expect("tempdir");
        let note = dir.path().join("New.md");

        let result = atomic_create_with_before_commit(&note, "hatchdoor", || {
            fs::write(&note, "created outside Hatchdoor").expect("external create");
        });

        assert!(matches!(result, Err(WriteError::Conflict(_))), "{result:?}");
        assert_eq!(
            fs::read_to_string(&note).unwrap(),
            "created outside Hatchdoor",
            "a create must never replace a file it did not write"
        );
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(
            names,
            vec![std::ffi::OsString::from("New.md")],
            "no temp file left"
        );
    }

    #[test]
    fn a_create_on_a_free_name_writes_it_with_or_without_rename_flags() {
        for forced in [false, true] {
            let dir = tempdir().expect("tempdir");
            if forced {
                crate::rename_flags::force_unsupported_for_tests(dir.path());
            }
            let note = dir.path().join("New.md");
            atomic_create(&note, "hatchdoor").expect("create on a free name");
            assert_eq!(fs::read_to_string(&note).unwrap(), "hatchdoor");
        }
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

    /// The in-flight temporary sidecar beside `note`.
    fn find_write_sidecar(note: &Path) -> PathBuf {
        let directory = note.parent().expect("note parent");
        let prefix =
            temporary_sidecar_prefix(&note.file_name().expect("note name").to_string_lossy());
        fs::read_dir(directory)
            .expect("read vault directory")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with(&prefix))
            })
            .expect("an in-flight write must have a temporary sidecar")
    }

    /// Removes that sidecar, standing in for an external process cleaning the
    /// Vault directory mid-write.
    fn remove_write_sidecar(note: &Path) {
        fs::remove_file(find_write_sidecar(note)).expect("remove sidecar");
    }

    /// The commit exchange has already published the new bytes by the time the
    /// displaced content is read back. If that read fails and the swap back
    /// cannot be undone, the write stands, and saying so is the whole point:
    /// reporting a plain I/O failure told the caller its write had not landed
    /// while the new content sat committed in the Vault.
    #[test]
    fn atomic_write_reports_recovery_when_a_committed_exchange_cannot_be_undone() {
        let dir = tempdir().expect("tempdir");
        let note = dir.path().join("Note.md");
        fs::write(&note, "original").expect("original note");
        let expected = content_hash("original");

        let error =
            atomic_write_if_unchanged_with_after_exchange(&note, "agent edit", &expected, || {
                remove_write_sidecar(&note)
            })
            .expect_err("an unverifiable committed write must not report plain success");

        assert_eq!(
            fs::read_to_string(&note).unwrap(),
            "agent edit",
            "the exchange already committed, so the new content is what is on disk"
        );
        let recovery = error.recovery_message().unwrap_or_else(|| {
            panic!("a landed write that cannot be undone needs operator action, got: {error:?}")
        });
        assert!(
            recovery.contains("Note.md"),
            "the operator has to be told which note: {recovery}"
        );
        assert!(
            !recovery.contains(dir.path().to_string_lossy().as_ref()),
            "a recovery message reaches clients unsanitized and must not carry the host path: {recovery}"
        );
    }

    /// The other half of the same window: when the swap back *does* succeed,
    /// the note must be left exactly as it was and the sidecar must not
    /// survive as litter in the Vault.
    #[cfg(unix)]
    #[test]
    fn atomic_write_restores_the_note_and_clears_its_sidecar_when_the_undo_succeeds() {
        use std::os::unix::fs::PermissionsExt;

        // Root reads through the mode bits, so the read-back would succeed
        // and this window would never open.
        if unsafe { libc::geteuid() } == 0 {
            return;
        }

        let dir = tempdir().expect("tempdir");
        let note = dir.path().join("Note.md");
        fs::write(&note, "original").expect("original note");
        let expected = content_hash("original");

        atomic_write_if_unchanged_with_after_exchange(&note, "agent edit", &expected, || {
            // Unreadable rather than absent, so the read fails with the
            // displaced file still there for the undo to swap back.
            let sidecar = find_write_sidecar(&note);
            fs::set_permissions(&sidecar, fs::Permissions::from_mode(0o000)).expect("seal sidecar");
        })
        .expect_err("a write whose displaced note cannot be read back must fail");
        // The restored file is the sealed one, so it comes back with the mode
        // the test set. Reopen it before reading its contents.
        fs::set_permissions(&note, fs::Permissions::from_mode(0o644)).expect("reopen note");
        assert_eq!(
            fs::read_to_string(&note).unwrap(),
            "original",
            "a write whose verification failed must leave the note untouched"
        );
        let leftovers = fs::read_dir(dir.path())
            .expect("read vault directory")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(&temporary_sidecar_prefix("Note.md")))
            .collect::<Vec<_>>();
        assert!(
            leftovers.is_empty(),
            "an undone write must not leave its sidecar behind: {leftovers:?}"
        );
    }

    /// The neighbouring branch: the read-back succeeds and reports a genuine
    /// concurrent change, but the restore that would undo the commit cannot
    /// run. The new content stands over someone else's edit, which is the
    /// same state as an unverifiable write and is reported the same way.
    #[test]
    fn atomic_write_reports_recovery_when_a_concurrent_change_cannot_be_restored() {
        let dir = tempdir().expect("tempdir");
        let note = dir.path().join("Note.md");
        fs::write(&note, "original").expect("original note");
        let stale = content_hash("what the caller last read");

        let error =
            atomic_write_if_unchanged_with_after_exchange(&note, "agent edit", &stale, || {
                fs::remove_file(&note).expect("external delete of the committed note")
            })
            .expect_err("a commit that cannot be restored must not report a plain conflict");

        let recovery = error.recovery_message().unwrap_or_else(|| {
            panic!("an unrestorable commit needs operator action, got: {error:?}")
        });
        assert!(
            recovery.contains("Note.md"),
            "the operator has to be told which note: {recovery}"
        );
    }

    /// A verified write is finished, whatever happens to its sidecar
    /// afterwards. Failing here reported a correct, committed, hash-checked
    /// save as an error.
    #[cfg(unix)]
    #[test]
    fn atomic_write_succeeds_when_its_verified_sidecar_cannot_be_removed() {
        use std::os::unix::fs::PermissionsExt;

        // Root writes through the mode bits, so the unlink would not fail and
        // this window would never open.
        if unsafe { libc::geteuid() } == 0 {
            return;
        }

        let dir = tempdir().expect("tempdir");
        let note = dir.path().join("Note.md");
        fs::write(&note, "original").expect("original note");
        let expected = content_hash("original");

        let outcome =
            atomic_write_if_unchanged_with_after_exchange(&note, "agent edit", &expected, || {
                // Read and search stay open so the verification still runs;
                // only the unlink that follows it is refused.
                fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o555))
                    .expect("seal vault directory");
            });
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).expect("reopen");

        outcome.expect("a verified write must not fail over an unremovable sidecar");
        assert_eq!(fs::read_to_string(&note).unwrap(), "agent edit");
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

        atomic_write_inner(
            &note,
            b"safe\n",
            Commit::Replace,
            None,
            || {
                fs::rename(&notes, &original_parent).expect("swap away opened parent");
                symlink(&external, &notes).expect("replace path with external symlink");
            },
            || {},
        )
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
    fn move_file_no_follow_rejects_a_path_carrying_a_parent_component() {
        // #225: the planner must normalise `..` away before it gets here. This
        // pins the syscall-level rejection that catches it if it ever does not:
        // the parent walk opens one plain name at a time, so `..` is refused
        // rather than followed. Nothing above may relax this.
        let dir = tempdir().expect("tempdir");
        let nested = dir.path().join("folder-x");
        fs::create_dir_all(&nested).expect("nested dir");
        let source = dir.path().join("asset.png");
        fs::write(&source, BINARY_ASSET).expect("source");
        let destination = nested.join("../moved.png");

        let error = move_file_no_follow(&source, &destination)
            .expect_err("a destination carrying '..' must not reach the filesystem");

        let WriteError::Io(message) = error else {
            panic!("expected an I/O error naming the unsupported component");
        };
        assert!(
            message.contains("write path has unsupported component"),
            "unexpected message: {message}"
        );
        assert!(source.exists(), "the source must be left alone");
        assert!(!dir.path().join("moved.png").exists());
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

        let mut journal = MutationJournal::new(root, None);
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

        let mut journal = MutationJournal::new(root, None);
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
    fn a_move_refused_at_its_gate_is_not_rolled_back_even_when_its_source_vanished() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        // The source was validated, then deleted by something outside
        // Hatchdoor; the destination name already holds the user's own file.
        let source = root.join("gone.png");
        let destination = root.join("taken.png");
        fs::write(&destination, "the user's own file").unwrap();

        let mut journal = MutationJournal::new(root, None);
        let cause = journal
            .move_file(MutationPhase::Asset, &source, &destination)
            .expect_err("an occupied destination refuses the move");
        assert!(matches!(cause, WriteError::Conflict(_)), "{cause:?}");
        let error = journal.rollback(cause);

        assert!(matches!(error, WriteError::Conflict(_)), "{error:?}");
        assert_eq!(
            fs::read_to_string(&destination).unwrap(),
            "the user's own file",
            "the rollback must not carry a file the move never touched"
        );
        assert!(!source.exists());
    }

    #[test]
    fn a_move_that_landed_before_its_cleanup_failed_is_kept_for_rollback() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        let source = root.join("Source.md");
        let destination = root.join("Moved.md");
        fs::write(&source, "original").unwrap();
        let expected = content_hash("original");

        let mut journal = MutationJournal::new(root, None);
        // Something lands inside the directory gate standing at the source
        // name, so the gate will not clear once the move has committed.
        let result =
            journal.move_note_with_after_exchange(&source, &destination, &expected, || {
                fs::write(source.join("stray"), "x").expect("write into the gate");
            });

        assert!(matches!(result, Err(WriteError::Io(_))), "{result:?}");
        assert_eq!(fs::read_to_string(&destination).unwrap(), "original");
        assert_eq!(
            journal.completed.len(),
            1,
            "a committed move whose cleanup failed must still be undone by the rollback"
        );
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
                original_hash: content_hash("original first\n"),
                content: "rewritten first\n".to_string(),
            },
            TextRewrite {
                path: missing,
                original_hash: content_hash(""),
                content: "rewritten second\n".to_string(),
            },
        ];
        let mut journal = MutationJournal::new(root, None);

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
        let mut journal = MutationJournal::new(root, None);
        journal
            .move_note(&note_source, &note_destination, &content_hash("note"))
            .expect("move note");
        journal
            .move_file(MutationPhase::Asset, &asset_source, &asset_destination)
            .expect("move asset");
        journal
            .apply_rewrites(vec![TextRewrite {
                path: backlink.clone(),
                original_hash: content_hash("original backlink"),
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

    /// Issue #321: the CAS expectation is the plan's hash, so a note edited
    /// at any point between the plan reading it and the loop reaching it is
    /// refused — not just one edited in the microseconds between the
    /// journal's own re-read and its rename. The loop below writes the first
    /// note, and the manual save to the *second* one lands while it does.
    #[test]
    fn mutation_journal_refuses_a_rewrite_whose_note_changed_since_the_plan_read_it() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        let first = root.join("First.md");
        let second = root.join("Second.md");
        fs::write(&first, "original first\n").expect("first");
        fs::write(&second, "original second\n").expect("second");
        let rewrites = vec![
            TextRewrite {
                path: first.clone(),
                original_hash: content_hash("original first\n"),
                content: "rewritten first\n".to_string(),
            },
            TextRewrite {
                path: second.clone(),
                original_hash: content_hash("original second\n"),
                content: "rewritten second\n".to_string(),
            },
        ];
        let mut journal = MutationJournal::new(root, None);

        let cause = journal
            .apply_rewrites_with_before_commit(rewrites, |path| {
                // A concurrent save to the note the loop has not reached yet.
                if path == first {
                    fs::write(&second, "saved by hand\n").expect("manual save");
                }
            })
            .expect_err("the second rewrite must be refused");
        let error = journal.rollback(cause);

        let WriteError::Conflict(message) = error else {
            panic!("expected rewrite conflict, got {error:?}");
        };
        assert!(message.contains("rollback succeeded"), "{message}");
        assert_eq!(
            fs::read_to_string(&second).expect("second"),
            "saved by hand\n",
            "the concurrent save must survive untouched"
        );
        assert_eq!(
            fs::read_to_string(&first).expect("first"),
            "original first\n",
            "the rewrite already applied must be rolled back"
        );
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
        let mut journal = MutationJournal::new(root, None);
        journal
            .move_note(&note_source, &note_destination, &content_hash("note"))
            .expect("move note");

        let cause = journal
            .apply_rewrites_with_before_commit(
                vec![TextRewrite {
                    path: backlink.clone(),
                    original_hash: content_hash("original backlink"),
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

    /// Issue #299: a timestamp is cosmetic next to the content, so neither
    /// reading nor applying one may fail the write it rides on.
    #[test]
    fn a_timestamp_that_cannot_be_read_or_applied_does_not_fail_the_write() {
        use std::os::unix::fs::OpenOptionsExt;

        let dir = tempdir().expect("tempdir");
        let note = dir.path().join("Note.md");
        fs::write(&note, "original\n").expect("note");

        assert_eq!(captured_modified_time(&dir.path().join("Gone.md")), None);
        atomic_write_if_unchanged_keeping_modified(
            &note,
            "rewritten\n",
            &content_hash("original\n"),
            None,
        )
        .expect("a note whose time could not be read is still rewritten");
        assert_eq!(fs::read_to_string(&note).unwrap(), "rewritten\n");

        // An O_PATH handle refuses timestamp changes, standing in for a
        // filesystem that does the same.
        let refusing = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_PATH)
            .open(&note)
            .expect("path-only handle");
        assert!(
            refusing
                .set_times(FileTimes::new().set_modified(SystemTime::UNIX_EPOCH))
                .is_err(),
            "the stand-in must actually refuse"
        );
        keep_modified_time(&refusing, &note, SystemTime::UNIX_EPOCH);
        assert_ne!(
            fs::metadata(&note).unwrap().modified().unwrap(),
            SystemTime::UNIX_EPOCH
        );
    }
}
