use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};

use git2::{
    AnnotatedCommit, Cred, FetchOptions, MergeOptions, ObjectType, PushOptions, RemoteCallbacks,
    Repository, ResetType, Signature, TreeWalkMode, TreeWalkResult, build::CheckoutBuilder,
};

use super::config::{GitConfig, GitMode};
use super::managed_task::ManagedGitOutcome;
use super::message::build_commit_message;
use crate::vault_work::VaultWorkError;

/// What a sync attempt did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncOutcome {
    /// Working tree matched HEAD and nothing was unpushed; no commit created.
    NoChanges,
    /// A commit was created and pushed (possibly after a clean merge).
    Pushed { committed: bool },
    /// A local-only versioning run committed without contacting a remote.
    Committed { committed: bool },
}

/// Result of a sync attempt, suitable for status reporting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncReport {
    pub outcome: SyncOutcome,
}

/// Result of the local commit phase: whether a new commit was created, and
/// whether the remote must be contacted (either we committed, or earlier
/// commits are still unpushed). When `needs_remote` is false the sync is a
/// no-op and no network I/O should happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommitOutcome {
    pub committed: bool,
    pub needs_remote: bool,
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

impl GitError {
    /// Machine-readable category, surfaced in `GitSyncStatus.last_error_kind`.
    pub fn kind(&self) -> &'static str {
        match self {
            GitError::Validation(_) => "validation",
            GitError::Conflict { .. } => "conflict",
            GitError::DirtyWorkingTree { .. } => "dirty_tree",
            GitError::ManualRecovery { .. } => "manual_recovery",
            GitError::Remote(_) => "remote",
            GitError::Other(_) => "other",
        }
    }
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

/// Build credential + transfer callbacks bound to the configured HTTPS token.
fn remote_callbacks(config: &GitConfig) -> RemoteCallbacks<'_> {
    let mut cb = RemoteCallbacks::new();
    cb.credentials(move |_url, _username_from_url, _allowed| {
        Cred::userpass_plaintext(&config.username, &config.token)
    });
    cb
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
/// not the full settings-derived `GitConfig` the legacy single-Vault path
/// builds from `HATCHDOOR_GIT_*` configuration.
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
/// redacted [`VaultWorkError`], mirroring the legacy single-Vault task's
/// existing transient/non-transient split
/// (`git/task.rs::run_one_sync_with_message`): `Remote`/`Other` are worth an
/// automatic retry, everything else needs a human or the enclosing
/// checkout's state to change first.
///
/// `Conflict` and `DirtyWorkingTree` are only ever produced by
/// `merge_remote`, which `commit_local` never calls for `GitMode::Local`, and
/// `Remote` similarly never occurs since neither `validate_local_repo` nor
/// `commit_local` touch network state for that mode — both are classified
/// defensively here rather than assumed unreachable. `ManualRecovery` is
/// reachable: `commit_local` refuses to touch a repository left in an
/// operation Hatchdoor cannot prove it owns, and only an operator can clear
/// that, so it is never retryable.
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

/// True when the local branch has commits the remote tracking ref lacks.
/// Used at startup to flush commits stranded by an earlier outage.
pub fn has_unpushed(config: &GitConfig) -> Result<bool, GitError> {
    let repo = Repository::open(&config.vault_path)?;
    let local = repo
        .refname_to_id(&format!("refs/heads/{}", config.branch))
        .map_err(GitError::from)?;
    let remote_ref = format!("refs/remotes/{}/{}", config.remote, config.branch);
    match repo.refname_to_id(&remote_ref) {
        Ok(remote_oid) => {
            let (ahead, _behind) = repo.graph_ahead_behind(local, remote_oid)?;
            Ok(ahead > 0)
        }
        // No tracking ref yet → treat as needing a push.
        Err(_) => Ok(true),
    }
}

/// Number of local commits on the configured branch not yet on the remote
/// tracking ref. Surfaced in status so a client can tell a conflict left a
/// commit stranded locally. Best-effort: callers treat an error as "unknown".
pub fn unpushed_count(config: &GitConfig) -> Result<usize, GitError> {
    let repo = Repository::open(&config.vault_path)?;
    let local = repo.refname_to_id(&format!("refs/heads/{}", config.branch))?;
    let remote_ref = format!("refs/remotes/{}/{}", config.remote, config.branch);
    match repo.refname_to_id(&remote_ref) {
        Ok(remote_oid) => {
            let (ahead, _behind) = repo.graph_ahead_behind(local, remote_oid)?;
            Ok(ahead)
        }
        // No tracking ref yet → every commit on the branch is unpushed.
        Err(_) => {
            let mut walk = repo.revwalk()?;
            walk.push(local)?;
            Ok(walk.count())
        }
    }
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

const MERGE_RECOVERY_MARKER: &str = "hatchdoor-merge-recovery";
const MERGE_OPERATION_NONCE_PREFIX: &str = "Hatchdoor-Operation-Nonce: ";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MergeMarkerPhase {
    Prepared,
    Active,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MergeRecoveryMarker {
    phase: MergeMarkerPhase,
    nonce: [u8; 16],
    local_oid: git2::Oid,
    remote_oid: git2::Oid,
    index_fingerprint: Option<git2::Oid>,
    worktree_fingerprint: Option<git2::Oid>,
    metadata_fingerprint: Option<git2::Oid>,
}

fn merge_marker_path(repo: &Repository) -> PathBuf {
    repo.path().join(MERGE_RECOVERY_MARKER)
}

fn manual_recovery(state: git2::RepositoryState, reason: impl Into<String>) -> GitError {
    GitError::ManualRecovery {
        state: format!("{state:?}"),
        reason: reason.into(),
    }
}

fn fresh_operation_nonce() -> Result<[u8; 16], GitError> {
    let mut nonce = [0_u8; 16];
    getrandom::fill(&mut nonce)
        .map_err(|error| GitError::Other(format!("cannot create Git operation nonce: {error}")))?;
    Ok(nonce)
}

fn encode_nonce(nonce: &[u8; 16]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(32);
    for byte in nonce {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn parse_nonce(value: &str) -> Option<[u8; 16]> {
    if value.len() != 32 {
        return None;
    }
    fn nibble(byte: u8) -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            _ => None,
        }
    }
    let mut nonce = [0_u8; 16];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        nonce[index] = (nibble(pair[0])? << 4) | nibble(pair[1])?;
    }
    Some(nonce)
}

fn write_merge_marker(repo: &Repository, marker: MergeRecoveryMarker) -> Result<(), GitError> {
    let phase = match marker.phase {
        MergeMarkerPhase::Prepared => "prepared",
        MergeMarkerPhase::Active => "active",
    };
    let mut contents = format!(
        "version=3\noperation=merge\nphase={phase}\nnonce={}\nlocal={}\nremote={}\n",
        encode_nonce(&marker.nonce),
        marker.local_oid,
        marker.remote_oid
    );
    if let (Some(index), Some(worktree), Some(metadata)) = (
        marker.index_fingerprint,
        marker.worktree_fingerprint,
        marker.metadata_fingerprint,
    ) {
        contents.push_str(&format!(
            "index={index}\nworktree={worktree}\nmetadata={metadata}\n"
        ));
    }

    let path = merge_marker_path(repo);
    let mut options = std::fs::OpenOptions::new();
    options.write(true);
    match marker.phase {
        MergeMarkerPhase::Prepared => {
            options.create_new(true);
        }
        MergeMarkerPhase::Active => {
            options.create(false).truncate(true);
        }
    }
    let mut file = options.open(&path).map_err(|error| {
        GitError::Other(format!(
            "cannot persist Hatchdoor merge ownership marker {}: {error}",
            path.display()
        ))
    })?;
    file.write_all(contents.as_bytes()).map_err(|error| {
        GitError::Other(format!(
            "cannot write Hatchdoor merge ownership marker {}: {error}",
            path.display()
        ))
    })?;
    file.sync_all().map_err(|error| {
        GitError::Other(format!(
            "cannot sync Hatchdoor merge ownership marker {}: {error}",
            path.display()
        ))
    })
}

fn clear_merge_marker(repo: &Repository) -> Result<(), GitError> {
    let path = merge_marker_path(repo);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(GitError::Other(format!(
            "cannot remove Hatchdoor merge ownership marker {}: {error}",
            path.display()
        ))),
    }
}

fn read_merge_marker(
    repo: &Repository,
    state: git2::RepositoryState,
) -> Result<MergeRecoveryMarker, GitError> {
    let path = merge_marker_path(repo);
    let contents = std::fs::read_to_string(&path).map_err(|error| {
        manual_recovery(
            state,
            if error.kind() == std::io::ErrorKind::NotFound {
                "no Hatchdoor ownership marker exists"
            } else {
                "the Hatchdoor ownership marker cannot be read"
            },
        )
    })?;
    let mut fields = std::collections::HashMap::new();
    for line in contents.lines() {
        let Some((key, value)) = line.split_once('=') else {
            return Err(manual_recovery(state, "the ownership marker is malformed"));
        };
        if fields.insert(key, value).is_some() {
            return Err(manual_recovery(
                state,
                "the ownership marker contains duplicate fields",
            ));
        }
    }
    if fields.get("version") != Some(&"3") || fields.get("operation") != Some(&"merge") {
        return Err(manual_recovery(
            state,
            "the ownership marker has an unsupported version or operation",
        ));
    }
    let parse_oid = |name: &str| {
        fields
            .get(name)
            .ok_or_else(|| manual_recovery(state, format!("the ownership marker lacks {name}")))
            .and_then(|value| {
                git2::Oid::from_str(value).map_err(|_| {
                    manual_recovery(state, format!("the ownership marker has invalid {name}"))
                })
            })
    };
    let phase = match fields.get("phase") {
        Some(&"prepared") => MergeMarkerPhase::Prepared,
        Some(&"active") => MergeMarkerPhase::Active,
        _ => {
            return Err(manual_recovery(
                state,
                "the ownership marker has an invalid phase",
            ));
        }
    };
    let marker = MergeRecoveryMarker {
        phase,
        nonce: fields
            .get("nonce")
            .and_then(|value| parse_nonce(value))
            .ok_or_else(|| manual_recovery(state, "the ownership marker has an invalid nonce"))?,
        local_oid: parse_oid("local")?,
        remote_oid: parse_oid("remote")?,
        index_fingerprint: fields
            .get("index")
            .map(|_| parse_oid("index"))
            .transpose()?,
        worktree_fingerprint: fields
            .get("worktree")
            .map(|_| parse_oid("worktree"))
            .transpose()?,
        metadata_fingerprint: fields
            .get("metadata")
            .map(|_| parse_oid("metadata"))
            .transpose()?,
    };
    let active_has_fingerprints = marker.index_fingerprint.is_some()
        && marker.worktree_fingerprint.is_some()
        && marker.metadata_fingerprint.is_some();
    if (phase == MergeMarkerPhase::Active) != active_has_fingerprints {
        return Err(manual_recovery(
            state,
            "the ownership marker is incomplete for its phase",
        ));
    }
    Ok(marker)
}

fn append_fingerprint_frame(bytes: &mut Vec<u8>, value: &[u8]) {
    bytes.extend_from_slice(&(value.len() as u64).to_be_bytes());
    bytes.extend_from_slice(value);
}

fn collect_merge_paths(
    repo: &Repository,
    index: &git2::Index,
    local_oid: git2::Oid,
    remote_oid: git2::Oid,
) -> Result<BTreeSet<String>, GitError> {
    let mut paths = BTreeSet::new();
    for entry in index.iter() {
        let path = std::str::from_utf8(&entry.path).map_err(|_| {
            GitError::Other("cannot prove merge ownership for a non-UTF-8 index path".to_string())
        })?;
        paths.insert(path.to_string());
    }
    for oid in [local_oid, remote_oid] {
        let tree = repo.find_commit(oid)?.tree()?;
        let mut invalid_path = false;
        tree.walk(TreeWalkMode::PreOrder, |root, entry| {
            if entry.kind() == Some(ObjectType::Tree) {
                return TreeWalkResult::Ok;
            }
            match std::str::from_utf8(entry.name_bytes()) {
                Ok(name) => {
                    paths.insert(format!("{root}{name}"));
                    TreeWalkResult::Ok
                }
                Err(_) => {
                    invalid_path = true;
                    TreeWalkResult::Abort
                }
            }
        })?;
        if invalid_path {
            return Err(GitError::Other(
                "cannot prove merge ownership for a non-UTF-8 tree path".to_string(),
            ));
        }
    }
    Ok(paths)
}

fn merge_fingerprints_for_index(
    index: &git2::Index,
    worktree: &Path,
    paths: BTreeSet<String>,
) -> Result<(git2::Oid, git2::Oid), GitError> {
    let mut index_bytes = Vec::new();
    for entry in index.iter() {
        let path = std::str::from_utf8(&entry.path).map_err(|_| {
            GitError::Other("cannot prove merge ownership for a non-UTF-8 index path".to_string())
        })?;
        append_fingerprint_frame(&mut index_bytes, path.as_bytes());
        append_fingerprint_frame(&mut index_bytes, entry.id.as_bytes());
        index_bytes.extend_from_slice(&entry.mode.to_be_bytes());
        index_bytes.push(((entry.flags >> 12) & 0x3) as u8);
    }
    let index_fingerprint = git2::Oid::hash_object(ObjectType::Blob, &index_bytes)?;

    let mut worktree_bytes = Vec::new();
    for relative in paths {
        append_fingerprint_frame(&mut worktree_bytes, relative.as_bytes());
        let path = worktree.join(&relative);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_file() => {
                worktree_bytes.push(b'f');
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    worktree_bytes.push(u8::from(metadata.permissions().mode() & 0o111 != 0));
                }
                #[cfg(not(unix))]
                worktree_bytes.push(0);
                let contents = std::fs::read(&path).map_err(|error| {
                    GitError::Other(format!(
                        "cannot fingerprint tracked path {}: {error}",
                        path.display()
                    ))
                })?;
                append_fingerprint_frame(&mut worktree_bytes, &contents);
            }
            Ok(metadata) if metadata.file_type().is_symlink() => {
                worktree_bytes.push(b'l');
                let target = std::fs::read_link(&path).map_err(|error| {
                    GitError::Other(format!(
                        "cannot fingerprint tracked symlink {}: {error}",
                        path.display()
                    ))
                })?;
                let target = target.to_str().ok_or_else(|| {
                    GitError::Other(format!(
                        "cannot prove merge ownership for non-UTF-8 symlink {}",
                        path.display()
                    ))
                })?;
                append_fingerprint_frame(&mut worktree_bytes, target.as_bytes());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                worktree_bytes.push(b'm');
            }
            Ok(_) => {
                return Err(GitError::Other(format!(
                    "cannot prove merge ownership for unsupported tracked path {}",
                    path.display()
                )));
            }
            Err(error) => {
                return Err(GitError::Other(format!(
                    "cannot fingerprint tracked path {}: {error}",
                    path.display()
                )));
            }
        }
    }
    let worktree_fingerprint = git2::Oid::hash_object(ObjectType::Blob, &worktree_bytes)?;
    Ok((index_fingerprint, worktree_fingerprint))
}

/// Fingerprint the semantic index entries and the exact Git-relevant tracked
/// worktree identity for every path present in either parent or the merge
/// index. Regular files include content and executable status; symlinks include
/// their target. Recovery therefore refuses resolutions, edits, chmods, type
/// changes, or recreation of merge-deleted paths.
fn merge_state_fingerprints(
    repo: &Repository,
    local_oid: git2::Oid,
    remote_oid: git2::Oid,
) -> Result<(git2::Oid, git2::Oid), GitError> {
    let index = repo.index()?;
    let paths = collect_merge_paths(repo, &index, local_oid, remote_oid)?;
    let workdir = repo
        .workdir()
        .ok_or_else(|| GitError::Other("cannot fingerprint a bare repository".to_string()))?;
    merge_fingerprints_for_index(&index, workdir, paths)
}

struct MergePreview {
    path: PathBuf,
}

struct ExpectedMergeMetadata {
    head: Vec<u8>,
    mode: Vec<u8>,
    message: Vec<u8>,
}

impl ExpectedMergeMetadata {
    fn from_index(their: &AnnotatedCommit<'_>, index: &git2::Index) -> Result<Self, GitError> {
        let remote_oid = their.id();
        let head = format!("{remote_oid}\n").into_bytes();
        let mode = b"no-ff".to_vec();
        // Hatchdoor constructs `their` with `find_annotated_commit`, so
        // libgit2 formats it as an OID rather than a named ref.
        let mut message = format!("Merge commit '{remote_oid}'\n").into_bytes();
        let mut conflict_paths = BTreeSet::new();
        for entry in index.iter() {
            if ((entry.flags >> 12) & 0x3) == 0 {
                continue;
            }
            let path = std::str::from_utf8(&entry.path).map_err(|_| {
                GitError::Other(
                    "cannot predict merge metadata for a non-UTF-8 conflict path".to_string(),
                )
            })?;
            conflict_paths.insert(path.to_string());
        }
        if !conflict_paths.is_empty() {
            message.extend_from_slice(b"\n#Conflicts:\n");
            for path in conflict_paths {
                message.extend_from_slice(format!("#\t{path}\n").as_bytes());
            }
        }
        Ok(Self {
            head,
            mode,
            message,
        })
    }

    fn with_nonce(&self, nonce: &[u8; 16]) -> Self {
        let mut message = self.message.clone();
        if !message.ends_with(b"\n") {
            message.push(b'\n');
        }
        message.extend_from_slice(
            format!("{MERGE_OPERATION_NONCE_PREFIX}{}\n", encode_nonce(nonce)).as_bytes(),
        );
        Self {
            head: self.head.clone(),
            mode: self.mode.clone(),
            message,
        }
    }
}

fn read_merge_metadata(repo: &Repository) -> Result<ExpectedMergeMetadata, GitError> {
    let read = |name: &str| {
        let path = repo.path().join(name);
        std::fs::read(&path).map_err(|error| {
            GitError::Other(format!(
                "cannot read merge metadata {}: {error}",
                path.display()
            ))
        })
    };
    Ok(ExpectedMergeMetadata {
        head: read("MERGE_HEAD")?,
        mode: read("MERGE_MODE")?,
        message: read("MERGE_MSG")?,
    })
}

fn merge_metadata_fingerprint(metadata: &ExpectedMergeMetadata) -> Result<git2::Oid, GitError> {
    let mut bytes = Vec::new();
    for (name, contents) in [
        ("MERGE_HEAD", metadata.head.as_slice()),
        ("MERGE_MODE", metadata.mode.as_slice()),
        ("MERGE_MSG", metadata.message.as_slice()),
    ] {
        append_fingerprint_frame(&mut bytes, name.as_bytes());
        append_fingerprint_frame(&mut bytes, contents);
    }
    Ok(git2::Oid::hash_object(ObjectType::Blob, &bytes)?)
}

fn verify_expected_merge_metadata(
    repo: &Repository,
    expected: &ExpectedMergeMetadata,
    state: git2::RepositoryState,
) -> Result<(), GitError> {
    let actual = read_merge_metadata(repo).map_err(|error| {
        manual_recovery(
            state,
            format!("the live merge metadata cannot be verified: {error}"),
        )
    })?;
    if actual.head != expected.head
        || actual.mode != expected.mode
        || actual.message != expected.message
    {
        return Err(manual_recovery(
            state,
            "the live merge metadata differs from the predicted operation",
        ));
    }
    Ok(())
}

impl MergePreview {
    fn create(repo: &Repository, nonce: &[u8; 16]) -> Result<Self, GitError> {
        let path = repo
            .path()
            .join(format!("hatchdoor-merge-preview-{}", encode_nonce(nonce)));
        std::fs::create_dir(&path).map_err(|error| {
            GitError::Other(format!(
                "cannot create merge ownership preview {}: {error}",
                path.display()
            ))
        })?;
        Ok(Self { path })
    }
}

impl Drop for MergePreview {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Produce Hatchdoor's ownership baseline without exposing a live merge. The
/// real merge is allowed to become Active only when its index and worktree are
/// exactly the result libgit2 predicted from the two immutable parent commits.
fn expected_merge_fingerprints(
    repo: &Repository,
    local_oid: git2::Oid,
    their: &AnnotatedCommit<'_>,
    merge_opts: &MergeOptions,
    nonce: &[u8; 16],
) -> Result<(git2::Oid, git2::Oid, ExpectedMergeMetadata), GitError> {
    let remote_oid = their.id();
    let local = repo.find_commit(local_oid)?;
    let remote = repo.find_commit(remote_oid)?;
    let mut index = repo.merge_commits(&local, &remote, Some(merge_opts))?;
    let metadata = ExpectedMergeMetadata::from_index(their, &index)?;
    let paths = collect_merge_paths(repo, &index, local_oid, remote_oid)?;
    let preview = MergePreview::create(repo, nonce)?;
    let mut checkout = CheckoutBuilder::new();
    checkout
        .target_dir(&preview.path)
        .force()
        .update_index(false)
        .refresh(false)
        .our_label("HEAD")
        .their_label(&remote_oid.to_string());
    repo.checkout_index(Some(&mut index), Some(&mut checkout))?;
    let (index_fingerprint, worktree_fingerprint) =
        merge_fingerprints_for_index(&index, &preview.path, paths)?;
    Ok((index_fingerprint, worktree_fingerprint, metadata))
}

fn merge_head_oid(repo: &Repository, state: git2::RepositoryState) -> Result<git2::Oid, GitError> {
    std::fs::read_to_string(repo.path().join("MERGE_HEAD"))
        .ok()
        .and_then(|contents| {
            let mut lines = contents.lines();
            let oid = lines
                .next()
                .and_then(|line| git2::Oid::from_str(line).ok())?;
            lines.next().is_none().then_some(oid)
        })
        .ok_or_else(|| manual_recovery(state, "MERGE_HEAD cannot be verified"))
}

fn bind_merge_metadata(
    repo: &Repository,
    nonce: &[u8; 16],
    expected: &ExpectedMergeMetadata,
    state: git2::RepositoryState,
) -> Result<git2::Oid, GitError> {
    verify_expected_merge_metadata(repo, expected, state)?;
    let owned = expected.with_nonce(nonce);
    let path = repo.path().join("MERGE_MSG");
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .map_err(|error| {
            GitError::Other(format!(
                "cannot open merge metadata {} to bind ownership: {error}",
                path.display()
            ))
        })?;
    if !expected.message.ends_with(b"\n") {
        file.write_all(b"\n").map_err(|error| {
            GitError::Other(format!(
                "cannot extend merge metadata {}: {error}",
                path.display()
            ))
        })?;
    }
    file.write_all(format!("{MERGE_OPERATION_NONCE_PREFIX}{}\n", encode_nonce(nonce)).as_bytes())
        .map_err(|error| {
            GitError::Other(format!(
                "cannot bind ownership into merge metadata {}: {error}",
                path.display()
            ))
        })?;
    file.sync_all().map_err(|error| {
        GitError::Other(format!(
            "cannot sync merge ownership metadata {}: {error}",
            path.display()
        ))
    })?;
    verify_expected_merge_metadata(repo, &owned, state)?;
    merge_metadata_fingerprint(&owned)
}

fn verify_merge_nonce(
    repo: &Repository,
    nonce: &[u8; 16],
    state: git2::RepositoryState,
) -> Result<(), GitError> {
    let contents = std::fs::read_to_string(repo.path().join("MERGE_MSG"))
        .map_err(|_| manual_recovery(state, "the current merge lacks its ownership nonce"))?;
    let found = contents
        .lines()
        .filter_map(|line| line.strip_prefix(MERGE_OPERATION_NONCE_PREFIX))
        .map(parse_nonce)
        .collect::<Vec<_>>();
    if found.as_slice() != [Some(*nonce)] {
        return Err(manual_recovery(
            state,
            "the current merge nonce does not match the Hatchdoor ownership marker",
        ));
    }
    Ok(())
}

fn verify_merge_metadata(
    repo: &Repository,
    marker: MergeRecoveryMarker,
    state: git2::RepositoryState,
) -> Result<(), GitError> {
    verify_merge_nonce(repo, &marker.nonce, state)?;
    let actual = read_merge_metadata(repo).map_err(|error| {
        manual_recovery(
            state,
            format!("the current merge metadata cannot be verified: {error}"),
        )
    })?;
    let fingerprint = merge_metadata_fingerprint(&actual).map_err(|error| {
        manual_recovery(
            state,
            format!("the current merge metadata cannot be fingerprinted: {error}"),
        )
    })?;
    if Some(fingerprint) != marker.metadata_fingerprint {
        return Err(manual_recovery(
            state,
            "the cleanup-sensitive merge metadata changed after Hatchdoor's merge",
        ));
    }
    Ok(())
}

fn verify_active_merge_snapshot(
    repo: &Repository,
    marker: MergeRecoveryMarker,
    state: git2::RepositoryState,
) -> Result<git2::Oid, GitError> {
    if state != git2::RepositoryState::Merge || marker.phase != MergeMarkerPhase::Active {
        return Err(manual_recovery(
            state,
            "the Hatchdoor merge marker is not active for the current operation",
        ));
    }
    verify_merge_metadata(repo, marker, state)?;
    if merge_head_oid(repo, state)? != marker.remote_oid {
        return Err(manual_recovery(
            state,
            "MERGE_HEAD does not match the owned remote commit",
        ));
    }
    let head_oid = repo
        .head()
        .ok()
        .and_then(|head| head.target())
        .ok_or_else(|| manual_recovery(state, "HEAD does not identify a commit"))?;
    if head_oid != marker.local_oid {
        let head_commit = repo
            .find_commit(head_oid)
            .map_err(|_| manual_recovery(state, "the current HEAD commit cannot be verified"))?;
        if head_commit.parent_count() != 2
            || head_commit.parent_id(0).ok() != Some(marker.local_oid)
            || head_commit.parent_id(1).ok() != Some(marker.remote_oid)
        {
            return Err(manual_recovery(
                state,
                "HEAD is neither the owned local commit nor its completed merge commit",
            ));
        }
    }
    let (index_fingerprint, worktree_fingerprint) =
        merge_state_fingerprints(repo, marker.local_oid, marker.remote_oid).map_err(|error| {
            manual_recovery(
                state,
                format!("the owned merge snapshot cannot be verified: {error}"),
            )
        })?;
    if Some(index_fingerprint) != marker.index_fingerprint
        || Some(worktree_fingerprint) != marker.worktree_fingerprint
    {
        return Err(manual_recovery(
            state,
            "the index or tracked worktree changed after Hatchdoor's merge",
        ));
    }
    Ok(head_oid)
}

#[cfg(test)]
fn begin_owned_merge(
    repo: &Repository,
    their: &AnnotatedCommit<'_>,
    local_oid: git2::Oid,
    merge_opts: &mut MergeOptions,
) -> Result<(), GitError> {
    begin_owned_merge_with_post_merge(repo, their, local_oid, merge_opts, |_| Ok(()))
}

fn begin_owned_merge_with_post_merge<F>(
    repo: &Repository,
    their: &AnnotatedCommit<'_>,
    local_oid: git2::Oid,
    merge_opts: &mut MergeOptions,
    post_merge: F,
) -> Result<(), GitError>
where
    F: FnOnce(&Repository) -> Result<(), GitError>,
{
    if repo.state() != git2::RepositoryState::Clean {
        return Err(manual_recovery(
            repo.state(),
            "Hatchdoor will not start a merge while another Git operation is active",
        ));
    }
    clear_merge_marker(repo)?;
    let nonce = fresh_operation_nonce()?;
    let (index_fingerprint, worktree_fingerprint, expected_metadata) =
        expected_merge_fingerprints(repo, local_oid, their, merge_opts, &nonce)?;
    let prepared = MergeRecoveryMarker {
        phase: MergeMarkerPhase::Prepared,
        nonce,
        local_oid,
        remote_oid: their.id(),
        index_fingerprint: None,
        worktree_fingerprint: None,
        metadata_fingerprint: None,
    };
    write_merge_marker(repo, prepared)?;
    if let Err(error) = repo.merge(&[their], Some(merge_opts), None) {
        if repo.state() == git2::RepositoryState::Clean {
            clear_merge_marker(repo)?;
        }
        return Err(error.into());
    }
    post_merge(repo)?;
    let state = repo.state();
    if state != git2::RepositoryState::Merge
        || repo.head().ok().and_then(|head| head.target()) != Some(local_oid)
        || merge_head_oid(repo, state)? != their.id()
    {
        return Err(manual_recovery(
            state,
            "the operation created by repo.merge does not match Hatchdoor's prepared marker",
        ));
    }
    let actual = merge_state_fingerprints(repo, local_oid, their.id()).map_err(|error| {
        manual_recovery(
            state,
            format!("the live merge result cannot be compared to its expected baseline: {error}"),
        )
    })?;
    if actual != (index_fingerprint, worktree_fingerprint) {
        return Err(manual_recovery(
            state,
            "the live merge changed before Hatchdoor could activate ownership",
        ));
    }
    let metadata_fingerprint = bind_merge_metadata(repo, &nonce, &expected_metadata, state)?;
    write_merge_marker(
        repo,
        MergeRecoveryMarker {
            phase: MergeMarkerPhase::Active,
            index_fingerprint: Some(index_fingerprint),
            worktree_fingerprint: Some(worktree_fingerprint),
            metadata_fingerprint: Some(metadata_fingerprint),
            ..prepared
        },
    )
}

/// If a previous operation (typically a merge) was interrupted — e.g. the
/// process was killed after `repo.merge()` but before `cleanup_state()` — the
/// repository is left in a non-`Clean` state with a half-merged, possibly
/// conflicted index. Every later sync would then fail at `write_tree` with an
/// opaque "not fully merged index" error and the subsystem would be wedged
/// until a human ran `git merge --abort`. Reset the working tree/index back to
/// HEAD and clear the in-progress state so the sync starts from a clean slate;
/// the remote integration is simply redone. Returns the state we recovered
/// from, if any.
fn recover_interrupted_state(repo: &Repository) -> Result<Option<git2::RepositoryState>, GitError> {
    let state = repo.state();
    if state == git2::RepositoryState::Clean {
        clear_merge_marker(repo)?;
        return Ok(None);
    }

    if state != git2::RepositoryState::Merge {
        return Err(manual_recovery(
            state,
            "Hatchdoor only creates merge operations",
        ));
    }
    let marker = read_merge_marker(repo, state)?;
    if marker.phase != MergeMarkerPhase::Active {
        return Err(manual_recovery(
            state,
            "the ownership marker was not activated after the merge began",
        ));
    }
    let head_oid = verify_active_merge_snapshot(repo, marker, state)?;

    if head_oid == marker.local_oid {
        let head_commit = repo.find_commit(head_oid)?;
        repo.reset(head_commit.as_object(), ResetType::Hard, None)?;
    }
    // When HEAD is already the verified two-parent merge commit, only
    // operation metadata remains; no checkout/reset is needed.
    repo.cleanup_state()?;
    clear_merge_marker(repo)?;
    Ok(Some(state))
}

/// Local phase: Remote mode may recover an interrupted integration; all modes
/// stage only the Vault subtree and commit if it differs from HEAD. It never
/// contacts a remote, so callers hold the vault-write lock across this. Returns
/// whether a commit was made and whether the remote phases are needed.
///
/// `paths` (the current write batch) is retained for the `SyncOps` contract and
/// the commit message, but staging covers all detected Vault-subtree drift, not
/// merely the batch — see `commit_working_tree`.
pub fn commit_local(
    config: &GitConfig,
    paths: &[PathBuf],
    message: &str,
) -> Result<CommitOutcome, GitError> {
    let _ = paths;
    let repo = Repository::discover(&config.vault_path)?;
    // Interrupted merge recovery hard-resets the checkout. It is restricted to
    // remote integration; local history must preserve every manual edit.
    if config.mode == GitMode::Remote
        && let Some(state) = recover_interrupted_state(&repo)?
    {
        tracing::warn!("git sync: recovered repository from interrupted {state:?} state");
    }

    let committed = commit_working_tree(&repo, config, message)?;
    let needs_remote = config.mode == GitMode::Remote && (committed || has_unpushed(config)?);
    Ok(CommitOutcome {
        committed,
        needs_remote,
    })
}

/// Network read phase: fetch the configured branch. Only reads/writes `.git`
/// (remote-tracking refs and the object store); it does NOT touch the working
/// tree, so it is safe — and important — to run WITHOUT the vault-write lock so
/// a slow or hanging remote cannot block concurrent vault writes.
pub fn fetch_remote(config: &GitConfig) -> Result<(), GitError> {
    let repo = Repository::open(&config.vault_path)?;
    let mut remote = repo.find_remote(&config.remote)?;
    let mut fetch_opts = FetchOptions::new();
    fetch_opts.remote_callbacks(remote_callbacks(config));
    remote
        .fetch(&[&config.branch], Some(&mut fetch_opts), None)
        .map_err(|e| GitError::Remote(e.message().to_string()))?;
    Ok(())
}

/// Integrate phase: if the already-fetched remote-tracking ref is ahead, merge
/// it into the local branch (may write the working tree via a checkout), so
/// callers hold the vault-write lock across this. Assumes `fetch_remote` ran.
pub fn integrate_fetched(config: &GitConfig) -> Result<(), GitError> {
    let repo = Repository::open(&config.vault_path)?;
    // A write may have raced into the lock-free fetch window, or a note may have
    // been edited by hand directly on the server. Commit any such pending
    // working-tree changes before merging: they are the source of truth on disk,
    // so they must not be discarded by the merge's force checkout, and an
    // uncommitted tracked edit must not block the merge (and thus every push).
    commit_working_tree(&repo, config, "hatchdoor: local vault changes")?;
    let local_oid = repo.refname_to_id(&format!("refs/heads/{}", config.branch))?;
    let remote_ref = format!("refs/remotes/{}/{}", config.remote, config.branch);
    let remote_oid = match repo.refname_to_id(&remote_ref) {
        Ok(oid) => oid,
        Err(_) => return Ok(()), // remote has no such branch yet; push will create it
    };

    let (_ahead, behind) = repo.graph_ahead_behind(local_oid, remote_oid)?;
    if behind == 0 {
        return Ok(()); // we are up to date or strictly ahead; push will fast-forward
    }

    let their = repo.find_annotated_commit(remote_oid)?;
    merge_remote(&repo, config, &their, local_oid)
}

/// Network write phase: push the local branch to the remote. Reads local refs
/// and uploads objects; does NOT touch the working tree, so it runs WITHOUT the
/// vault-write lock.
pub fn push_branch(config: &GitConfig) -> Result<(), GitError> {
    let repo = Repository::open(&config.vault_path)?;
    push(&repo, config)
}

/// Stage the given absolute paths, commit, fetch, integrate, and push.
///
/// This composes the phase functions above in order. The background task calls
/// the phases directly so it can hold the vault-write lock only across the
/// local/working-tree phases (`commit_local`, `integrate_fetched`) and release
/// it across the network phases (`fetch_remote`, `push_branch`).
pub fn sync(config: &GitConfig, paths: &[PathBuf], message: &str) -> Result<SyncReport, GitError> {
    let commit = commit_local(config, paths, message)?;
    if config.mode == GitMode::Local {
        return Ok(SyncReport {
            outcome: SyncOutcome::Committed {
                committed: commit.committed,
            },
        });
    }
    if !commit.needs_remote {
        return Ok(SyncReport {
            outcome: SyncOutcome::NoChanges,
        });
    }

    fetch_remote(config)?;
    integrate_fetched(config)?;
    push_branch(config)?;

    Ok(SyncReport {
        outcome: SyncOutcome::Pushed {
            committed: commit.committed,
        },
    })
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

fn merge_remote(
    repo: &Repository,
    config: &GitConfig,
    their: &AnnotatedCommit,
    local_oid: git2::Oid,
) -> Result<(), GitError> {
    merge_remote_with_post_merge(repo, config, their, local_oid, |_| Ok(()))
}

fn merge_remote_with_post_merge<F>(
    repo: &Repository,
    config: &GitConfig,
    their: &AnnotatedCommit,
    local_oid: git2::Oid,
    post_merge: F,
) -> Result<(), GitError>
where
    F: FnOnce(&Repository) -> Result<(), GitError>,
{
    // The caller committed pending manual edits before entering this function.
    // The ownership marker additionally fingerprints the merge result so an
    // external edit or resolution causes a safe refusal before any reset.
    let mut merge_opts = MergeOptions::new();
    begin_owned_merge_with_post_merge(repo, their, local_oid, &mut merge_opts, post_merge)?;

    let mut index = repo.index()?;
    if index.has_conflicts() {
        // Collect conflicting paths for the error, then abort cleanly.
        let mut files = Vec::new();
        if let Ok(conflicts) = index.conflicts() {
            for conflict in conflicts.flatten() {
                if let Some(entry) = conflict.our.or(conflict.their)
                    && let Ok(path) = std::str::from_utf8(&entry.path)
                {
                    files.push(path.to_string());
                }
            }
        }
        // Abort: hard-reset back to our commit first, then clear merge state.
        let marker = read_merge_marker(repo, repo.state())?;
        verify_active_merge_snapshot(repo, marker, repo.state())?;
        let our_commit = repo.find_commit(local_oid)?;
        repo.reset(our_commit.as_object(), ResetType::Hard, None)?;
        repo.cleanup_state()?;
        clear_merge_marker(repo)?;
        return Err(GitError::Conflict { files });
    }

    // Clean merge: write tree and create a two-parent merge commit.
    let marker = read_merge_marker(repo, repo.state())?;
    verify_active_merge_snapshot(repo, marker, repo.state())?;
    let tree_oid = index.write_tree()?;
    let tree = repo.find_tree(tree_oid)?;
    let sig = signature(config)?;
    let our_commit = repo.find_commit(local_oid)?;
    let their_commit = repo.find_commit(their.id())?;
    repo.commit(
        Some("HEAD"),
        &sig,
        &sig,
        &format!(
            "Merge remote {}/{} into {}",
            config.remote, config.branch, config.branch
        ),
        &tree,
        &[&our_commit, &their_commit],
    )?;
    repo.cleanup_state()?;
    clear_merge_marker(repo)?;
    // `repo.merge` already wrote the verified merge result to the worktree.
    // Avoid a force checkout here: an editor or manual Git process does not
    // acquire Hatchdoor's process-local vault lock.
    Ok(())
}

fn push(repo: &Repository, config: &GitConfig) -> Result<(), GitError> {
    let mut remote = repo.find_remote(&config.remote)?;
    let mut push_opts = PushOptions::new();
    push_opts.remote_callbacks(remote_callbacks(config));
    let refspec = format!("refs/heads/{0}:refs/heads/{0}", config.branch);
    remote
        .push(&[&refspec], Some(&mut push_opts))
        .map_err(|e| GitError::Remote(e.message().to_string()))?;
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

        let report = sync(&config, &[], "local Vault history").expect("commit subtree");
        assert_eq!(
            report.outcome,
            SyncOutcome::Committed { committed: true },
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
                .expect("operator index after sync")
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

        sync(&config, &[], "local Vault history").expect("commit working tree");

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
            .expect("operator index after sync")
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

    fn remote_head_message(remote_dir: &Path, branch: &str) -> String {
        let repo = Repository::open_bare(remote_dir).unwrap();
        let oid = repo.refname_to_id(&format!("refs/heads/{branch}")).unwrap();
        repo.find_commit(oid)
            .unwrap()
            .message()
            .unwrap()
            .to_string()
    }

    /// Make a second clone of the remote, change `Home.md`, and push, so the
    /// remote moves ahead of our working clone.
    fn advance_remote(remote_dir: &Path, contents: &str) {
        let tmp = TempDir::new().unwrap();
        let clone = tmp.path().join("other");
        let repo = Repository::clone(remote_dir.to_str().unwrap(), &clone).unwrap();
        fs::write(clone.join("Home.md"), contents).unwrap();
        commit_all(&repo, "remote edit");
        let mut remote = repo.find_remote("origin").unwrap();
        remote
            .push(&["refs/heads/main:refs/heads/main"], None)
            .unwrap();
        // Keep tmp alive until push completes.
        drop(remote);
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
    fn sync_commits_and_pushes_new_file() {
        let (_tmp, work, remote) = init_repo_with_remote();
        let config = base_config(&work);
        let new_file = work.join("Note.md");
        fs::write(&new_file, "# Note\n").unwrap();

        let report = sync(
            &config,
            std::slice::from_ref(&new_file),
            "hatchdoor: add Note",
        )
        .unwrap();
        assert_eq!(report.outcome, SyncOutcome::Pushed { committed: true });
        assert_eq!(remote_head_message(&remote, "main"), "hatchdoor: add Note");
    }

    #[test]
    fn sync_stages_deletion() {
        let (_tmp, work, remote) = init_repo_with_remote();
        let config = base_config(&work);
        let home = work.join("Home.md");
        fs::remove_file(&home).unwrap();

        let report = sync(
            &config,
            std::slice::from_ref(&home),
            "hatchdoor: delete Home",
        )
        .unwrap();
        assert_eq!(report.outcome, SyncOutcome::Pushed { committed: true });
        // Remote no longer has the file in its tree.
        let repo = Repository::open_bare(&remote).unwrap();
        let oid = repo.refname_to_id("refs/heads/main").unwrap();
        let tree = repo.find_commit(oid).unwrap().tree().unwrap();
        assert!(tree.get_name("Home.md").is_none());
    }

    #[test]
    fn sync_no_changes_is_noop() {
        let (_tmp, work, _remote) = init_repo_with_remote();
        let config = base_config(&work);
        let report = sync(&config, &[], "nothing").unwrap();
        assert_eq!(report.outcome, SyncOutcome::NoChanges);
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

        let report = sync(&config, &[vault.join("Home.md")], "hatchdoor: local Home")
            .expect("commit locally");
        assert_eq!(report.outcome, SyncOutcome::Committed { committed: true });
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

    #[test]
    fn sync_clean_merge_when_remote_changed_other_file() {
        let (_tmp, work, remote) = init_repo_with_remote();
        let config = base_config(&work);
        // Remote adds an unrelated change to Home.md.
        advance_remote(&remote, "# Home\nremote line\n");
        // We add a brand new file locally.
        let note = work.join("Local.md");
        fs::write(&note, "# Local\n").unwrap();

        let report = sync(&config, &[note], "hatchdoor: add Local").unwrap();
        assert_eq!(report.outcome, SyncOutcome::Pushed { committed: true });
        // Remote tree now contains both files.
        let repo = Repository::open_bare(&remote).unwrap();
        let oid = repo.refname_to_id("refs/heads/main").unwrap();
        let tree = repo.find_commit(oid).unwrap().tree().unwrap();
        assert!(tree.get_name("Local.md").is_some());
        assert!(tree.get_name("Home.md").is_some());
    }

    #[test]
    fn sync_conflict_aborts_keeps_local_commit_and_does_not_push() {
        let (_tmp, work, remote) = init_repo_with_remote();
        let config = base_config(&work);
        // Remote changes Home.md line 1.
        advance_remote(&remote, "# Home\nremote version\n");
        // We change the SAME line differently and sync.
        fs::write(work.join("Home.md"), "# Home\nlocal version\n").unwrap();

        let err = sync(&config, &[work.join("Home.md")], "hatchdoor: edit Home").unwrap_err();
        match err {
            GitError::Conflict { files } => assert!(files.iter().any(|f| f == "Home.md")),
            other => panic!("expected conflict, got {other:?}"),
        }

        // Local working tree is restored to our committed content.
        let restored = fs::read_to_string(work.join("Home.md")).unwrap();
        assert_eq!(restored, "# Home\nlocal version\n");

        // Local HEAD has our commit; remote was NOT advanced to it.
        let repo = Repository::open(&work).unwrap();
        let head = repo.refname_to_id("refs/heads/main").unwrap();
        let head_msg = repo
            .find_commit(head)
            .unwrap()
            .message()
            .unwrap()
            .to_string();
        assert_eq!(head_msg, "hatchdoor: edit Home");
        assert_eq!(remote_head_message(&remote, "main"), "remote edit");
    }

    #[test]
    fn unpushed_count_is_zero_after_successful_push() {
        let (_tmp, work, _remote) = init_repo_with_remote();
        let config = base_config(&work);
        let note = work.join("Note.md");
        fs::write(&note, "# Note\n").unwrap();
        sync(&config, &[note], "hatchdoor: add Note").unwrap();
        assert_eq!(unpushed_count(&config).unwrap(), 0);
    }

    #[test]
    fn unpushed_count_reflects_commit_stranded_by_conflict() {
        let (_tmp, work, remote) = init_repo_with_remote();
        let config = base_config(&work);
        advance_remote(&remote, "# Home\nremote version\n");
        fs::write(work.join("Home.md"), "# Home\nlocal version\n").unwrap();
        // Conflict aborts but keeps our local commit, which is now unpushed.
        let _ = sync(&config, &[work.join("Home.md")], "hatchdoor: edit Home").unwrap_err();
        assert_eq!(unpushed_count(&config).unwrap(), 1);
    }

    #[test]
    fn sync_recovers_from_interrupted_merge_state() {
        let (_tmp, work, remote) = init_repo_with_remote();
        let config = base_config(&work);

        // Simulate a crash mid-merge: the remote and local both change the same
        // line, we start a merge (which records a conflicted index and puts the
        // repo into RepositoryState::Merge) and then are "killed" before
        // committing or calling cleanup_state().
        {
            advance_remote(&remote, "# Home\nremote line\n");
            let repo = Repository::open(&work).unwrap();
            fs::write(work.join("Home.md"), "# Home\nlocal line\n").unwrap();
            commit_all(&repo, "local edit");
            let mut rmt = repo.find_remote("origin").unwrap();
            rmt.fetch(&["main"], None, None).unwrap();
            drop(rmt);
            let remote_oid = repo.refname_to_id("refs/remotes/origin/main").unwrap();
            let their = repo.find_annotated_commit(remote_oid).unwrap();
            let local_oid = repo.head().unwrap().target().unwrap();
            let mut merge_opts = MergeOptions::new();
            begin_owned_merge(&repo, &their, local_oid, &mut merge_opts).unwrap();
            assert_ne!(
                repo.state(),
                git2::RepositoryState::Clean,
                "precondition: repo should be left in a merge state"
            );
        }

        // A later sync must not be permanently wedged at write_tree on the
        // half-merged index. It should recover (reset to HEAD + cleanup_state),
        // then surface the genuine divergence as a clean Conflict — not an opaque
        // "not fully merged" Other error — and leave the repo Clean.
        let err = sync(&config, &[], "hatchdoor: later sync").unwrap_err();
        assert!(
            matches!(err, GitError::Conflict { .. }),
            "expected a clean Conflict after recovery, got {err:?}"
        );
        let repo = Repository::open(&work).unwrap();
        assert_eq!(
            repo.state(),
            git2::RepositoryState::Clean,
            "repo should be left in a clean state, not wedged in Merge"
        );
    }

    #[test]
    fn commit_local_preserves_manual_merge_and_staged_resolution() {
        let (_tmp, work, remote) = init_repo_with_remote();
        let config = base_config(&work);

        advance_remote(&remote, "# Home\nremote line\n");
        let repo = Repository::open(&work).unwrap();
        fs::write(work.join("Home.md"), "# Home\nlocal line\n").unwrap();
        commit_all(&repo, "local edit");
        let mut origin = repo.find_remote("origin").unwrap();
        origin.fetch(&["main"], None, None).unwrap();
        drop(origin);
        let remote_oid = repo.refname_to_id("refs/remotes/origin/main").unwrap();
        let their = repo.find_annotated_commit(remote_oid).unwrap();
        repo.merge(&[&their], None, None).unwrap();
        assert_eq!(repo.state(), git2::RepositoryState::Merge);

        let resolution = "# Home\nmanual resolution that must survive\n";
        fs::write(work.join("Home.md"), resolution).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("Home.md")).unwrap();
        index.write().unwrap();
        let staged_oid = repo
            .index()
            .unwrap()
            .get_path(Path::new("Home.md"), 0)
            .expect("staged resolution")
            .id;

        let result = commit_local(&config, &[], "hatchdoor: later sync");
        assert!(
            result.is_err(),
            "manual in-progress Git work must require manual recovery"
        );
        assert_eq!(
            fs::read_to_string(work.join("Home.md")).unwrap(),
            resolution,
            "manual resolution must not be overwritten"
        );
        assert_eq!(
            Repository::open(&work).unwrap().state(),
            git2::RepositoryState::Merge,
            "manual merge metadata must remain intact"
        );
        assert_eq!(
            Repository::open(&work)
                .unwrap()
                .index()
                .unwrap()
                .get_path(Path::new("Home.md"), 0)
                .expect("staged resolution remains")
                .id,
            staged_oid,
            "manual staged resolution must remain intact"
        );
    }

    #[test]
    fn commit_local_refuses_owned_merge_changed_after_interruption() {
        let (_tmp, work, remote) = init_repo_with_remote();
        let config = base_config(&work);

        advance_remote(&remote, "# Home\nremote line\n");
        let repo = Repository::open(&work).unwrap();
        fs::write(work.join("Home.md"), "# Home\nlocal line\n").unwrap();
        commit_all(&repo, "local edit");
        let mut origin = repo.find_remote("origin").unwrap();
        origin.fetch(&["main"], None, None).unwrap();
        drop(origin);
        let remote_oid = repo.refname_to_id("refs/remotes/origin/main").unwrap();
        let their = repo.find_annotated_commit(remote_oid).unwrap();
        let local_oid = repo.head().unwrap().target().unwrap();
        let mut merge_opts = MergeOptions::new();
        begin_owned_merge(&repo, &their, local_oid, &mut merge_opts).unwrap();

        // The marker proves who started the operation, but it must not grant
        // permission to erase work added afterward by a human.
        let resolution = "# Home\nmanual resolution after interruption\n";
        fs::write(work.join("Home.md"), resolution).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("Home.md")).unwrap();
        index.write().unwrap();
        let staged_oid = repo
            .index()
            .unwrap()
            .get_path(Path::new("Home.md"), 0)
            .expect("staged resolution")
            .id;

        let error = commit_local(&config, &[], "hatchdoor: later sync").unwrap_err();
        assert!(
            matches!(error, GitError::ManualRecovery { .. }),
            "expected manual recovery, got {error:?}"
        );
        assert_eq!(
            fs::read_to_string(work.join("Home.md")).unwrap(),
            resolution
        );
        assert_eq!(
            Repository::open(&work).unwrap().state(),
            git2::RepositoryState::Merge
        );
        assert_eq!(
            Repository::open(&work)
                .unwrap()
                .index()
                .unwrap()
                .get_path(Path::new("Home.md"), 0)
                .expect("staged resolution remains")
                .id,
            staged_oid
        );
    }

    #[test]
    fn merge_refuses_mutation_before_ownership_activation_without_cleanup() {
        let (_tmp, work, remote) = init_repo_with_remote();
        let config = base_config(&work);

        advance_remote(&remote, "# Home\nremote line\n");
        let repo = Repository::open(&work).unwrap();
        fs::write(work.join("Home.md"), "# Home\nlocal line\n").unwrap();
        commit_all(&repo, "local edit");
        let mut origin = repo.find_remote("origin").unwrap();
        origin.fetch(&["main"], None, None).unwrap();
        drop(origin);
        let remote_oid = repo.refname_to_id("refs/remotes/origin/main").unwrap();
        let their = repo.find_annotated_commit(remote_oid).unwrap();
        let local_oid = repo.head().unwrap().target().unwrap();

        // Deterministically model an editor racing the process after
        // `repo.merge` has exposed its conflicted worktree but before
        // Hatchdoor activates its ownership evidence.
        let manual_contents = b"# Home\nmanual edit in activation window\n";
        let mut interrupted_snapshot = None;
        let error =
            merge_remote_with_post_merge(&repo, &config, &their, local_oid, |merging_repo| {
                fs::write(work.join("Home.md"), manual_contents).unwrap();
                interrupted_snapshot = Some((
                    fs::read(merging_repo.path().join("index")).unwrap(),
                    fs::read(merging_repo.path().join("MERGE_HEAD")).unwrap(),
                    fs::read(merging_repo.path().join("MERGE_MSG")).unwrap(),
                    fs::read(merge_marker_path(merging_repo)).unwrap(),
                ));
                Ok(())
            })
            .unwrap_err();

        assert!(
            matches!(error, GitError::ManualRecovery { .. }),
            "expected manual recovery, got {error:?}"
        );
        assert_eq!(repo.state(), git2::RepositoryState::Merge);
        assert_eq!(fs::read(work.join("Home.md")).unwrap(), manual_contents);
        let (index, merge_head, merge_message, ownership_marker) = interrupted_snapshot.unwrap();
        assert_eq!(fs::read(repo.path().join("index")).unwrap(), index);
        assert_eq!(
            fs::read(repo.path().join("MERGE_HEAD")).unwrap(),
            merge_head
        );
        assert_eq!(
            fs::read(repo.path().join("MERGE_MSG")).unwrap(),
            merge_message
        );
        assert_eq!(
            fs::read(merge_marker_path(&repo)).unwrap(),
            ownership_marker
        );
        assert_eq!(
            read_merge_marker(&repo, repo.state()).unwrap().phase,
            MergeMarkerPhase::Prepared
        );
        assert_eq!(repo.head().unwrap().target(), Some(local_oid));
    }

    #[cfg(unix)]
    #[test]
    fn merge_refuses_chmod_before_ownership_activation_without_cleanup() {
        use std::os::unix::fs::PermissionsExt;

        let (_tmp, work, remote) = init_repo_with_remote();
        let config = base_config(&work);

        advance_remote(&remote, "# Home\nremote line\n");
        let repo = Repository::open(&work).unwrap();
        fs::write(work.join("Home.md"), "# Home\nlocal line\n").unwrap();
        commit_all(&repo, "local edit");
        let mut origin = repo.find_remote("origin").unwrap();
        origin.fetch(&["main"], None, None).unwrap();
        drop(origin);
        let remote_oid = repo.refname_to_id("refs/remotes/origin/main").unwrap();
        let their = repo.find_annotated_commit(remote_oid).unwrap();
        let local_oid = repo.head().unwrap().target().unwrap();

        let mut interrupted_snapshot = None;
        let error =
            merge_remote_with_post_merge(&repo, &config, &their, local_oid, |merging_repo| {
                let path = work.join("Home.md");
                let mut permissions = fs::metadata(&path).unwrap().permissions();
                permissions.set_mode(permissions.mode() | 0o111);
                fs::set_permissions(&path, permissions).unwrap();
                interrupted_snapshot = Some((
                    fs::read(&path).unwrap(),
                    fs::metadata(&path).unwrap().permissions().mode(),
                    fs::read(merging_repo.path().join("index")).unwrap(),
                    fs::read(merging_repo.path().join("MERGE_HEAD")).unwrap(),
                    fs::read(merging_repo.path().join("MERGE_MSG")).unwrap(),
                    fs::read(merge_marker_path(merging_repo)).unwrap(),
                ));
                Ok(())
            })
            .unwrap_err();

        assert!(
            matches!(error, GitError::ManualRecovery { .. }),
            "expected manual recovery, got {error:?}"
        );
        assert_eq!(repo.state(), git2::RepositoryState::Merge);
        let (contents, mode, index, merge_head, merge_message, ownership_marker) =
            interrupted_snapshot.unwrap();
        assert_eq!(fs::read(work.join("Home.md")).unwrap(), contents);
        assert_eq!(
            fs::metadata(work.join("Home.md"))
                .unwrap()
                .permissions()
                .mode(),
            mode
        );
        assert_eq!(fs::read(repo.path().join("index")).unwrap(), index);
        assert_eq!(
            fs::read(repo.path().join("MERGE_HEAD")).unwrap(),
            merge_head
        );
        assert_eq!(
            fs::read(repo.path().join("MERGE_MSG")).unwrap(),
            merge_message
        );
        assert_eq!(
            fs::read(merge_marker_path(&repo)).unwrap(),
            ownership_marker
        );
        assert_eq!(repo.head().unwrap().target(), Some(local_oid));
    }

    #[test]
    fn commit_local_preserves_manual_merge_message_edit_with_owned_nonce() {
        let (_tmp, work, remote) = init_repo_with_remote();
        let config = base_config(&work);

        advance_remote(&remote, "# Home\nremote line\n");
        let repo = Repository::open(&work).unwrap();
        fs::write(work.join("Home.md"), "# Home\nlocal line\n").unwrap();
        commit_all(&repo, "local edit");
        let mut origin = repo.find_remote("origin").unwrap();
        origin.fetch(&["main"], None, None).unwrap();
        drop(origin);
        let remote_oid = repo.refname_to_id("refs/remotes/origin/main").unwrap();
        let their = repo.find_annotated_commit(remote_oid).unwrap();
        let local_oid = repo.head().unwrap().target().unwrap();
        let mut merge_opts = MergeOptions::new();
        begin_owned_merge(&repo, &their, local_oid, &mut merge_opts).unwrap();

        let merge_message_path = repo.path().join("MERGE_MSG");
        let mut merge_message = fs::read(&merge_message_path).unwrap();
        assert!(
            String::from_utf8_lossy(&merge_message).contains(MERGE_OPERATION_NONCE_PREFIX),
            "precondition: the manual edit retains the owned nonce"
        );
        merge_message.extend_from_slice(b"manual merge message edit\n");
        fs::write(&merge_message_path, &merge_message).unwrap();
        let index = fs::read(repo.path().join("index")).unwrap();
        let merge_head = fs::read(repo.path().join("MERGE_HEAD")).unwrap();
        let ownership_marker = fs::read(merge_marker_path(&repo)).unwrap();
        let worktree = fs::read(work.join("Home.md")).unwrap();

        let error = commit_local(&config, &[], "hatchdoor: later sync").unwrap_err();
        assert!(
            matches!(error, GitError::ManualRecovery { .. }),
            "expected manual recovery, got {error:?}"
        );
        assert_eq!(repo.state(), git2::RepositoryState::Merge);
        assert_eq!(fs::read(work.join("Home.md")).unwrap(), worktree);
        assert_eq!(fs::read(repo.path().join("index")).unwrap(), index);
        assert_eq!(
            fs::read(repo.path().join("MERGE_HEAD")).unwrap(),
            merge_head
        );
        assert_eq!(fs::read(&merge_message_path).unwrap(), merge_message);
        assert_eq!(
            fs::read(merge_marker_path(&repo)).unwrap(),
            ownership_marker
        );
        assert_eq!(repo.head().unwrap().target(), Some(local_oid));
    }

    #[test]
    fn commit_local_rejects_replayed_marker_after_manual_abort_and_restart() {
        let (_tmp, work, remote) = init_repo_with_remote();
        let config = base_config(&work);

        advance_remote(&remote, "# Home\nremote line\n");
        let repo = Repository::open(&work).unwrap();
        fs::write(work.join("Home.md"), "# Home\nlocal line\n").unwrap();
        commit_all(&repo, "local edit");
        let mut origin = repo.find_remote("origin").unwrap();
        origin.fetch(&["main"], None, None).unwrap();
        drop(origin);
        let remote_oid = repo.refname_to_id("refs/remotes/origin/main").unwrap();
        let their = repo.find_annotated_commit(remote_oid).unwrap();
        let local_oid = repo.head().unwrap().target().unwrap();
        let mut merge_opts = MergeOptions::new();
        begin_owned_merge(&repo, &their, local_oid, &mut merge_opts).unwrap();

        // A user aborts Hatchdoor's interrupted merge. cleanup_state removes
        // Git's operation metadata but deliberately knows nothing about the
        // separate Hatchdoor marker, leaving it stale.
        let local_commit = repo.find_commit(local_oid).unwrap();
        repo.reset(local_commit.as_object(), ResetType::Hard, None)
            .unwrap();
        repo.cleanup_state().unwrap();
        assert_eq!(repo.state(), git2::RepositoryState::Clean);
        assert!(merge_marker_path(&repo).exists(), "stale marker remains");

        // The user manually restarts the identical merge. All deterministic
        // commit and merge-result values can match the stale active marker.
        repo.merge(&[&their], None, None).unwrap();
        assert_eq!(repo.state(), git2::RepositoryState::Merge);
        let resolution = "# Home\nmanual replay resolution must survive\n";
        fs::write(work.join("Home.md"), resolution).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("Home.md")).unwrap();
        index.write().unwrap();
        let staged_oid = repo
            .index()
            .unwrap()
            .get_path(Path::new("Home.md"), 0)
            .expect("staged resolution")
            .id;

        let error = commit_local(&config, &[], "hatchdoor: later sync").unwrap_err();
        match error {
            GitError::ManualRecovery { reason, .. } => assert!(
                reason.contains("nonce"),
                "the operation-specific nonce must reject marker replay before snapshot checks: {reason}"
            ),
            other => panic!("expected manual recovery, got {other:?}"),
        }
        assert_eq!(
            fs::read_to_string(work.join("Home.md")).unwrap(),
            resolution
        );
        assert_eq!(
            Repository::open(&work).unwrap().state(),
            git2::RepositoryState::Merge
        );
        assert_eq!(
            Repository::open(&work)
                .unwrap()
                .index()
                .unwrap()
                .get_path(Path::new("Home.md"), 0)
                .expect("staged resolution remains")
                .id,
            staged_oid
        );
    }

    #[test]
    fn sync_auto_commits_uncommitted_manual_edit_instead_of_refusing() {
        let (_tmp, work, remote) = init_repo_with_remote();
        let config = base_config(&work);

        // Remote moves ahead with an unrelated new file, so integrating it is a
        // clean merge that ends in a force checkout.
        {
            let tmp = TempDir::new().unwrap();
            let clone = tmp.path().join("other");
            let repo = Repository::clone(remote.to_str().unwrap(), &clone).unwrap();
            fs::write(clone.join("Remote.md"), "# Remote\n").unwrap();
            commit_all(&repo, "remote add Remote");
            let mut other_remote = repo.find_remote("origin").unwrap();
            other_remote
                .push(&["refs/heads/main:refs/heads/main"], None)
                .unwrap();
            drop(other_remote);
        }

        // A human edits a tracked file directly on the server, uncommitted.
        fs::write(work.join("Home.md"), "# Home\nhand edited\n").unwrap();

        // An MCP write to a different file triggers a sync. The manual edit is a
        // pending change to the vault (the source of truth), so it must be
        // committed and pushed — not refused forever, and not force-discarded.
        let local = work.join("Local.md");
        fs::write(&local, "# Local\n").unwrap();
        let report = sync(&config, &[local], "hatchdoor: add Local").unwrap();
        assert_eq!(report.outcome, SyncOutcome::Pushed { committed: true });

        // The manual edit survives locally and reached the remote.
        assert_eq!(
            fs::read_to_string(work.join("Home.md")).unwrap(),
            "# Home\nhand edited\n"
        );
        let repo = Repository::open_bare(&remote).unwrap();
        let oid = repo.refname_to_id("refs/heads/main").unwrap();
        let tree = repo.find_commit(oid).unwrap().tree().unwrap();
        let entry = tree.get_name("Home.md").expect("Home.md in remote tree");
        let obj = entry.to_object(&repo).unwrap();
        let blob = obj.as_blob().unwrap();
        assert!(
            std::str::from_utf8(blob.content())
                .unwrap()
                .contains("hand edited"),
            "manual edit should have been committed and pushed"
        );
    }

    #[test]
    fn sync_commits_uncommitted_vault_changes_not_in_the_batch() {
        // A vault file written to disk but stranded out of git (its batch's
        // commit failed, the process crashed before the debounced commit, or a
        // write raced the sync window) must be captured by a later sync even
        // when that sync's batch does not name it — here, an empty batch that
        // stands in for startup_flush.
        let (_tmp, work, remote) = init_repo_with_remote();
        let config = base_config(&work);
        fs::write(work.join("Stranded.md"), "# Stranded\n").unwrap();

        let report = sync(&config, &[], "hatchdoor: flush").unwrap();
        assert_eq!(report.outcome, SyncOutcome::Pushed { committed: true });

        let repo = Repository::open_bare(&remote).unwrap();
        let oid = repo.refname_to_id("refs/heads/main").unwrap();
        let tree = repo.find_commit(oid).unwrap().tree().unwrap();
        assert!(
            tree.get_name("Stranded.md").is_some(),
            "stranded vault file should have been committed and pushed"
        );
    }
}
