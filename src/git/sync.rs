use std::path::{Path, PathBuf};

use git2::{Repository, Signature};

use super::config::{GitConfig, GitMode};
use super::managed_task::ManagedGitOutcome;
use super::message::build_commit_message;
use crate::vault_work::VaultWorkError;

/// Result of the local commit phase: whether a new commit was created.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommitOutcome {
    pub committed: bool,
}

/// All errors are non-fatal to the server; they are recorded and surfaced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitError {
    /// Startup validation failed (not a repo, wrong branch, missing remote, etc.).
    Validation(String),
    /// A merge produced conflicts; the local commit was kept and not pushed.
    Conflict { files: Vec<String> },
    /// A remote-integrating merge was needed but the working tree has
    /// uncommitted manual edits to tracked files outside the write batch.
    /// Refused rather than force-overwriting them (silent data loss).
    DirtyWorkingTree { files: Vec<String> },
    /// The repository is in an operation Hatchdoor cannot prove it owns.
    /// The index, working tree, and operation metadata are deliberately left
    /// untouched for the operator to inspect and recover manually.
    ManualRecovery { state: String, reason: String },
    /// Network / auth / push failure. Retried on the next batch.
    Remote(String),
    /// Any other libgit2 failure.
    Other(String),
}

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GitError::Validation(m) => write!(f, "git validation failed: {m}"),
            GitError::Conflict { files } => {
                write!(f, "git merge conflict (not pushed): {}", files.join(", "))
            }
            GitError::DirtyWorkingTree { files } => write!(
                f,
                "git refusing to overwrite uncommitted manual edits: {}",
                files.join(", ")
            ),
            GitError::ManualRecovery { state, reason } => write!(
                f,
                "git repository is in {state} state and requires manual recovery: {reason}"
            ),
            GitError::Remote(m) => write!(f, "git remote error: {m}"),
            GitError::Other(m) => write!(f, "git error: {m}"),
        }
    }
}

impl From<git2::Error> for GitError {
    fn from(e: git2::Error) -> Self {
        GitError::Other(e.message().to_string())
    }
}

fn signature(config: &GitConfig) -> Result<Signature<'_>, GitError> {
    Signature::now(&config.author_name, &config.author_email).map_err(GitError::from)
}

/// Startup validation: vault is a repo whose root is the vault, HEAD is on the
/// configured branch, and the configured remote exists.
pub fn validate_repo(config: &GitConfig) -> Result<(), GitError> {
    let repo = Repository::open(&config.vault_path).map_err(|e| {
        GitError::Validation(format!("cannot open vault as git repo: {}", e.message()))
    })?;

    let workdir = repo
        .workdir()
        .ok_or_else(|| GitError::Validation("repository is bare".to_string()))?;
    if !same_path(workdir, &config.vault_path) {
        return Err(GitError::Validation(format!(
            "vault path {} is not the repository root {}",
            config.vault_path.display(),
            workdir.display()
        )));
    }

    let head = repo
        .head()
        .map_err(|e| GitError::Validation(format!("cannot read HEAD: {}", e.message())))?;
    if !head.is_branch() {
        return Err(GitError::Validation("HEAD is detached".to_string()));
    }
    let head_name = head.shorthand().map_err(|e| {
        GitError::Validation(format!("cannot read HEAD branch name: {}", e.message()))
    })?;
    if head_name != config.branch {
        return Err(GitError::Validation(format!(
            "HEAD is on '{}', expected configured branch '{}'",
            head_name, config.branch
        )));
    }

    repo.find_remote(&config.remote).map_err(|e| {
        GitError::Validation(format!(
            "remote '{}' not found: {}",
            config.remote,
            e.message()
        ))
    })?;

    Ok(())
}

/// Local mode needs only a non-bare repository containing the vault. Branch and
/// remote checks are deliberately remote-only: local history follows whatever
/// branch the operator has checked out.
pub fn validate_local_repo(config: &GitConfig) -> Result<(), GitError> {
    let repo = Repository::discover(&config.vault_path).map_err(|e| {
        GitError::Validation(format!("cannot open vault as git repo: {}", e.message()))
    })?;
    let workdir = repo
        .workdir()
        .ok_or_else(|| GitError::Validation("repository is bare".to_string()))?;
    let vault_path = config.vault_path.canonicalize().map_err(|error| {
        GitError::Validation(format!(
            "cannot canonicalize Vault path {}: {error}",
            config.vault_path.display()
        ))
    })?;
    let workdir = workdir.canonicalize().map_err(|error| {
        GitError::Validation(format!("cannot canonicalize repository root: {error}"))
    })?;
    if !vault_path.starts_with(&workdir) {
        return Err(GitError::Validation(format!(
            "vault path {} is not contained by repository root {}",
            vault_path.display(),
            workdir.display()
        )));
    }
    Ok(())
}

/// Run one local-history Git turn for an `ExistingGit` Vault in
/// `VaultGitMode::LocalHistory`: validate the enclosing checkout, then commit
/// whatever Vault-subtree drift has accumulated since the last turn. Never
/// contacts a remote, regardless of what the enclosing checkout's `origin`
/// might be — this is `dispatch_managed_git_turn_with`'s counterpart to
/// `run_managed_git_turn` for that source/mode combination, the concrete
/// blocking `git2` operation `VaultWorkKind::Git` executes. Must run from
/// `spawn_blocking`.
///
/// Builds its own `GitMode::Local` [`GitConfig`]: `remote`, `branch`,
/// `username`, and `token` are placeholders that `validate_local_repo` and
/// `commit_local` never read for that mode (see their doc comments), so a
/// caller here needs only the Vault's resolved path and commit identity —
/// not the full settings-derived `GitConfig` the retired single-Vault lane
/// built from `HATCHDOOR_GIT_*` configuration.
pub fn run_local_history_git_turn(
    vault_path: PathBuf,
    author_name: String,
    author_email: String,
) -> Result<ManagedGitOutcome, VaultWorkError> {
    let config = GitConfig {
        vault_path,
        mode: GitMode::Local,
        remote: String::new(),
        branch: String::new(),
        username: String::new(),
        token: String::new(),
        debounce_seconds: 0,
        author_name,
        author_email,
    };
    validate_local_repo(&config).map_err(classify_local_history_error)?;
    let message = build_commit_message(&[]);
    let outcome = commit_local(&config, &[], &message).map_err(classify_local_history_error)?;
    Ok(if outcome.committed {
        ManagedGitOutcome::Synchronized
    } else {
        ManagedGitOutcome::UpToDate
    })
}

/// Classify a [`GitError`] from [`run_local_history_git_turn`] into a
/// redacted [`VaultWorkError`], keeping the transient/non-transient split
/// the retired single-Vault task used (#185): `Remote`/`Other` are worth an
/// automatic retry, everything else needs a human or the enclosing
/// checkout's state to change first.
///
/// Only `Validation` and `Other` are actually reachable from this function's
/// callee pair. #185 deleted the fetch/integrate/push half of this module,
/// and with it the merge-marker recovery `commit_local` used to run, so
/// `Conflict`, `DirtyWorkingTree`, `Remote`, and `ManualRecovery` have no
/// producer left on this path. All four keep an arm rather than being
/// dropped: `GitError` is the shared error of a module `init_local_repo` and
/// `validate_repo` also raise, and its variants are not this function's to
/// narrow.
fn classify_local_history_error(error: GitError) -> VaultWorkError {
    let code = match &error {
        GitError::Validation(_) => "existing_git_local_history_validation_failed",
        GitError::Conflict { .. } => "existing_git_local_history_conflict",
        GitError::DirtyWorkingTree { .. } => "existing_git_local_history_dirty_working_tree",
        GitError::ManualRecovery { .. } => "existing_git_local_history_manual_recovery_required",
        GitError::Remote(_) => "existing_git_local_history_remote_unexpected",
        GitError::Other(_) => "existing_git_local_history_git_error",
    };
    let retryable = matches!(error, GitError::Remote(_) | GitError::Other(_));
    VaultWorkError::new(code, error.to_string(), retryable)
}

/// Initialise a vault for explicitly-confirmed local versioning. The ignore
/// entries keep Hatchdoor's disposable cache database and durable settings
/// file out of the user's Markdown history — derived from the *actual
/// configured paths*, not a hardcoded guess, and only when those paths live
/// inside the vault. Appends to an existing `.gitignore` rather than skipping
/// it, so a vault that already has one still gets the generated files excluded
/// without hiding adjacent user content (issue #61).
pub fn init_local_repo(
    config: &GitConfig,
    cache_db_path: &Path,
    settings_file_path: &Path,
) -> Result<(), GitError> {
    let git_path = config.vault_path.join(".git");
    if git_path.exists() {
        return Err(GitError::Validation(format!(
            "cannot initialize local versioning because '{}' already exists",
            git_path.display()
        )));
    }
    let ignore_path = config.vault_path.join(".gitignore");
    let prior_ignore = match std::fs::read(&ignore_path) {
        Ok(contents) => Some(contents),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(GitError::Other(format!(
                "cannot read existing .gitignore before local initialization: {error}"
            )));
        }
    };
    let repo = Repository::init(&config.vault_path)?;
    drop(repo);
    let initialized = (|| {
        ensure_gitignore_entries(&config.vault_path, cache_db_path, settings_file_path)
            .map_err(|error| GitError::Other(format!("cannot write .gitignore: {error}")))?;
        validate_local_repo(config)
    })();
    if let Err(error) = initialized {
        let ignore_restore = match prior_ignore {
            Some(contents) => std::fs::write(&ignore_path, contents),
            None => match std::fs::remove_file(&ignore_path) {
                Ok(()) => Ok(()),
                Err(remove_error) if remove_error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(remove_error) => Err(remove_error),
            },
        };
        let git_restore = std::fs::remove_dir_all(&git_path);
        if let Err(restore_error) = ignore_restore.and(git_restore) {
            return Err(GitError::Other(format!(
                "local versioning setup failed ({error}) and cleanup was incomplete: {restore_error}"
            )));
        }
        return Err(error);
    }
    Ok(())
}

fn ensure_gitignore_entries(
    vault_path: &Path,
    cache_db_path: &Path,
    settings_file_path: &Path,
) -> Result<(), String> {
    let mut entries = Vec::new();
    if let Some(entry) = vault_relative_ignore_entry(vault_path, cache_db_path, false) {
        entries.push(entry);
    }
    if let Some(entry) = vault_relative_ignore_entry(vault_path, settings_file_path, false) {
        entries.push(entry);
    }
    if entries.is_empty() {
        return Ok(());
    }

    let ignore_path = vault_path.join(".gitignore");
    let existing = std::fs::read_to_string(&ignore_path).unwrap_or_default();
    let existing_lines: std::collections::HashSet<&str> = existing.lines().collect();
    let missing: Vec<&String> = entries
        .iter()
        .filter(|entry| !existing_lines.contains(entry.as_str()))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }

    let mut updated = existing;
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    for entry in missing {
        updated.push_str(entry);
        updated.push('\n');
    }
    std::fs::write(&ignore_path, updated).map_err(|error| error.to_string())
}

fn absolutize(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

/// A gitignore-style entry for `target` relative to `vault_path`, or `None`
/// when `target` does not live inside the vault (nothing to ignore: the file
/// is already outside the repository).
fn vault_relative_ignore_entry(
    vault_path: &Path,
    target: &Path,
    trailing_slash: bool,
) -> Option<String> {
    let vault_abs = absolutize(vault_path);
    let target_abs = absolutize(target);
    let relative = target_abs.strip_prefix(&vault_abs).ok()?;
    if relative.as_os_str().is_empty() {
        return None;
    }
    let mut pattern = relative
        .components()
        .map(|component| escape_gitignore_component(&component.as_os_str().to_string_lossy()))
        .collect::<Vec<_>>()
        .join("/");
    if trailing_slash {
        pattern.push('/');
    }
    Some(pattern)
}

/// Quote one filesystem component for use in a Git ignore pattern. A generated
/// cache/settings filename is data, never an operator-supplied glob: escaping
/// every Git pattern metacharacter keeps the entry exact even when a configured
/// location happens to contain a wildcard, bracket, negation, comment, or
/// backslash. Separators are added only after components are escaped.
fn escape_gitignore_component(component: &str) -> String {
    let mut escaped = String::with_capacity(component.len());
    for character in component.chars() {
        if matches!(character, '*' | '?' | '[' | ']' | '!' | '#' | '\\') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

/// True when the working tree has uncommitted drift (new, modified, or
/// deleted files, tracked or not — anything `.gitignore` does not already
/// exclude). Used at startup so turning versioning on for a vault with
/// existing edits commits that drift immediately, in both local and remote
/// mode, rather than waiting for the next write (issue #56).
pub fn has_uncommitted_changes(config: &GitConfig) -> Result<bool, GitError> {
    let repo = Repository::discover(&config.vault_path)?;
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true);
    let statuses = repo.statuses(Some(&mut opts))?;
    let vault_relative = vault_relative_path(&repo, config)?;
    Ok(statuses.iter().any(|entry| {
        entry
            .path()
            .is_ok_and(|path| status_path_is_in_vault(path, &vault_relative))
    }))
}

fn same_path(a: &Path, b: &Path) -> bool {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    canon(a) == canon(b)
}

/// Stage only the Vault subtree and commit if it differs from HEAD. It never
/// contacts a remote, so callers hold the vault-write lock across this.
/// Returns whether a commit was made.
///
/// `paths` (the current write batch) is retained for the commit message, but
/// staging covers all detected Vault-subtree drift, not merely the batch — see
/// `commit_working_tree`.
pub fn commit_local(
    config: &GitConfig,
    paths: &[PathBuf],
    message: &str,
) -> Result<CommitOutcome, GitError> {
    let _ = paths;
    let repo = Repository::discover(&config.vault_path)?;
    let committed = commit_working_tree(&repo, config, message)?;
    Ok(CommitOutcome { committed })
}

/// Stage all Vault-subtree drift from a fresh in-memory index seeded at HEAD
/// and commit if it differs. Afterwards, refresh only that subtree in the
/// on-disk index so it agrees with the new HEAD; an operator's staging outside
/// the Vault stays untouched and is never swept into Local history.
fn commit_working_tree(
    repo: &Repository,
    config: &GitConfig,
    message: &str,
) -> Result<bool, GitError> {
    let mut index = repo.index()?;
    let head_commit = repo.head().ok().and_then(|head| head.peel_to_commit().ok());
    let vault_relative = vault_relative_path(repo, config)?;
    let preserve_vault_index = has_staged_vault_changes(repo, &vault_relative)?;
    if let Some(parent) = &head_commit {
        index.read_tree(&parent.tree()?)?;
    } else {
        index.clear()?;
    }
    stage_vault_drift(repo, &mut index, &vault_relative)?;
    let tree_oid = index.write_tree_to(repo)?;
    let tree = repo.find_tree(tree_oid)?;

    if let Some(parent) = head_commit {
        if parent.tree()?.id() == tree_oid {
            return Ok(false); // nothing staged differs from HEAD
        }
        let sig = signature(config)?;
        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent])?;
    } else {
        let sig = signature(config)?;
        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[])?;
    }
    // `commit` advances HEAD but does not update the index. Refresh precisely
    // the Vault subtree when it had no existing staging. If an operator did
    // stage Vault content, retain that index exactly: it may intentionally
    // differ from both the working tree and the just-created commit.
    if !preserve_vault_index {
        let mut worktree_index = repo.index()?;
        stage_vault_drift(repo, &mut worktree_index, &vault_relative)?;
        worktree_index.write()?;
    }
    Ok(true)
}

/// Return the Vault location relative to the discovered checkout. This is a
/// filesystem path, deliberately not a Git pathspec: a Vault name is
/// operator-controlled and must never be interpreted as a wildcard.
fn vault_relative_path(repo: &Repository, config: &GitConfig) -> Result<PathBuf, GitError> {
    let workdir = repo
        .workdir()
        .ok_or_else(|| GitError::Validation("repository is bare".to_string()))?;
    let vault_path = config.vault_path.canonicalize().map_err(|error| {
        GitError::Validation(format!("cannot canonicalize Vault path: {error}"))
    })?;
    let workdir = workdir.canonicalize().map_err(|error| {
        GitError::Validation(format!("cannot canonicalize repository root: {error}"))
    })?;
    let relative = vault_path.strip_prefix(&workdir).map_err(|_| {
        GitError::Validation("Vault path is outside its discovered repository".to_string())
    })?;
    Ok(relative.to_path_buf())
}

fn status_path_is_in_vault(status_path: &str, vault_relative: &Path) -> bool {
    vault_relative.as_os_str().is_empty() || Path::new(status_path).starts_with(vault_relative)
}

fn has_staged_vault_changes(repo: &Repository, vault_relative: &Path) -> Result<bool, GitError> {
    let statuses = repo.statuses(None)?;
    let staged = git2::Status::INDEX_NEW
        | git2::Status::INDEX_MODIFIED
        | git2::Status::INDEX_DELETED
        | git2::Status::INDEX_RENAMED
        | git2::Status::INDEX_TYPECHANGE;
    statuses.iter().try_fold(false, |found, entry| {
        if found {
            return Ok(true);
        }
        let path = entry.path().map_err(|error| {
            GitError::Validation(format!("cannot read changed Git path as UTF-8: {error}"))
        })?;
        Ok(status_path_is_in_vault(path, vault_relative) && entry.status().intersects(staged))
    })
}

/// Add or remove exactly the changed, non-ignored paths reported by Git inside
/// the configured Vault. Working from status entries rather than a pathspec
/// keeps special characters in the Vault directory literal.
fn stage_vault_drift(
    repo: &Repository,
    index: &mut git2::Index,
    vault_relative: &Path,
) -> Result<(), GitError> {
    let mut options = git2::StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .renames_head_to_index(false)
        .renames_index_to_workdir(false);
    for entry in repo.statuses(Some(&mut options))?.iter() {
        let path = entry.path().map_err(|error| {
            GitError::Validation(format!("cannot read changed Git path as UTF-8: {error}"))
        })?;
        if !status_path_is_in_vault(path, vault_relative) {
            continue;
        }
        let relative_path = Path::new(path);
        let worktree_path = repo
            .workdir()
            .expect("non-bare repository was validated")
            .join(relative_path);
        if worktree_path.exists() {
            index.add_path(relative_path)?;
        } else {
            index.remove_path(relative_path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::Repository;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn local_history_commits_only_the_contained_vault_subtree() {
        let root = tempfile::tempdir().expect("repository root");
        let repo = Repository::init(root.path()).expect("init repository");
        let vault_name = "notes*";
        fs::create_dir(root.path().join(vault_name)).expect("notes directory");
        fs::write(root.path().join(vault_name).join("inside.md"), "inside").expect("inside note");
        fs::create_dir(root.path().join("notes-sibling")).expect("sibling directory");
        fs::write(
            root.path().join("notes-sibling/outside.md"),
            "outside sibling",
        )
        .expect("outside sibling");
        fs::write(root.path().join("outside.md"), "outside").expect("outside file");
        let mut operator_index = repo.index().expect("operator index");
        operator_index
            .add_path(Path::new("outside.md"))
            .expect("stage outside file");
        operator_index.write().expect("preserve operator staging");
        let config = GitConfig {
            vault_path: root.path().join(vault_name),
            mode: GitMode::Local,
            remote: "origin".to_string(),
            branch: "main".to_string(),
            username: String::new(),
            token: String::new(),
            debounce_seconds: 1,
            author_name: "Test".to_string(),
            author_email: "test@example.invalid".to_string(),
        };

        let outcome = commit_local(&config, &[], "local Vault history").expect("commit subtree");
        assert!(
            outcome.committed,
            "Local history must finish without consulting a remote"
        );
        let head = repo.head().expect("head").peel_to_commit().expect("commit");
        let tree = head.tree().expect("tree");
        assert!(tree.get_path(Path::new("notes*/inside.md")).is_ok());
        assert!(
            tree.get_path(Path::new("notes-sibling/outside.md"))
                .is_err(),
            "pathspec syntax in a Vault name must not stage a sibling"
        );
        assert!(tree.get_path(Path::new("outside.md")).is_err());
        assert!(
            repo.index()
                .expect("operator index after the turn")
                .get_path(Path::new("outside.md"), 0)
                .is_some(),
            "outside staged work remains in the operator index"
        );
        let mut status_options = git2::StatusOptions::new();
        status_options
            .include_untracked(true)
            .recurse_untracked_dirs(true);
        assert!(
            !repo
                .statuses(Some(&mut status_options))
                .expect("statuses")
                .iter()
                .any(|entry| entry.path().is_ok_and(|path| path == "notes*/inside.md")),
            "the committed Vault path is clean in the operator index"
        );
        assert!(
            root.path().join("outside.md").exists(),
            "manual outside work remains"
        );
    }

    #[test]
    fn local_history_preserves_staged_vault_content() {
        let root = tempfile::tempdir().expect("repository root");
        let repo = Repository::init(root.path()).expect("init repository");
        let vault_path = root.path().join("notes");
        fs::create_dir(&vault_path).expect("notes directory");
        let note = vault_path.join("inside.md");
        fs::write(&note, "operator staging").expect("staged content");
        let mut operator_index = repo.index().expect("operator index");
        operator_index
            .add_path(Path::new("notes/inside.md"))
            .expect("stage Vault content");
        operator_index.write().expect("preserve operator staging");
        fs::write(&note, "working tree content").expect("working content");
        let config = GitConfig {
            vault_path,
            mode: GitMode::Local,
            remote: "origin".to_string(),
            branch: "main".to_string(),
            username: String::new(),
            token: String::new(),
            debounce_seconds: 1,
            author_name: "Test".to_string(),
            author_email: "test@example.invalid".to_string(),
        };

        commit_local(&config, &[], "local Vault history").expect("commit working tree");

        let head = repo.head().expect("head").peel_to_commit().expect("commit");
        let committed = head
            .tree()
            .expect("tree")
            .get_path(Path::new("notes/inside.md"))
            .expect("committed note")
            .to_object(&repo)
            .expect("note object")
            .peel_to_blob()
            .expect("note blob");
        assert_eq!(committed.content(), b"working tree content");
        let staged = repo
            .index()
            .expect("operator index after the turn")
            .get_path(Path::new("notes/inside.md"), 0)
            .expect("preserved staged entry");
        assert_eq!(
            repo.find_blob(staged.id)
                .expect("preserved staged blob")
                .content(),
            b"operator staging",
            "Local history must not overwrite the operator's staged Vault content"
        );
    }

    /// `run_local_history_git_turn` is `dispatch_managed_git_turn_with`'s
    /// `ExistingGit` + `VaultGitMode::LocalHistory` counterpart to
    /// `run_managed_git_turn`. The composed `vault_runtime` test exercises it
    /// through the full async dispatch path; this proves its own contract
    /// directly: repository root and Vault root differ, accumulated
    /// Vault-subtree drift lands in a new commit, and manual work sitting
    /// outside the Vault in the same enclosing checkout is left completely
    /// untouched rather than force-checked-out or swept in (issue #94's
    /// third and fourth acceptance criteria).
    #[test]
    fn run_local_history_git_turn_commits_contained_drift_and_leaves_outside_work_untouched() {
        let root = tempfile::tempdir().expect("repository root");
        let repo = Repository::init(root.path()).expect("init repository");
        // A HEAD commit unrelated to the Vault, so the repository root and
        // Vault root genuinely differ and there is a parent to commit atop.
        fs::write(root.path().join("README.md"), "root readme").expect("root readme");
        commit_all(&repo, "initial commit");

        let vault_path = root.path().join("notes");
        fs::create_dir(&vault_path).expect("notes directory");
        // Drift inside the Vault subtree, uncommitted before the turn runs.
        fs::write(vault_path.join("Idea.md"), "# idea\n").expect("drift note");
        // Manual work directly in the repository root, outside the Vault,
        // uncommitted: must never be staged, committed, or discarded.
        fs::write(root.path().join("outside.md"), "manual outside work").expect("outside file");

        let outcome = run_local_history_git_turn(
            vault_path.clone(),
            "Test".to_string(),
            "test@example.invalid".to_string(),
        )
        .expect("local history turn succeeds");
        assert_eq!(outcome, ManagedGitOutcome::Synchronized);

        let head = repo.head().expect("head").peel_to_commit().expect("commit");
        assert_eq!(head.parent_count(), 1, "exactly one new commit was made");
        let tree = head.tree().expect("tree");
        assert!(
            tree.get_path(Path::new("notes/Idea.md")).is_ok(),
            "the Vault-subtree drift was committed"
        );
        assert!(
            tree.get_path(Path::new("outside.md")).is_err(),
            "work outside the Vault must never be swept into Local history"
        );
        assert_eq!(
            fs::read_to_string(root.path().join("outside.md")).expect("outside file survives"),
            "manual outside work",
            "manual outside work must survive byte-for-byte, never force-checked-out over"
        );
        let mut status_options = git2::StatusOptions::new();
        status_options
            .include_untracked(true)
            .recurse_untracked_dirs(true);
        assert!(
            repo.statuses(Some(&mut status_options))
                .expect("statuses")
                .iter()
                .any(|entry| entry.path().is_ok_and(|path| path == "outside.md")
                    && entry.status().contains(git2::Status::WT_NEW)),
            "the outside file remains untracked, exactly as the operator left it"
        );

        // A second turn with nothing new to commit reports UpToDate.
        let second = run_local_history_git_turn(
            vault_path,
            "Test".to_string(),
            "test@example.invalid".to_string(),
        )
        .expect("second local history turn succeeds");
        assert_eq!(second, ManagedGitOutcome::UpToDate);
    }

    /// Direct proof of issue #94's fourth acceptance criterion for a tracked
    /// (not merely untracked) manual edit: an uncommitted change to a file
    /// already inside the Vault subtree is committed with its edited
    /// content, never reverted or discarded — there is no interrupted-merge
    /// hard reset in `GitMode::Local` (that recovery path is gated to
    /// `GitMode::Remote` in `commit_local`) to force-checkout over it.
    #[test]
    fn run_local_history_git_turn_preserves_an_uncommitted_tracked_edit_instead_of_discarding_it() {
        let root = tempfile::tempdir().expect("repository root");
        let repo = Repository::init(root.path()).expect("init repository");
        let vault_path = root.path().join("notes");
        fs::create_dir(&vault_path).expect("notes directory");
        fs::write(vault_path.join("Idea.md"), "v1").expect("initial note content");
        commit_all(&repo, "initial commit");

        // A manual, uncommitted edit to the already-tracked file.
        fs::write(vault_path.join("Idea.md"), "v2 - manual edit").expect("manual edit");

        let outcome = run_local_history_git_turn(
            vault_path,
            "Test".to_string(),
            "test@example.invalid".to_string(),
        )
        .expect("local history turn succeeds");
        assert_eq!(outcome, ManagedGitOutcome::Synchronized);

        let head = repo.head().expect("head").peel_to_commit().expect("commit");
        let committed = head
            .tree()
            .expect("tree")
            .get_path(Path::new("notes/Idea.md"))
            .expect("committed note")
            .to_object(&repo)
            .expect("note object")
            .peel_to_blob()
            .expect("note blob");
        assert_eq!(
            committed.content(),
            b"v2 - manual edit",
            "the manual edit was committed, not discarded or reverted to v1"
        );
    }

    #[test]
    fn run_local_history_git_turn_classifies_a_bare_repository_as_non_retryable() {
        // A bare repository has no working tree to version, so
        // `validate_local_repo`'s "repository is bare" check must reject it.
        // Deliberately does not rely on "no repository at all" here: with
        // `TMPDIR` redirected onto real disk (see repo docs), a plain
        // non-repo tempdir can still land inside some ambient enclosing
        // checkout and make `Repository::discover` succeed unexpectedly.
        let root = tempfile::tempdir().expect("bare repository directory");
        Repository::init_bare(root.path()).expect("init bare repository");
        let vault_path = root.path().join("notes");
        fs::create_dir(&vault_path).expect("notes directory");

        let error = run_local_history_git_turn(
            vault_path,
            "Test".to_string(),
            "test@example.invalid".to_string(),
        )
        .expect_err("bare repository has no working tree to version");
        assert_eq!(error.code(), "existing_git_local_history_validation_failed");
        assert!(!error.retryable());
    }

    fn base_config(vault: &Path) -> GitConfig {
        GitConfig {
            vault_path: vault.to_path_buf(),
            mode: super::super::config::GitMode::Remote,
            remote: "origin".to_string(),
            branch: "main".to_string(),
            username: "hatchdoor".to_string(),
            token: "unused-local".to_string(),
            debounce_seconds: 30,
            author_name: "Hatchdoor".to_string(),
            author_email: "hatchdoor@localhost".to_string(),
        }
    }

    /// Create a bare "remote" plus a working clone on branch `main` with one commit,
    /// the remote named `origin`. Returns (tmp, work_dir, remote_dir).
    fn init_repo_with_remote() -> (TempDir, PathBuf, PathBuf) {
        let tmp = TempDir::new().unwrap();
        let remote_dir = tmp.path().join("remote.git");
        Repository::init_bare(&remote_dir).unwrap();

        let work_dir = tmp.path().join("work");
        let repo = Repository::init(&work_dir).unwrap();
        // Force the initial branch to `main`.
        repo.set_head("refs/heads/main").unwrap();
        fs::write(work_dir.join("Home.md"), "# Home\n").unwrap();
        commit_all(&repo, "initial");
        repo.remote("origin", remote_dir.to_str().unwrap()).unwrap();
        // Push the initial commit so the remote has `main`.
        let mut remote = repo.find_remote("origin").unwrap();
        remote
            .push(&["refs/heads/main:refs/heads/main"], None)
            .unwrap();
        drop(remote);
        drop(repo);
        // A bare repository created by libgit2 can retain an unborn `master`
        // HEAD even after `main` is pushed. Point HEAD at the fixture branch so
        // follow-up clones reliably check out the commit on every host.
        Repository::open_bare(&remote_dir)
            .unwrap()
            .set_head("refs/heads/main")
            .unwrap();
        (tmp, work_dir, remote_dir)
    }

    fn commit_all(repo: &Repository, message: &str) {
        let mut index = repo.index().unwrap();
        index
            .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = Signature::now("Test", "test@localhost").unwrap();
        let parents: Vec<git2::Commit> = match repo.head().ok().and_then(|h| h.target()) {
            Some(oid) => vec![repo.find_commit(oid).unwrap()],
            None => vec![],
        };
        let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parent_refs)
            .unwrap();
    }

    #[test]
    fn validate_accepts_well_formed_repo() {
        let (_tmp, work, _remote) = init_repo_with_remote();
        let config = base_config(&work);
        assert!(validate_repo(&config).is_ok());
    }

    #[test]
    fn validate_rejects_non_repo() {
        let tmp = TempDir::new().unwrap();
        let config = base_config(tmp.path());
        let err = validate_repo(&config).unwrap_err();
        assert!(matches!(err, GitError::Validation(_)));
    }

    #[test]
    fn validate_rejects_wrong_branch() {
        let (_tmp, work, _remote) = init_repo_with_remote();
        let mut config = base_config(&work);
        config.branch = "release".to_string();
        let err = validate_repo(&config).unwrap_err();
        assert!(matches!(err, GitError::Validation(_)));
    }

    #[test]
    fn local_mode_initializes_and_commits_without_a_remote() {
        let temp = TempDir::new().unwrap();
        let vault = temp.path().join("vault");
        std::fs::create_dir(&vault).unwrap();
        let mut config = base_config(&vault);
        config.mode = GitMode::Local;
        config.token.clear();
        let cache_db_path = vault.join("data/cache/hatchdoor-cache.sqlite3");
        let settings_file_path = vault.join("data/cache/settings.json");
        init_local_repo(&config, &cache_db_path, &settings_file_path)
            .expect("initialize local history");
        std::fs::write(vault.join("Home.md"), "# Home\n").unwrap();

        let outcome = commit_local(&config, &[vault.join("Home.md")], "hatchdoor: local Home")
            .expect("commit locally");
        assert!(outcome.committed);
        assert!(vault.join(".git").exists());
        assert_eq!(
            std::fs::read_to_string(vault.join(".gitignore")).unwrap(),
            "data/cache/hatchdoor-cache.sqlite3\ndata/cache/settings.json\n"
        );
    }

    #[test]
    fn init_local_repo_appends_to_an_existing_gitignore_instead_of_skipping_it() {
        // Issue #61: a vault that already has its own .gitignore must not end
        // up committing the cache database just because Hatchdoor declined to
        // touch a file that already existed.
        let temp = TempDir::new().unwrap();
        let vault = temp.path().join("vault");
        std::fs::create_dir(&vault).unwrap();
        std::fs::write(vault.join(".gitignore"), "*.tmp\n").unwrap();
        let mut config = base_config(&vault);
        config.mode = GitMode::Local;
        config.token.clear();

        let cache_db_path = vault.join("data/cache/hatchdoor-cache.sqlite3");
        let settings_file_path = vault.join("data/cache/settings.json");
        init_local_repo(&config, &cache_db_path, &settings_file_path)
            .expect("initialize local history");

        let contents = std::fs::read_to_string(vault.join(".gitignore")).unwrap();
        assert!(contents.contains("*.tmp"), "kept the operator's own entry");
        assert!(
            contents.contains("data/cache/hatchdoor-cache.sqlite3"),
            "appended the cache database entry: {contents}"
        );
        assert!(
            contents.contains("data/cache/settings.json"),
            "appended the settings file entry: {contents}"
        );
    }

    #[test]
    fn one_turn_commits_a_whole_batch_of_writes_as_a_single_commit() {
        // #177's "ONE Git commit covers the whole batch" acceptance criterion.
        // `batch` itself contains no Git handling by design: it writes Markdown
        // exactly as the standalone tools do, and holds each touched Vault's
        // mutation lock for the whole call so a sync turn cannot run between two
        // of its items. What that buys is asserted here, at the layer that
        // actually makes commits — a turn finding N dirty files makes one commit
        // carrying all N, never one per file.
        let temp = TempDir::new().unwrap();
        let vault = temp.path().join("vault");
        std::fs::create_dir(&vault).unwrap();
        let mut config = base_config(&vault);
        config.mode = GitMode::Local;
        config.token.clear();
        init_local_repo(
            &config,
            &vault.join("data/cache/hatchdoor-cache.sqlite3"),
            &vault.join("data/cache/settings.json"),
        )
        .expect("initialize local history");

        // A first turn, so the batch below has a parent commit to hang from.
        std::fs::write(vault.join("Home.md"), "# Home\n").unwrap();
        commit_local(&config, &[], "hatchdoor: seed").expect("seed commit");
        let before = Repository::open(&vault)
            .unwrap()
            .head()
            .unwrap()
            .peel_to_commit()
            .unwrap()
            .id();

        // Stand in for one batch call: several note writes, no turn between them.
        std::fs::create_dir_all(vault.join("Inbox")).unwrap();
        for (path, contents) in [
            ("Inbox/One.md", "# One\n"),
            ("Inbox/Two.md", "# Two\n"),
            ("Inbox/Three.md", "# Three\n"),
        ] {
            std::fs::write(vault.join(path), contents).unwrap();
        }

        let outcome = commit_local(&config, &[], "hatchdoor: batch").expect("commit the batch");
        assert!(outcome.committed);

        let repo = Repository::open(&vault).unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(
            head.parent(0).unwrap().id(),
            before,
            "the batch must add exactly one commit, not one per written note"
        );

        let tree = head.tree().unwrap();
        for path in ["Inbox/One.md", "Inbox/Two.md", "Inbox/Three.md"] {
            assert!(
                tree.get_path(Path::new(path)).is_ok(),
                "{path} must be inside that single commit"
            );
        }
    }

    #[test]
    fn init_local_repo_ignores_nothing_when_cache_and_settings_live_outside_the_vault() {
        let temp = TempDir::new().unwrap();
        let vault = temp.path().join("vault");
        std::fs::create_dir(&vault).unwrap();
        let mut config = base_config(&vault);
        config.mode = GitMode::Local;
        config.token.clear();

        let outside = temp.path().join("data/cache/hatchdoor-cache.sqlite3");
        let outside_settings = temp.path().join("data/cache/settings.json");
        init_local_repo(&config, &outside, &outside_settings).expect("initialize local history");

        assert!(
            !vault.join(".gitignore").exists(),
            "nothing inside the vault needs ignoring"
        );
    }

    #[test]
    fn init_local_repo_ignores_exact_root_and_nested_generated_sidecars() {
        let temp = TempDir::new().unwrap();
        let vault = temp.path().join("vault");
        std::fs::create_dir(&vault).unwrap();
        let mut config = base_config(&vault);
        config.mode = GitMode::Local;
        config.token.clear();

        let root_cache = vault.join(".hatchdoor-cache.sqlite3");
        let root_settings = vault.join(".hatchdoor-settings.json");
        init_local_repo(&config, &root_cache, &root_settings).expect("initialize root sidecars");

        let root_ignore = std::fs::read_to_string(vault.join(".gitignore")).unwrap();
        let entries: std::collections::HashSet<_> =
            root_ignore.lines().map(str::to_owned).collect();
        assert!(entries.contains(".hatchdoor-cache.sqlite3"));
        assert!(entries.contains(".hatchdoor-settings.json"));

        let nested_vault = temp.path().join("nested-vault");
        std::fs::create_dir(&nested_vault).unwrap();
        let mut nested_config = base_config(&nested_vault);
        nested_config.mode = GitMode::Local;
        nested_config.token.clear();
        let nested_cache = nested_vault.join(".hatchdoor/cache/cache.sqlite3");
        let nested_settings = nested_vault.join(".hatchdoor/settings.json");
        init_local_repo(&nested_config, &nested_cache, &nested_settings)
            .expect("initialize nested sidecars");

        let nested_ignore = std::fs::read_to_string(nested_vault.join(".gitignore")).unwrap();
        let entries: std::collections::HashSet<_> =
            nested_ignore.lines().map(str::to_owned).collect();
        assert!(entries.contains(".hatchdoor/cache/cache.sqlite3"));
        assert!(entries.contains(".hatchdoor/settings.json"));
        assert!(
            !entries.contains(".hatchdoor/cache/"),
            "a generated database must not hide unrelated user files in its directory"
        );
    }

    #[test]
    fn init_local_repo_ignores_only_literal_configured_sidecars_with_git_metacharacters() {
        let temp = TempDir::new().unwrap();
        let root_vault = temp.path().join("root-vault");
        std::fs::create_dir(&root_vault).unwrap();
        let mut root_config = base_config(&root_vault);
        root_config.mode = GitMode::Local;
        root_config.token.clear();
        let root_cache = root_vault.join("cache*.sqlite3");
        let root_settings = root_vault.join("settings[old].json");
        init_local_repo(&root_config, &root_cache, &root_settings).expect("initialize root");
        let root_repo = Repository::open(&root_vault).expect("open root repository");
        assert_git_ignore_exact(
            &root_repo,
            std::path::Path::new("cache*.sqlite3"),
            std::path::Path::new("cachex.sqlite3"),
        );
        assert_git_ignore_exact(
            &root_repo,
            std::path::Path::new("settings[old].json"),
            std::path::Path::new("settingsold.json"),
        );

        let nested_vault = temp.path().join("nested-vault");
        let nested_dir = nested_vault.join("#sidecars");
        std::fs::create_dir(&nested_vault).unwrap();
        std::fs::create_dir(&nested_dir).unwrap();
        let mut nested_config = base_config(&nested_vault);
        nested_config.mode = GitMode::Local;
        nested_config.token.clear();
        let nested_cache = nested_dir.join("cache?.sqlite3");
        let nested_settings = nested_dir.join("!settings.json");
        init_local_repo(&nested_config, &nested_cache, &nested_settings)
            .expect("initialize nested");
        let nested_repo = Repository::open(&nested_vault).expect("open nested repository");
        assert_git_ignore_exact(
            &nested_repo,
            std::path::Path::new("#sidecars/cache?.sqlite3"),
            std::path::Path::new("#sidecars/cachex.sqlite3"),
        );
        assert_git_ignore_exact(
            &nested_repo,
            std::path::Path::new("#sidecars/!settings.json"),
            std::path::Path::new("#sidecars/xsettings.json"),
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn init_local_repo_escapes_backslashes_in_configured_sidecars() {
        let temp = TempDir::new().unwrap();
        let vault = temp.path().join("vault");
        std::fs::create_dir(&vault).unwrap();
        let mut config = base_config(&vault);
        config.mode = GitMode::Local;
        config.token.clear();
        let cache = vault.join(r"cache\generated.sqlite3");
        let settings = vault.join(r"settings\generated.json");
        init_local_repo(&config, &cache, &settings).expect("initialize local history");

        let repo = Repository::open(&vault).expect("open repository");
        assert_git_ignore_exact(
            &repo,
            std::path::Path::new(r"cache\generated.sqlite3"),
            std::path::Path::new("cachegenerated.sqlite3"),
        );
        assert_git_ignore_exact(
            &repo,
            std::path::Path::new(r"settings\generated.json"),
            std::path::Path::new("settingsgenerated.json"),
        );
    }

    fn assert_git_ignore_exact(repo: &Repository, generated: &Path, adjacent_user_file: &Path) {
        assert!(
            repo.status_should_ignore(generated)
                .expect("evaluate generated sidecar ignore"),
            "configured generated sidecar '{}' must be ignored",
            generated.display()
        );
        assert!(
            !repo
                .status_should_ignore(adjacent_user_file)
                .expect("evaluate adjacent user file ignore"),
            "adjacent user file '{}' must not match the generated-sidecar pattern",
            adjacent_user_file.display()
        );
    }
}
