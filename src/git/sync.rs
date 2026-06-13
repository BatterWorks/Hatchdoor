use std::path::{Path, PathBuf};

use git2::{
    AnnotatedCommit, Cred, FetchOptions, MergeOptions, PushOptions, RemoteCallbacks, Repository,
    ResetType, Signature,
};

use super::config::GitConfig;

/// What a sync attempt did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncOutcome {
    /// Working tree matched HEAD and nothing was unpushed; no commit created.
    NoChanges,
    /// A commit was created and pushed (possibly after a clean merge).
    Pushed { committed: bool },
}

/// Result of a sync attempt, suitable for status reporting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncReport {
    pub outcome: SyncOutcome,
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
    match head.shorthand() {
        Some(name) if name == config.branch => {}
        other => {
            return Err(GitError::Validation(format!(
                "HEAD is on '{}', expected configured branch '{}'",
                other.unwrap_or("<unknown>"),
                config.branch
            )));
        }
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

fn same_path(a: &Path, b: &Path) -> bool {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    canon(a) == canon(b)
}

/// Stage the given absolute paths, commit, fetch, integrate, and push.
pub fn sync(config: &GitConfig, paths: &[PathBuf], message: &str) -> Result<SyncReport, GitError> {
    let repo = Repository::open(&config.vault_path)?;
    let workdir = repo
        .workdir()
        .ok_or_else(|| GitError::Other("repository is bare".to_string()))?
        .to_path_buf();

    let committed = stage_and_commit(&repo, config, &workdir, paths, message)?;

    if !committed && !has_unpushed(config)? {
        return Ok(SyncReport {
            outcome: SyncOutcome::NoChanges,
        });
    }

    integrate_remote(&repo, config)?;
    push(&repo, config)?;

    Ok(SyncReport {
        outcome: SyncOutcome::Pushed { committed },
    })
}

/// Stage the batch's paths and create a commit if the tree changed.
/// Returns true when a commit was created.
fn stage_and_commit(
    repo: &Repository,
    config: &GitConfig,
    workdir: &Path,
    paths: &[PathBuf],
    message: &str,
) -> Result<bool, GitError> {
    let mut index = repo.index()?;
    for absolute in paths {
        let relative = match absolute.strip_prefix(workdir) {
            Ok(rel) => rel,
            // Path outside the repo — ignore defensively.
            Err(_) => continue,
        };
        if absolute.exists() {
            index.add_path(relative)?;
        } else {
            // Deletions / rename sources: ignore "not in index" errors.
            let _ = index.remove_path(relative);
        }
    }
    index.write()?;

    let tree_oid = index.write_tree()?;
    let tree = repo.find_tree(tree_oid)?;

    let head_commit = repo.head().ok().and_then(|h| h.target());
    if let Some(parent_oid) = head_commit {
        let parent = repo.find_commit(parent_oid)?;
        if parent.tree()?.id() == tree_oid {
            return Ok(false); // nothing staged differs from HEAD
        }
        let sig = signature(config)?;
        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent])?;
    } else {
        let sig = signature(config)?;
        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[])?;
    }
    Ok(true)
}

/// Fetch the configured branch and, if the remote moved ahead, merge it into
/// the local branch. A conflicting merge aborts and returns `GitError::Conflict`.
fn integrate_remote(repo: &Repository, config: &GitConfig) -> Result<(), GitError> {
    let mut remote = repo.find_remote(&config.remote)?;
    let mut fetch_opts = FetchOptions::new();
    fetch_opts.remote_callbacks(remote_callbacks(config));
    remote
        .fetch(&[&config.branch], Some(&mut fetch_opts), None)
        .map_err(|e| GitError::Remote(e.message().to_string()))?;

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
    merge_remote(repo, config, &their, local_oid)
}

fn merge_remote(
    repo: &Repository,
    config: &GitConfig,
    their: &AnnotatedCommit,
    local_oid: git2::Oid,
) -> Result<(), GitError> {
    // The merge below ends in a force checkout that resets the working tree to
    // HEAD. If a tracked file was edited by hand on the server and never
    // committed (Hatchdoor only stages MCP-written paths), that force checkout
    // would silently discard it. Refuse instead so the edit can be preserved.
    let dirty = dirty_tracked_files(repo)?;
    if !dirty.is_empty() {
        return Err(GitError::DirtyWorkingTree { files: dirty });
    }

    let mut merge_opts = MergeOptions::new();
    repo.merge(&[their], Some(&mut merge_opts), None)?;

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
        let our_commit = repo.find_commit(local_oid)?;
        repo.reset(our_commit.as_object(), ResetType::Hard, None)?;
        repo.cleanup_state()?;
        return Err(GitError::Conflict { files });
    }

    // Clean merge: write tree and create a two-parent merge commit.
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
    // Make the working tree match the new HEAD.
    repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))?;
    Ok(())
}

/// Tracked files with uncommitted working-tree changes (modified, deleted,
/// renamed, or type-changed). Untracked and ignored files are excluded.
fn dirty_tracked_files(repo: &Repository) -> Result<Vec<String>, GitError> {
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(false).include_ignored(false);
    let statuses = repo.statuses(Some(&mut opts))?;
    let dirty_mask = git2::Status::WT_MODIFIED
        | git2::Status::WT_DELETED
        | git2::Status::WT_TYPECHANGE
        | git2::Status::WT_RENAMED;
    let mut files = Vec::new();
    for entry in statuses.iter() {
        if entry.status().intersects(dirty_mask)
            && let Some(path) = entry.path()
        {
            files.push(path.to_string());
        }
    }
    Ok(files)
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

    fn base_config(vault: &Path) -> GitConfig {
        GitConfig {
            vault_path: vault.to_path_buf(),
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
    fn sync_refuses_to_overwrite_uncommitted_manual_edit() {
        let (_tmp, work, remote) = init_repo_with_remote();
        let config = base_config(&work);

        // Remote moves ahead with an unrelated new file, so integrating it would
        // be a clean merge ending in a force checkout.
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

        // An MCP write to a different file triggers a sync.
        let local = work.join("Local.md");
        fs::write(&local, "# Local\n").unwrap();
        let err = sync(&config, &[local], "hatchdoor: add Local").unwrap_err();

        match err {
            GitError::DirtyWorkingTree { files } => {
                assert!(files.iter().any(|f| f == "Home.md"), "got {files:?}");
            }
            other => panic!("expected dirty working tree, got {other:?}"),
        }

        // The manual edit survives rather than being reset to HEAD.
        assert_eq!(
            fs::read_to_string(work.join("Home.md")).unwrap(),
            "# Home\nhand edited\n"
        );
    }
}
