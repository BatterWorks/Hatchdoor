//! Synchronization graphs for a validated, Vault-ID-owned managed checkout.

use std::path::{Path, PathBuf};

use git2::{
    Cred, FetchOptions, MergeOptions, PushOptions, RemoteCallbacks, Repository, ResetType,
    Signature,
};

use super::managed_checkout::ManagedHttpsCredentials;
use super::message::WriteLedger;
use crate::vault_registry::VaultId;

/// The two managed remote behaviors that share checkout synchronization.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManagedSyncMode {
    PullOnly,
    TwoWay,
}

/// Credential-safe input for one synchronization attempt of a checkout that
/// was already acquired and validated by the managed-checkout boundary.
#[derive(Clone, PartialEq, Eq)]
pub struct ManagedSyncConfig {
    pub repository_path: PathBuf,
    pub vault_path: PathBuf,
    pub repository_url: String,
    pub branch: String,
    pub mode: ManagedSyncMode,
    pub credentials: Option<ManagedHttpsCredentials>,
    pub author_name: String,
    pub author_email: String,
}

impl std::fmt::Debug for ManagedSyncConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ManagedSyncConfig")
            .field("repository_path", &self.repository_path)
            .field("vault_path", &self.vault_path)
            .field("repository_url", &self.repository_url)
            .field("branch", &self.branch)
            .field("mode", &self.mode)
            .field("credentials", &self.credentials)
            .field("author_name", &self.author_name)
            .field("author_email", &self.author_email)
            .finish()
    }
}

/// The graph outcome of one managed synchronization attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManagedSyncOutcome {
    UpToDate,
    PullOnlyFastForwarded,
    TwoWaySynchronized { committed: bool, integrated: bool },
}

/// A redacted, non-destructive managed synchronization failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManagedSyncError {
    Validation,
    DirtyWorkingCopy {
        files: Vec<String>,
    },
    LocalCommits {
        ahead: usize,
    },
    Conflict {
        files: Vec<String>,
    },
    PushRace,
    /// The remote accepted the push connection but refused to update the
    /// branch: a protected branch, a pre-receive hook, a quota. Carries the
    /// remote's own one-line reason. Nothing landed on the remote, so a turn
    /// that ends here must never read as synchronized (#323).
    PushRejected {
        reason: String,
    },
    /// The checkout is part-way through a merge, rebase, cherry-pick or
    /// revert, or its index still holds unresolved conflict entries.
    /// Committing that state would record whatever conflict markers are on
    /// disk and drop the operation's other parent, so every turn refuses it
    /// and leaves the checkout exactly as found for a human to finish or
    /// abort (#323). `files` lists the conflicted paths, when there are any.
    OperationInProgress {
        files: Vec<String>,
    },
    /// The remote's recovery branch holds commits the local head does not,
    /// usually because someone started resolving the conflict on it. A
    /// publish is a fast-forward or nothing, so it refuses rather than
    /// overwrite that work (ADR-30).
    RecoveryDiverged,
    /// The remote refused to create or update the recovery branch: a token
    /// that may not create branches, a hook, a protected pattern. Carries the
    /// remote's own one-line reason, sanitized like [`Self::PushRejected`].
    RecoveryRejected {
        reason: String,
    },
    /// The remote rejected the supplied (or absent) credentials. Distinct from
    /// `Remote` so a caller can wait for a credential change or manual retry
    /// rather than backing off and retrying blindly.
    Authentication,
    Remote,
}

impl std::fmt::Display for ManagedSyncError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Validation => formatter.write_str("managed checkout validation failed"),
            Self::DirtyWorkingCopy { files } => {
                write!(
                    formatter,
                    "managed checkout has unsupported local work: {}",
                    files.join(", ")
                )
            }
            Self::LocalCommits { ahead } => {
                write!(formatter, "pull-only checkout has {ahead} local commits")
            }
            Self::Conflict { files } => {
                write!(
                    formatter,
                    "managed checkout merge conflict: {}",
                    files.join(", ")
                )
            }
            Self::PushRace => {
                formatter.write_str("managed checkout push raced with a remote update")
            }
            Self::PushRejected { reason } => {
                write!(
                    formatter,
                    "managed checkout push was rejected by the remote: {reason}"
                )
            }
            Self::OperationInProgress { files } if files.is_empty() => formatter
                .write_str("managed checkout has an unfinished merge or other Git operation"),
            Self::OperationInProgress { files } => write!(
                formatter,
                "managed checkout has an unfinished merge with conflicts in: {}",
                files.join(", ")
            ),
            Self::RecoveryDiverged => formatter.write_str(
                "the recovery branch on the remote has commits this Vault does not; \
                 Hatchdoor will not overwrite them",
            ),
            Self::RecoveryRejected { reason } => {
                write!(
                    formatter,
                    "the remote refused the recovery branch: {reason}"
                )
            }
            Self::Authentication => formatter.write_str("managed checkout authentication failed"),
            Self::Remote => formatter.write_str("managed checkout remote operation failed"),
        }
    }
}

impl std::error::Error for ManagedSyncError {}

const MAX_PUSH_RACE_ATTEMPTS: usize = 2;

/// Synchronize one previously validated managed checkout.
///
/// The caller retains the checkout lease and serializes this function with
/// Vault writes. This boundary neither acquires a checkout nor schedules,
/// polls, retries later, persists status, or repairs a failed checkout.
///
/// `ledger` is the Vault's pending batch of write records (issue #249). Only
/// the Two-way graph ever commits, and it takes the batch at the moment it
/// builds a commit message; a Pull-only checkout leaves it alone.
pub fn synchronize_managed_checkout(
    config: &ManagedSyncConfig,
    ledger: &WriteLedger,
) -> Result<ManagedSyncOutcome, ManagedSyncError> {
    let repository = open_validated_repository(config)?;
    match config.mode {
        ManagedSyncMode::PullOnly => synchronize_pull_only(&repository, config),
        ManagedSyncMode::TwoWay => synchronize_two_way(&repository, config, ledger),
    }
}

/// Commit whatever has changed in one previously validated checkout's Vault
/// subtree, and stop there. No fetch, no merge, no push, no remote of any
/// kind. See [`VaultWorkKind::Commit`](crate::vault_work::VaultWorkKind).
///
/// Shares [`prepare_two_way_worktree`] with the Two-way graph rather than
/// carrying a second commit implementation, so the two agree on what counts
/// as the Vault's subtree, on refusing drift outside it, and on how the
/// commit message is built from `ledger`.
///
/// Only a mode that commits reaches this: `PullOnly` refuses writes and has
/// nothing of its own to commit, and a folder its operator dirtied by hand is
/// exactly what its turn is supposed to leave alone.
pub fn commit_managed_checkout(
    config: &ManagedSyncConfig,
    ledger: &WriteLedger,
) -> Result<ManagedSyncOutcome, ManagedSyncError> {
    if config.mode != ManagedSyncMode::TwoWay {
        return Err(ManagedSyncError::Validation);
    }
    let repository = open_commit_repository(config)?;
    let committed = prepare_two_way_worktree(&repository, config, ledger)?;
    Ok(if committed {
        ManagedSyncOutcome::TwoWaySynchronized {
            committed: true,
            integrated: false,
        }
    } else {
        ManagedSyncOutcome::UpToDate
    })
}

/// What one recovery-branch publish pushed (ADR-30).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryPublication {
    /// The branch on the remote, without `refs/heads/`.
    pub branch: String,
    /// The local commit the recovery branch now points at.
    pub published_commit: String,
    /// The configured branch's tip as last fetched: the remote side of the
    /// conflict. Absent when this checkout has never fetched it.
    pub conflicting_commit: Option<String>,
}

/// The recovery branch a Vault publishes to: one per Vault and configured
/// branch, named by the immutable Vault ID so Vaults and instances sharing a
/// repository never collide and a rename changes nothing (ADR-30).
pub fn recovery_branch_name(branch: &str, vault_id: VaultId) -> String {
    format!("hatchdoor-recovery/{branch}/{vault_id}")
}

/// Publish this checkout's local head to the Vault's recovery branch on the
/// remote, so a conflict can be resolved on the Git host (ADR-30).
///
/// Commits the Vault's pending drift first, exactly as a sync would, so the
/// branch carries every save made so far. The push is a fast-forward of the
/// recovery branch and nothing else: it never force-pushes, never names the
/// configured branch, and never deletes a remote branch. A recovery branch
/// someone has added to is refused as [`ManagedSyncError::RecoveryDiverged`].
///
/// Like [`synchronize_managed_checkout`], the caller holds the checkout
/// lease and serializes this with Vault writes.
pub fn publish_recovery_branch(
    config: &ManagedSyncConfig,
    vault_id: VaultId,
    ledger: &WriteLedger,
) -> Result<RecoveryPublication, ManagedSyncError> {
    if config.mode != ManagedSyncMode::TwoWay {
        return Err(ManagedSyncError::Validation);
    }
    let repository = open_validated_repository(config)?;
    prepare_two_way_worktree(&repository, config, ledger)?;

    let branch = recovery_branch_name(&config.branch, vault_id);
    if !git2::Reference::is_valid_name(&format!("refs/heads/{branch}")) {
        return Err(ManagedSyncError::Validation);
    }
    let published = repository
        .refname_to_id(&format!("refs/heads/{}", config.branch))
        .map_err(|_| ManagedSyncError::Validation)?;
    let remote_name = managed_remote_name(&repository, config)?;
    let conflicting = repository
        .refname_to_id(&format!("refs/remotes/{remote_name}/{}", config.branch))
        .ok();

    push_refspec(
        &repository,
        config,
        &format!("refs/heads/{}:refs/heads/{branch}", config.branch),
    )
    .map_err(|error| match error {
        ManagedSyncError::PushRace => ManagedSyncError::RecoveryDiverged,
        ManagedSyncError::PushRejected { reason } => ManagedSyncError::RecoveryRejected { reason },
        other => other,
    })?;

    Ok(RecoveryPublication {
        branch,
        published_commit: published.to_string(),
        conflicting_commit: conflicting.map(|oid| oid.to_string()),
    })
}

fn synchronize_pull_only(
    repository: &Repository,
    config: &ManagedSyncConfig,
) -> Result<ManagedSyncOutcome, ManagedSyncError> {
    reject_dirty_worktree(repository)?;
    fetch(repository, config)?;
    reject_dirty_worktree(repository)?;

    let relation = graph(repository, config)?;
    if relation.ahead > 0 {
        return Err(ManagedSyncError::LocalCommits {
            ahead: relation.ahead,
        });
    }
    if relation.behind == 0 {
        return Ok(ManagedSyncOutcome::UpToDate);
    }

    fast_forward(repository, config, relation.remote_oid)?;
    open_validated_repository(config)?;
    Ok(ManagedSyncOutcome::PullOnlyFastForwarded)
}

fn synchronize_two_way(
    repository: &Repository,
    config: &ManagedSyncConfig,
    ledger: &WriteLedger,
) -> Result<ManagedSyncOutcome, ManagedSyncError> {
    synchronize_two_way_with_push(repository, config, ledger, push)
}

fn synchronize_two_way_with_push<F>(
    repository: &Repository,
    config: &ManagedSyncConfig,
    ledger: &WriteLedger,
    mut push_operation: F,
) -> Result<ManagedSyncOutcome, ManagedSyncError>
where
    F: FnMut(&Repository, &ManagedSyncConfig) -> Result<(), ManagedSyncError>,
{
    let mut committed = prepare_two_way_worktree(repository, config, ledger)?;
    let mut integrated = false;

    for attempt in 0..MAX_PUSH_RACE_ATTEMPTS {
        fetch(repository, config)?;
        committed |= prepare_two_way_worktree(repository, config, ledger)?;
        let relation = graph(repository, config)?;

        if relation.behind > 0 {
            if relation.ahead == 0 {
                fast_forward(repository, config, relation.remote_oid)?;
            } else {
                merge_remote(repository, config, relation.remote_oid)?;
            }
            open_validated_repository(config)?;
            integrated = true;
        }

        if graph(repository, config)?.ahead == 0 {
            return Ok(if committed || integrated {
                ManagedSyncOutcome::TwoWaySynchronized {
                    committed,
                    integrated,
                }
            } else {
                ManagedSyncOutcome::UpToDate
            });
        }

        match push_operation(repository, config) {
            Ok(()) => {
                return Ok(ManagedSyncOutcome::TwoWaySynchronized {
                    committed,
                    integrated,
                });
            }
            Err(ManagedSyncError::PushRace) if attempt + 1 < MAX_PUSH_RACE_ATTEMPTS => continue,
            Err(error) => return Err(error),
        }
    }

    Err(ManagedSyncError::PushRace)
}

/// Open the checkout and prove it is the one this config describes: a
/// non-bare repository whose working directory *is* `repository_path`, with
/// `vault_path` a real directory inside it, and with a branch checked out.
///
/// The branch matters to a commit and not only to a fetch or a push, because
/// `commit_vault_drift` commits to `HEAD`: on a detached HEAD that leaves the
/// commit on no branch at all, and on the wrong branch it puts the Vault's
/// history somewhere the sync turn will never push from. So a configured
/// `branch` is required to be the one checked out, exactly as
/// [`open_validated_repository`] requires. An empty `branch` means the Vault
/// has none configured, and then any branch will do, which extends
/// `super::sync::validate_local_repo`'s Local-history policy of following
/// whatever the operator has checked out (#267).
///
/// What this does *not* check is the remote, which only an operation that
/// talks to one needs. That is [`open_validated_repository`]'s to add.
fn open_commit_repository(config: &ManagedSyncConfig) -> Result<Repository, ManagedSyncError> {
    let repository_path = config
        .repository_path
        .canonicalize()
        .map_err(|_| ManagedSyncError::Validation)?;
    let vault_path = config
        .vault_path
        .canonicalize()
        .map_err(|_| ManagedSyncError::Validation)?;
    let repository =
        Repository::open(&repository_path).map_err(|_| ManagedSyncError::Validation)?;
    let workdir = repository
        .workdir()
        .ok_or(ManagedSyncError::Validation)?
        .canonicalize()
        .map_err(|_| ManagedSyncError::Validation)?;
    if workdir != repository_path
        || !vault_path.starts_with(&repository_path)
        || !std::fs::metadata(&vault_path)
            .map_err(|_| ManagedSyncError::Validation)?
            .is_dir()
    {
        return Err(ManagedSyncError::Validation);
    }
    let head = repository
        .head()
        .map_err(|_| ManagedSyncError::Validation)?;
    if !head.is_branch() {
        return Err(ManagedSyncError::Validation);
    }
    if !config.branch.is_empty()
        && head.shorthand().map_err(|_| ManagedSyncError::Validation)? != config.branch
    {
        return Err(ManagedSyncError::Validation);
    }
    drop(head);
    reject_unfinished_operation(&repository)?;
    Ok(repository)
}

/// Refuse a checkout that is mid-merge (or mid-rebase, cherry-pick, revert)
/// or whose index still carries conflict entries. Such a checkout reaches a
/// turn only when something outside this module left it that way: the
/// process died between `merge` and its abort, or an operator is resolving a
/// merge in their own `ExistingGit` checkout. Either way the working tree may
/// hold conflict markers, and committing it as ordinary drift would publish
/// them and silently drop the merge's second parent. This boundary cannot
/// tell a Hatchdoor-interrupted merge from an operator's deliberate one, so
/// it repairs neither and reports both (#323).
fn reject_unfinished_operation(repository: &Repository) -> Result<(), ManagedSyncError> {
    let index = repository
        .index()
        .map_err(|_| ManagedSyncError::Validation)?;
    if repository.state() == git2::RepositoryState::Clean && !index.has_conflicts() {
        return Ok(());
    }
    Err(unfinished_operation_error(repository))
}

fn unfinished_operation_error(repository: &Repository) -> ManagedSyncError {
    ManagedSyncError::OperationInProgress {
        files: repository
            .index()
            .map(|mut index| conflict_paths(&mut index))
            .unwrap_or_default(),
    }
}

fn open_validated_repository(config: &ManagedSyncConfig) -> Result<Repository, ManagedSyncError> {
    let repository = open_commit_repository(config)?;
    managed_remote_name(&repository, config)?;
    Ok(repository)
}

fn managed_remote_name(
    repository: &Repository,
    config: &ManagedSyncConfig,
) -> Result<String, ManagedSyncError> {
    let remote_names = repository
        .remotes()
        .map_err(|_| ManagedSyncError::Validation)?;
    let mut matching = Vec::new();
    #[cfg(test)]
    let mut test_local = Vec::new();
    for name in remote_names.iter().flatten().flatten() {
        let remote = repository
            .find_remote(name)
            .map_err(|_| ManagedSyncError::Validation)?;
        let url = remote.url().map_err(|_| ManagedSyncError::Validation)?;
        if url != config.repository_url {
            #[cfg(test)]
            if url.starts_with('/') && !url.contains(['?', '#']) {
                test_local.push(name.to_string());
            }
            continue;
        }
        if !safe_managed_remote_url(url) {
            return Err(ManagedSyncError::Validation);
        }
        match remote.pushurl() {
            Ok(Some(url)) if url != config.repository_url || !safe_managed_remote_url(url) => {
                return Err(ManagedSyncError::Validation);
            }
            Ok(_) => {}
            Err(error) if error.code() == git2::ErrorCode::NotFound => {}
            Err(_) => return Err(ManagedSyncError::Validation),
        }
        matching.push(name.to_string());
    }
    // Registry integration tests cannot persist local filesystem URLs because
    // the production registry correctly accepts HTTPS only. Preserve the
    // existing test-only local-remote allowance when there is one unambiguous
    // fixture remote; production builds compile out this branch entirely.
    #[cfg(test)]
    if matching.is_empty() && test_local.len() == 1 {
        return test_local.pop().ok_or(ManagedSyncError::Validation);
    }
    match matching.as_slice() {
        [name] => Ok(name.clone()),
        _ => Err(ManagedSyncError::Validation),
    }
}

fn safe_managed_remote_url(url: &str) -> bool {
    crate::vault_registry::is_safe_https_repository_url(url)
        || (cfg!(test) && url.starts_with('/') && !url.contains(['?', '#']))
}

fn reject_dirty_worktree(repository: &Repository) -> Result<(), ManagedSyncError> {
    let files = changed_paths(repository)?;
    if files.is_empty() {
        Ok(())
    } else {
        Err(dirty_worktree_error(files))
    }
}

fn dirty_worktree_error(files: Vec<PathBuf>) -> ManagedSyncError {
    ManagedSyncError::DirtyWorkingCopy {
        files: files
            .into_iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
    }
}

fn prepare_two_way_worktree(
    repository: &Repository,
    config: &ManagedSyncConfig,
    ledger: &WriteLedger,
) -> Result<bool, ManagedSyncError> {
    let vault_relative = vault_relative_path(repository, config)?;
    let files = changed_paths(repository)?;
    let outside = files
        .iter()
        .filter(|path| !path.starts_with(&vault_relative))
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    if !outside.is_empty() {
        return Err(ManagedSyncError::DirtyWorkingCopy { files: outside });
    }
    if files.is_empty() {
        return Ok(false);
    }
    commit_vault_drift(repository, config, &vault_relative, ledger)
}

fn changed_paths(repository: &Repository) -> Result<Vec<PathBuf>, ManagedSyncError> {
    let mut options = git2::StatusOptions::new();
    options.include_untracked(true).recurse_untracked_dirs(true);
    repository
        .statuses(Some(&mut options))
        .map_err(|_| ManagedSyncError::Validation)?
        .iter()
        .map(|entry| {
            entry
                .path()
                .map(PathBuf::from)
                .map_err(|_| ManagedSyncError::Validation)
        })
        .collect()
}

fn vault_relative_path(
    repository: &Repository,
    config: &ManagedSyncConfig,
) -> Result<PathBuf, ManagedSyncError> {
    let workdir = repository.workdir().ok_or(ManagedSyncError::Validation)?;
    config
        .vault_path
        .canonicalize()
        .map_err(|_| ManagedSyncError::Validation)?
        .strip_prefix(
            workdir
                .canonicalize()
                .map_err(|_| ManagedSyncError::Validation)?,
        )
        .map(Path::to_path_buf)
        .map_err(|_| ManagedSyncError::Validation)
}

fn commit_vault_drift(
    repository: &Repository,
    config: &ManagedSyncConfig,
    vault_relative: &Path,
    ledger: &WriteLedger,
) -> Result<bool, ManagedSyncError> {
    // Read the operator's on-disk index status *before* building this
    // commit's own in-memory index below: `has_staged_vault_changes` reads
    // the real `.git/index` file, and nothing before the post-commit
    // refresh at the bottom of this function ever calls `.write()` on it, so
    // this check's result stays valid for the whole function regardless of
    // exactly when it runs — but checking first keeps the intent obvious.
    let preserve_vault_index = has_staged_vault_changes(repository, vault_relative)?;
    let parent = repository
        .head()
        .ok()
        .and_then(|head| head.peel_to_commit().ok());
    let mut index = repository
        .index()
        .map_err(|_| ManagedSyncError::Validation)?;
    if let Some(parent) = &parent {
        index
            .read_tree(&parent.tree().map_err(|_| ManagedSyncError::Validation)?)
            .map_err(|_| ManagedSyncError::Validation)?;
    } else {
        index.clear().map_err(|_| ManagedSyncError::Validation)?;
    }
    stage_vault_drift(repository, &mut index, vault_relative)?;
    let tree = repository
        .find_tree(
            index
                .write_tree_to(repository)
                .map_err(|_| ManagedSyncError::Validation)?,
        )
        .map_err(|_| ManagedSyncError::Validation)?;
    if parent.as_ref().is_some_and(|parent| {
        parent
            .tree()
            .is_ok_and(|parent_tree| parent_tree.id() == tree.id())
    }) {
        return Ok(false);
    }

    let signature = signature(config)?;
    let parents = parent.iter().collect::<Vec<_>>();
    // Reached only once this commit is certain, so a turn that found no drift
    // has already returned above with the batch untouched (issue #249).
    ledger.commit_batch(|message| {
        repository
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                message,
                &tree,
                &parents,
            )
            .map(|_| true)
            .map_err(|_| ManagedSyncError::Validation)
    })?;

    // `commit` advances HEAD but does not update the on-disk index. Refresh
    // precisely the Vault subtree when it had no existing staging. If an
    // operator did stage Vault content (a manual `git add` distinct from
    // HEAD), retain that index exactly: it may intentionally differ from
    // both the working tree and the just-created commit. Mirrors
    // `sync.rs`'s `commit_working_tree`, the analogous Local-history/legacy
    // function this one otherwise duplicates.
    if !preserve_vault_index {
        let mut worktree_index = repository
            .index()
            .map_err(|_| ManagedSyncError::Validation)?;
        stage_vault_drift(repository, &mut worktree_index, vault_relative)?;
        worktree_index
            .write()
            .map_err(|_| ManagedSyncError::Validation)?;
    }
    Ok(true)
}

/// True when the Vault subtree already has genuinely staged changes distinct
/// from HEAD — an operator's manual `git add` inside the Vault, not yet
/// committed. Mirrors `sync.rs`'s `has_staged_vault_changes` (used by
/// `commit_working_tree` for the exact same "retain the operator's staged
/// index" contract) with `ManagedSyncError` in place of `GitError`, since
/// `managed_sync.rs` and `sync.rs` do not share one error enum and this
/// ~15-line function is cheaper to mirror than to unify.
fn has_staged_vault_changes(
    repository: &Repository,
    vault_relative: &Path,
) -> Result<bool, ManagedSyncError> {
    let staged = git2::Status::INDEX_NEW
        | git2::Status::INDEX_MODIFIED
        | git2::Status::INDEX_DELETED
        | git2::Status::INDEX_RENAMED
        | git2::Status::INDEX_TYPECHANGE;
    repository
        .statuses(None)
        .map_err(|_| ManagedSyncError::Validation)?
        .iter()
        .try_fold(false, |found, entry| {
            if found {
                return Ok(true);
            }
            let path = entry.path().map_err(|_| ManagedSyncError::Validation)?;
            Ok(Path::new(path).starts_with(vault_relative) && entry.status().intersects(staged))
        })
}

fn stage_vault_drift(
    repository: &Repository,
    index: &mut git2::Index,
    vault_relative: &Path,
) -> Result<(), ManagedSyncError> {
    let mut options = git2::StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .renames_head_to_index(false)
        .renames_index_to_workdir(false);
    for entry in repository
        .statuses(Some(&mut options))
        .map_err(|_| ManagedSyncError::Validation)?
        .iter()
    {
        let path = entry.path().map_err(|_| ManagedSyncError::Validation)?;
        let path = Path::new(path);
        if !path.starts_with(vault_relative) {
            continue;
        }
        if repository
            .workdir()
            .ok_or(ManagedSyncError::Validation)?
            .join(path)
            .exists()
        {
            index
                .add_path(path)
                .map_err(|_| ManagedSyncError::Validation)?;
        } else {
            index
                .remove_path(path)
                .map_err(|_| ManagedSyncError::Validation)?;
        }
    }
    Ok(())
}

fn fetch(repository: &Repository, config: &ManagedSyncConfig) -> Result<(), ManagedSyncError> {
    let remote_name = managed_remote_name(repository, config)?;
    let mut remote = repository
        .find_remote(&remote_name)
        .map_err(|_| ManagedSyncError::Validation)?;
    super::bound_network_waits();
    let mut options = FetchOptions::new();
    if let Some(callbacks) = managed_remote_callbacks(config.credentials.as_ref()) {
        options.remote_callbacks(callbacks);
    }
    remote
        .fetch(&[&config.branch], Some(&mut options), None)
        .map_err(classify_remote_error)
}

/// Distinguish a credential rejection from any other remote failure.
fn classify_remote_error(error: git2::Error) -> ManagedSyncError {
    if error.code() == git2::ErrorCode::Auth {
        ManagedSyncError::Authentication
    } else {
        ManagedSyncError::Remote
    }
}

struct BranchRelation {
    ahead: usize,
    behind: usize,
    remote_oid: git2::Oid,
}

fn graph(
    repository: &Repository,
    config: &ManagedSyncConfig,
) -> Result<BranchRelation, ManagedSyncError> {
    let remote_name = managed_remote_name(repository, config)?;
    let local = repository
        .refname_to_id(&format!("refs/heads/{}", config.branch))
        .map_err(|_| ManagedSyncError::Validation)?;
    let remote = repository
        .refname_to_id(&format!("refs/remotes/{remote_name}/{}", config.branch))
        .map_err(|_| ManagedSyncError::Remote)?;
    let (ahead, behind) = repository
        .graph_ahead_behind(local, remote)
        .map_err(|_| ManagedSyncError::Validation)?;
    Ok(BranchRelation {
        ahead,
        behind,
        remote_oid: remote,
    })
}

fn fast_forward(
    repository: &Repository,
    config: &ManagedSyncConfig,
    remote_oid: git2::Oid,
) -> Result<(), ManagedSyncError> {
    let reference = format!("refs/heads/{}", config.branch);
    let remote = repository
        .find_commit(remote_oid)
        .map_err(|_| ManagedSyncError::Validation)?;
    repository
        .checkout_tree(
            remote.as_object(),
            Some(git2::build::CheckoutBuilder::new().safe()),
        )
        .map_err(|_| dirty_worktree_error(changed_paths(repository).unwrap_or_default()))?;
    repository
        .reference(
            &reference,
            remote_oid,
            true,
            "hatchdoor managed fast-forward",
        )
        .map_err(|_| ManagedSyncError::Validation)?;
    repository
        .set_head(&reference)
        .map_err(|_| ManagedSyncError::Validation)
}

fn merge_remote(
    repository: &Repository,
    config: &ManagedSyncConfig,
    remote_oid: git2::Oid,
) -> Result<(), ManagedSyncError> {
    let remote_name = managed_remote_name(repository, config)?;
    let local_oid = repository
        .refname_to_id(&format!("refs/heads/{}", config.branch))
        .map_err(|_| ManagedSyncError::Validation)?;
    let remote = repository
        .find_annotated_commit(remote_oid)
        .map_err(|_| ManagedSyncError::Validation)?;
    let mut options = MergeOptions::new();
    repository
        .merge(&[&remote], Some(&mut options), None)
        .map_err(|_| ManagedSyncError::Validation)?;
    let mut index = repository
        .index()
        .map_err(|_| ManagedSyncError::Validation)?;
    if index.has_conflicts() {
        let files = conflict_paths(&mut index);
        abort_merge(repository, config, local_oid)?;
        return Err(ManagedSyncError::Conflict { files });
    }
    let tree = repository
        .find_tree(
            index
                .write_tree()
                .map_err(|_| ManagedSyncError::Validation)?,
        )
        .map_err(|_| ManagedSyncError::Validation)?;
    let signature = signature(config)?;
    let local = repository
        .find_commit(local_oid)
        .map_err(|_| ManagedSyncError::Validation)?;
    let remote = repository
        .find_commit(remote_oid)
        .map_err(|_| ManagedSyncError::Validation)?;
    repository
        .commit(
            Some("HEAD"),
            &signature,
            &signature,
            &format!("Merge remote {remote_name}/{}", config.branch),
            &tree,
            &[&local, &remote],
        )
        .map_err(|_| ManagedSyncError::Validation)?;
    repository
        .cleanup_state()
        .map_err(|_| ManagedSyncError::Validation)?;
    repository
        .checkout_head(Some(git2::build::CheckoutBuilder::new().safe()))
        .map_err(|_| dirty_worktree_error(changed_paths(repository).unwrap_or_default()))
}

fn conflict_paths(index: &mut git2::Index) -> Vec<String> {
    let mut files = index
        .conflicts()
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|conflict| conflict.our.or(conflict.their))
        .filter_map(|entry| std::str::from_utf8(&entry.path).ok().map(str::to_owned))
        .collect::<Vec<_>>();
    files.sort();
    files.dedup();
    files
}

/// Undo a conflicted merge and put the checkout back on `local_oid` with no
/// merge state, no conflict entries in the index, and no conflict markers on
/// disk. Runs to completion on every conflicted merge, whatever else the
/// merge touched: an early return here strands the repository mid-merge with
/// markers in the Vault's Markdown (#323).
///
/// Only the paths the merge itself wrote are restored: the ones whose index
/// entry now differs from `local_oid` (the remote's cleanly merged changes,
/// inside the Vault subtree or outside it) and the conflicted ones. A
/// whole-tree hard reset would also revert a note an external editor saved
/// during the turn, which `git_merge`'s safe checkout deliberately left
/// alone, and that edit exists nowhere else. Such a note keeps its content
/// and is committed by the next turn as ordinary drift.
fn abort_merge(
    repository: &Repository,
    config: &ManagedSyncConfig,
    local_oid: git2::Oid,
) -> Result<(), ManagedSyncError> {
    let local = repository
        .find_commit(local_oid)
        .map_err(|_| ManagedSyncError::Validation)?;
    // With no merge-written path there is nothing on disk to restore, and an
    // empty path list would mean "every path", so the checkout is skipped. A
    // status read that fails counts as a failed restore.
    let restored = match merge_written_paths(repository) {
        Ok(merged_paths) if merged_paths.is_empty() => true,
        Ok(merged_paths) => {
            let mut checkout = git2::build::CheckoutBuilder::new();
            // Literal paths, not pathspecs: a note named `[draft].md` or
            // `*.md` must match itself and nothing else. Set here rather than
            // passed to `reset`, which replaces the strategy flags with a
            // bare FORCE.
            checkout.force().disable_pathspec_match(true);
            for path in &merged_paths {
                checkout.path(path);
            }
            // Checked out while the index still lists what the merge added,
            // so a file the remote introduced is removed as tracked content
            // rather than left behind as untracked.
            repository
                .checkout_tree(local.as_object(), Some(&mut checkout))
                .is_ok()
        }
        Err(_) => false,
    };
    // A mixed reset reads `local`'s tree into the whole index, which drops
    // every conflict entry, and clears MERGE_HEAD and the other merge state
    // files. When the targeted restore failed, a full hard reset is the
    // fallback: losing an in-flight external edit is recoverable from the
    // editor, conflict markers left in a clean-looking checkout are not,
    // because the next turn would commit them. If even that fails, the merge
    // state stays and `reject_unfinished_operation` stops every later turn.
    let reset_type = if restored {
        ResetType::Mixed
    } else {
        ResetType::Hard
    };
    repository
        .reset(local.as_object(), reset_type, None)
        .map_err(|_| unfinished_operation_error(repository))?;
    repository
        .cleanup_state()
        .map_err(|_| unfinished_operation_error(repository))?;
    open_validated_repository(config).map(|_| ())
}

/// Every path a just-failed merge wrote: its index entry differs from HEAD
/// (still the pre-merge local commit), or it is conflicted. A path whose only
/// difference is in the working tree was not written by the merge.
fn merge_written_paths(repository: &Repository) -> Result<Vec<PathBuf>, ManagedSyncError> {
    let merge_written = git2::Status::INDEX_NEW
        | git2::Status::INDEX_MODIFIED
        | git2::Status::INDEX_DELETED
        | git2::Status::INDEX_RENAMED
        | git2::Status::INDEX_TYPECHANGE
        | git2::Status::CONFLICTED;
    let mut options = git2::StatusOptions::new();
    options
        .include_untracked(false)
        .renames_head_to_index(false)
        .renames_index_to_workdir(false);
    repository
        .statuses(Some(&mut options))
        .map_err(|_| ManagedSyncError::Validation)?
        .iter()
        .filter(|entry| entry.status().intersects(merge_written))
        .map(|entry| {
            entry
                .path()
                .map(PathBuf::from)
                .map_err(|_| ManagedSyncError::Validation)
        })
        .collect()
}

fn push(repository: &Repository, config: &ManagedSyncConfig) -> Result<(), ManagedSyncError> {
    push_refspec(
        repository,
        config,
        &format!("refs/heads/{0}:refs/heads/{0}", config.branch),
    )
}

/// Push one non-forcing `refspec` to the managed remote. A push the remote
/// cannot take as a fast-forward is [`ManagedSyncError::PushRace`], and a
/// ref the remote refused is [`ManagedSyncError::PushRejected`]; callers
/// pushing something other than the configured branch rename those.
fn push_refspec(
    repository: &Repository,
    config: &ManagedSyncConfig,
    refspec: &str,
) -> Result<(), ManagedSyncError> {
    debug_assert!(!refspec.starts_with('+'), "a managed push never forces");
    let remote_name = managed_remote_name(repository, config)?;
    let mut remote = repository
        .find_remote(&remote_name)
        .map_err(|_| ManagedSyncError::Validation)?;
    super::bound_network_waits();
    // libgit2 reports a ref the remote refused to update only through this
    // callback: without one, `git_remote_push` discards the per-ref status
    // and returns success for a push that landed nothing (#323).
    let rejection = std::cell::RefCell::new(None::<String>);
    let mut callbacks = managed_remote_callbacks(config.credentials.as_ref()).unwrap_or_default();
    callbacks.push_update_reference(|_refname, status| {
        if let Some(reason) = status {
            rejection
                .borrow_mut()
                .get_or_insert_with(|| push_rejection_reason(reason));
        }
        Ok(())
    });
    let mut options = PushOptions::new();
    options.remote_callbacks(callbacks);
    let pushed = remote.push(&[refspec], Some(&mut options));
    drop(options);
    pushed.map_err(|error| {
        if error.code() == git2::ErrorCode::NotFastForward {
            ManagedSyncError::PushRace
        } else {
            classify_remote_error(error)
        }
    })?;
    match rejection.into_inner() {
        Some(reason) => Err(ManagedSyncError::PushRejected { reason }),
        None => Ok(()),
    }
}

/// The remote's rejection text, trimmed to one short line. It reaches every
/// client through the Vault's status, so control characters (a hook can
/// print anything) are dropped and the length is capped.
fn push_rejection_reason(reason: &str) -> String {
    const MAX_CHARS: usize = 200;
    let cleaned = reason
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.is_empty() {
        return "no reason given".to_string();
    }
    if cleaned.chars().count() > MAX_CHARS {
        let mut truncated = cleaned.chars().take(MAX_CHARS).collect::<String>();
        truncated.push('…');
        truncated
    } else {
        cleaned
    }
}

fn managed_remote_callbacks<'a>(
    credentials: Option<&ManagedHttpsCredentials>,
) -> Option<RemoteCallbacks<'a>> {
    let credentials = credentials?.clone();
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(move |_url, _username, _allowed| {
        Cred::userpass_plaintext(&credentials.username, &credentials.token)
    });
    Some(callbacks)
}

fn signature(config: &ManagedSyncConfig) -> Result<Signature<'_>, ManagedSyncError> {
    Signature::now(&config.author_name, &config.author_email)
        .map_err(|_| ManagedSyncError::Validation)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use git2::{Repository, Signature};
    use tempfile::TempDir;

    use super::super::message::WriteRecord;
    use super::*;

    fn commit(repository: &Repository, path: &str, contents: &str, message: &str) {
        let workdir = repository.workdir().expect("workdir");
        std::fs::write(workdir.join(path), contents).expect("write");
        let mut index = repository.index().expect("index");
        index.add_path(Path::new(path)).expect("stage");
        index.write().expect("write index");
        let tree = repository
            .find_tree(index.write_tree().expect("tree id"))
            .expect("tree");
        let signature = Signature::now("Test", "test@example.test").expect("signature");
        let parent = repository
            .head()
            .ok()
            .and_then(|head| head.peel_to_commit().ok());
        let parents = parent.iter().collect::<Vec<_>>();
        repository
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                message,
                &tree,
                &parents,
            )
            .expect("commit");
    }

    fn fixture(mode: ManagedSyncMode) -> (TempDir, ManagedSyncConfig) {
        let root = tempfile::tempdir().expect("tempdir");
        let source = root.path().join("source");
        let source_repository = Repository::init(&source).expect("source repository");
        std::fs::create_dir(source.join("vault")).expect("vault directory");
        commit(&source_repository, "vault/Home.md", "# Home\n", "initial");

        let remote = root.path().join("remote.git");
        Repository::init_bare(&remote).expect("bare remote");
        let mut origin = source_repository
            .remote("origin", remote.to_str().expect("remote path"))
            .expect("origin");
        origin
            .push(&["refs/heads/master:refs/heads/master"], None)
            .expect("initial push");

        let checkout = root.path().join("checkout");
        Repository::clone(remote.to_str().expect("remote path"), &checkout).expect("checkout");
        (
            root,
            ManagedSyncConfig {
                repository_path: checkout.clone(),
                vault_path: checkout.join("vault"),
                repository_url: remote.to_string_lossy().into_owned(),
                branch: "master".to_string(),
                mode,
                credentials: None,
                author_name: "Hatchdoor".to_string(),
                author_email: "hatchdoor@example.test".to_string(),
            },
        )
    }

    fn remote_commit(root: &Path, path: &str, contents: &str, message: &str) {
        let actor = root.join(format!("actor-{}", message.replace(' ', "-")));
        let repository = Repository::clone(
            root.join("remote.git").to_str().expect("remote path"),
            &actor,
        )
        .expect("actor checkout");
        commit(&repository, path, contents, message);
        let mut origin = repository.find_remote("origin").expect("origin");
        origin
            .push(&["refs/heads/master:refs/heads/master"], None)
            .expect("actor push");
    }

    fn file_at_head(repository: &Repository, path: &str) -> String {
        let head = repository
            .head()
            .expect("head")
            .peel_to_commit()
            .expect("commit");
        let entry = head
            .tree()
            .expect("tree")
            .get_path(Path::new(path))
            .expect("entry");
        let blob = repository.find_blob(entry.id()).expect("blob");
        String::from_utf8(blob.content().to_vec()).expect("UTF-8 file")
    }

    #[test]
    fn remote_errors_are_classified_as_authentication_or_generic_remote_failure() {
        let auth_error = git2::Error::new(
            git2::ErrorCode::Auth,
            git2::ErrorClass::Http,
            "authentication required",
        );
        assert_eq!(
            classify_remote_error(auth_error),
            ManagedSyncError::Authentication
        );

        let network_error = git2::Error::new(
            git2::ErrorCode::GenericError,
            git2::ErrorClass::Net,
            "could not resolve host",
        );
        assert_eq!(
            classify_remote_error(network_error),
            ManagedSyncError::Remote
        );
    }

    fn run_against_stalled_remote(
        mode: ManagedSyncMode,
    ) -> Result<ManagedSyncOutcome, ManagedSyncError> {
        let (root, mut config) = fixture(mode);
        let stalled = crate::git::stalled_https_remote();
        Repository::open(&config.repository_path)
            .expect("checkout")
            .remote_set_url("origin", &stalled)
            .expect("point origin at the stalled remote");
        config.repository_url = stalled;
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _root = root;
            let _ = sender.send(synchronize_managed_checkout(&config, &WriteLedger::new()));
        });
        receiver
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("sync against a stalled remote never returned")
    }

    #[test]
    fn a_stalled_remote_fails_a_pull_only_turn_as_a_retryable_remote_error() {
        assert_eq!(
            run_against_stalled_remote(ManagedSyncMode::PullOnly),
            Err(ManagedSyncError::Remote)
        );
    }

    #[test]
    fn a_stalled_remote_fails_a_two_way_turn_as_a_retryable_remote_error() {
        assert_eq!(
            run_against_stalled_remote(ManagedSyncMode::TwoWay),
            Err(ManagedSyncError::Remote)
        );
    }

    #[test]
    fn pull_only_preserves_dirty_local_work_without_fetching_or_overwriting() {
        let (_root, config) = fixture(ManagedSyncMode::PullOnly);
        std::fs::write(config.vault_path.join("Home.md"), "local edit\n").expect("local edit");

        let error = synchronize_managed_checkout(&config, &WriteLedger::new())
            .expect_err("dirty pull-only work");

        assert!(matches!(error, ManagedSyncError::DirtyWorkingCopy { .. }));
        assert_eq!(
            std::fs::read_to_string(config.vault_path.join("Home.md")).expect("local edit remains"),
            "local edit\n"
        );
    }

    #[test]
    fn pull_only_fast_forwards_a_clean_checkout_without_creating_a_local_commit() {
        let (root, config) = fixture(ManagedSyncMode::PullOnly);
        remote_commit(root.path(), "vault/Remote.md", "remote\n", "remote change");
        let before = Repository::open(&config.repository_path)
            .expect("checkout")
            .head()
            .expect("head")
            .target();

        let outcome =
            synchronize_managed_checkout(&config, &WriteLedger::new()).expect("pull-only sync");

        assert_eq!(outcome, ManagedSyncOutcome::PullOnlyFastForwarded);
        let checkout = Repository::open(&config.repository_path).expect("checkout");
        assert_ne!(checkout.head().expect("head").target(), before);
        assert_eq!(file_at_head(&checkout, "vault/Remote.md"), "remote\n");
    }

    #[test]
    fn pull_only_preserves_local_only_history_without_merging_or_pushing() {
        let (root, config) = fixture(ManagedSyncMode::PullOnly);
        let checkout = Repository::open(&config.repository_path).expect("checkout");
        commit(&checkout, "vault/Local.md", "local\n", "local-only commit");

        let error = synchronize_managed_checkout(&config, &WriteLedger::new())
            .expect_err("local history is unsupported");

        assert!(matches!(error, ManagedSyncError::LocalCommits { ahead: 1 }));
        let remote = Repository::open_bare(root.path().join("remote.git")).expect("remote");
        assert!(
            remote
                .head()
                .expect("remote head")
                .peel_to_commit()
                .expect("remote commit")
                .tree()
                .expect("remote tree")
                .get_path(Path::new("vault/Local.md"))
                .is_err(),
            "pull-only must not publish local history"
        );
    }

    #[test]
    fn two_way_commits_dirty_vault_work_before_pushing_it() {
        let (root, config) = fixture(ManagedSyncMode::TwoWay);
        std::fs::write(config.vault_path.join("Home.md"), "two-way local\n").expect("local edit");

        let outcome =
            synchronize_managed_checkout(&config, &WriteLedger::new()).expect("two-way sync");

        assert_eq!(
            outcome,
            ManagedSyncOutcome::TwoWaySynchronized {
                committed: true,
                integrated: false,
            }
        );
        let remote = Repository::open_bare(root.path().join("remote.git")).expect("remote");
        assert_eq!(file_at_head(&remote, "vault/Home.md"), "two-way local\n");
    }

    #[test]
    fn a_two_way_commit_is_named_by_the_writes_it_records() {
        let (_root, config) = fixture(ManagedSyncMode::TwoWay);
        // Longer than the committed content on purpose: git2's status check
        // trusts the index stat cache, and a same-size rewrite in the same
        // second reads as unchanged.
        std::fs::write(config.vault_path.join("Home.md"), "# Home\n\ntightened\n")
            .expect("local edit");
        let ledger = WriteLedger::new();
        ledger.record(WriteRecord {
            op: "update".to_string(),
            target: "Home".to_string(),
            affected_paths: vec![config.vault_path.join("Home.md")],
            summary: Some("tighten the intro".to_string()),
        });

        synchronize_managed_checkout(&config, &ledger).expect("two-way sync");

        let checkout = Repository::open(&config.repository_path).expect("checkout");
        let head = checkout
            .head()
            .expect("head")
            .peel_to_commit()
            .expect("commit");
        let message = head.message().expect("commit message");
        assert_eq!(
            message,
            "hatchdoor: update \"Home\" (1 file)\n\n- tighten the intro"
        );
        assert!(
            ledger.take().is_empty(),
            "the committed batch is not carried into the next commit"
        );
    }

    #[test]
    fn a_two_way_turn_with_no_drift_leaves_the_batch_for_the_turn_that_commits() {
        let (_root, config) = fixture(ManagedSyncMode::TwoWay);
        let ledger = WriteLedger::new();
        ledger.record(WriteRecord {
            op: "update".to_string(),
            target: "Home".to_string(),
            affected_paths: vec![config.vault_path.join("Home.md")],
            summary: Some("not committed yet".to_string()),
        });

        synchronize_managed_checkout(&config, &ledger).expect("two-way sync");

        let batch = ledger.take();
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].summary.as_deref(), Some("not committed yet"));
    }

    #[test]
    fn a_commit_recording_no_agent_writes_keeps_the_generic_message() {
        let (_root, config) = fixture(ManagedSyncMode::TwoWay);
        std::fs::write(
            config.vault_path.join("Home.md"),
            "# Home\n\nedited by hand\n",
        )
        .expect("local edit");

        synchronize_managed_checkout(&config, &WriteLedger::new()).expect("two-way sync");

        let checkout = Repository::open(&config.repository_path).expect("checkout");
        let head = checkout
            .head()
            .expect("head")
            .peel_to_commit()
            .expect("commit");
        assert_eq!(
            head.message().expect("commit message"),
            "hatchdoor: vault update"
        );
    }

    #[test]
    fn two_way_merges_diverged_histories_and_pushes_the_merge() {
        let (root, config) = fixture(ManagedSyncMode::TwoWay);
        remote_commit(root.path(), "vault/Remote.md", "remote\n", "remote change");
        std::fs::write(config.vault_path.join("Local.md"), "local\n").expect("local edit");

        let outcome =
            synchronize_managed_checkout(&config, &WriteLedger::new()).expect("diverged sync");

        assert_eq!(
            outcome,
            ManagedSyncOutcome::TwoWaySynchronized {
                committed: true,
                integrated: true,
            }
        );
        let remote = Repository::open_bare(root.path().join("remote.git")).expect("remote");
        let head = remote
            .head()
            .expect("head")
            .peel_to_commit()
            .expect("commit");
        assert_eq!(head.parent_count(), 2);
        assert_eq!(file_at_head(&remote, "vault/Local.md"), "local\n");
        assert_eq!(file_at_head(&remote, "vault/Remote.md"), "remote\n");
    }

    #[test]
    fn two_way_replays_fetch_integrate_push_once_after_a_push_race() {
        let (root, config) = fixture(ManagedSyncMode::TwoWay);
        std::fs::write(config.vault_path.join("Local.md"), "local\n").expect("local edit");
        let checkout = Repository::open(&config.repository_path).expect("checkout");
        let mut injected_race = false;

        let outcome = synchronize_two_way_with_push(
            &checkout,
            &config,
            &WriteLedger::new(),
            |repository, config| {
                if !injected_race {
                    injected_race = true;
                    remote_commit(root.path(), "vault/Race.md", "race\n", "push race");
                }
                push(repository, config)
            },
        )
        .expect("bounded replay resolves one push race");

        assert!(injected_race);
        assert_eq!(
            outcome,
            ManagedSyncOutcome::TwoWaySynchronized {
                committed: true,
                integrated: true,
            }
        );
        let remote = Repository::open_bare(root.path().join("remote.git")).expect("remote");
        assert_eq!(file_at_head(&remote, "vault/Local.md"), "local\n");
        assert_eq!(file_at_head(&remote, "vault/Race.md"), "race\n");
    }

    #[test]
    fn conflict_aborts_to_the_local_commit_and_leaves_the_remote_unchanged() {
        let (root, config) = fixture(ManagedSyncMode::TwoWay);
        remote_commit(
            root.path(),
            "vault/Home.md",
            "remote change\n",
            "remote conflict",
        );
        std::fs::write(config.vault_path.join("Home.md"), "local change\n").expect("local edit");

        let error =
            synchronize_managed_checkout(&config, &WriteLedger::new()).expect_err("merge conflict");

        assert!(matches!(error, ManagedSyncError::Conflict { .. }));
        let checkout = Repository::open(&config.repository_path).expect("checkout");
        assert_eq!(file_at_head(&checkout, "vault/Home.md"), "local change\n");
        assert_eq!(checkout.state(), git2::RepositoryState::Clean);
        let remote = Repository::open_bare(root.path().join("remote.git")).expect("remote");
        assert_eq!(file_at_head(&remote, "vault/Home.md"), "remote change\n");
    }

    #[test]
    fn two_way_rejects_outside_dirty_work_without_committing_or_overwriting_it() {
        let (_root, config) = fixture(ManagedSyncMode::TwoWay);
        let outside = config.repository_path.join("outside.txt");
        std::fs::write(&outside, "outside\n").expect("outside edit");
        std::fs::write(config.vault_path.join("Home.md"), "inside\n").expect("inside edit");

        let error = synchronize_managed_checkout(&config, &WriteLedger::new())
            .expect_err("outside dirty work");

        assert!(matches!(error, ManagedSyncError::DirtyWorkingCopy { .. }));
        assert_eq!(
            std::fs::read_to_string(outside).expect("outside remains"),
            "outside\n"
        );
        assert_eq!(
            std::fs::read_to_string(config.vault_path.join("Home.md")).expect("inside remains"),
            "inside\n"
        );
    }

    #[test]
    fn managed_sync_debug_output_never_reveals_https_credentials() {
        let (_root, mut config) = fixture(ManagedSyncMode::TwoWay);
        config.credentials = Some(ManagedHttpsCredentials {
            username: "private-user".to_string(),
            token: "private-token".to_string(),
        });

        let debug = format!("{config:?}");

        assert!(!debug.contains("private-user"));
        assert!(!debug.contains("private-token"));
    }

    #[test]
    fn credential_bearing_origin_is_rejected_without_revealing_it() {
        let (_root, config) = fixture(ManagedSyncMode::PullOnly);
        let checkout = Repository::open(&config.repository_path).expect("checkout");
        checkout
            .remote_set_url("origin", "https://private-token@example.test/vault.git")
            .expect("tamper origin");

        let error =
            synchronize_managed_checkout(&config, &WriteLedger::new()).expect_err("unsafe origin");

        assert_eq!(error, ManagedSyncError::Validation);
        assert!(!error.to_string().contains("private-token"));
    }

    #[test]
    fn credential_bearing_unrelated_remote_is_ignored_without_revealing_it() {
        let (_root, config) = fixture(ManagedSyncMode::PullOnly);
        let checkout = Repository::open(&config.repository_path).expect("checkout");
        checkout
            .remote("backup", "https://private-token@example.test/vault.git")
            .expect("secondary remote");

        let outcome =
            synchronize_managed_checkout(&config, &WriteLedger::new()).expect("configured remote");

        assert_eq!(outcome, ManagedSyncOutcome::UpToDate);
        assert!(!format!("{config:?}").contains("private-token"));
    }

    #[test]
    fn a_remote_transition_that_replaces_the_vault_directory_is_rejected() {
        let (root, config) = fixture(ManagedSyncMode::PullOnly);
        let actor = root.path().join("actor-replaces-vault");
        let repository = Repository::clone(
            root.path()
                .join("remote.git")
                .to_str()
                .expect("remote path"),
            &actor,
        )
        .expect("actor checkout");
        std::fs::remove_file(actor.join("vault/Home.md")).expect("remove note");
        std::fs::remove_dir(actor.join("vault")).expect("remove vault directory");
        std::fs::write(actor.join("vault"), "not a directory\n").expect("replace vault");
        let mut index = repository.index().expect("index");
        index
            .remove_path(Path::new("vault/Home.md"))
            .expect("remove staged note");
        index
            .add_path(Path::new("vault"))
            .expect("stage replacement");
        index.write().expect("write index");
        let tree = repository
            .find_tree(index.write_tree().expect("tree id"))
            .expect("tree");
        let parent = repository
            .head()
            .expect("head")
            .peel_to_commit()
            .expect("commit");
        let signature = Signature::now("Test", "test@example.test").expect("signature");
        repository
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                "replace vault",
                &tree,
                &[&parent],
            )
            .expect("commit replacement");
        repository
            .find_remote("origin")
            .expect("origin")
            .push(&["refs/heads/master:refs/heads/master"], None)
            .expect("push replacement");

        let error = synchronize_managed_checkout(&config, &WriteLedger::new())
            .expect_err("invalid Vault root");

        assert_eq!(error, ManagedSyncError::Validation);
    }

    #[test]
    fn missing_configured_remote_is_rejected() {
        let (_root, config) = fixture(ManagedSyncMode::PullOnly);
        let checkout = Repository::open(&config.repository_path).expect("checkout");
        checkout
            .remote_set_url("origin", "https://example.test/not valid.git")
            .expect("tamper origin");

        let error = synchronize_managed_checkout(&config, &WriteLedger::new())
            .expect_err("malformed origin");

        assert_eq!(error, ManagedSyncError::Validation);
    }

    #[test]
    fn configured_remote_is_used_while_an_unrelated_ssh_origin_is_ignored() {
        let (root, config) = fixture(ManagedSyncMode::TwoWay);
        let checkout = Repository::open(&config.repository_path).expect("checkout");
        checkout
            .remote_rename("origin", "gitsync")
            .expect("rename managed remote");
        checkout
            .remote("origin", "ssh://git@example.test/operator.git")
            .expect("operator remote");
        std::fs::write(config.vault_path.join("Home.md"), "selected remote\n").expect("local edit");

        let outcome =
            synchronize_managed_checkout(&config, &WriteLedger::new()).expect("two-way sync");

        assert_eq!(
            outcome,
            ManagedSyncOutcome::TwoWaySynchronized {
                committed: true,
                integrated: false,
            }
        );
        let remote = Repository::open_bare(root.path().join("remote.git")).expect("remote");
        assert_eq!(file_at_head(&remote, "vault/Home.md"), "selected remote\n");
    }

    #[test]
    fn duplicate_remotes_matching_the_configured_url_are_rejected() {
        let (_root, config) = fixture(ManagedSyncMode::PullOnly);
        let checkout = Repository::open(&config.repository_path).expect("checkout");
        checkout
            .remote("duplicate", &config.repository_url)
            .expect("duplicate remote");

        let error = synchronize_managed_checkout(&config, &WriteLedger::new())
            .expect_err("ambiguous remote");

        assert_eq!(error, ManagedSyncError::Validation);
    }

    #[test]
    fn selected_remote_with_a_different_push_url_is_rejected() {
        let (_root, config) = fixture(ManagedSyncMode::PullOnly);
        let checkout = Repository::open(&config.repository_path).expect("checkout");
        checkout
            .remote_set_pushurl("origin", Some("ssh://git@example.test/operator.git"))
            .expect("push URL");

        let error = synchronize_managed_checkout(&config, &WriteLedger::new())
            .expect_err("unsafe push URL");

        assert_eq!(error, ManagedSyncError::Validation);
    }

    /// Closes issue #96's reopening defect 3: `commit_vault_drift` used to
    /// unconditionally reload the on-disk index from the current working
    /// tree after committing, silently discarding any content an operator
    /// had staged but not yet committed. For HEAD=A ("# Home\n" from
    /// `fixture`), operator-staged=B ("operator staged\n", staged via a
    /// manual `git add` and never committed), worktree=C ("working tree
    /// content\n", edited again after staging): the commit correctly
    /// reflects C (already-correct existing behavior — `stage_vault_drift`
    /// stages current working-tree content, not the pre-existing staged
    /// index), but B must survive in the index afterward rather than being
    /// silently replaced by C.
    ///
    /// Before the fix this failed: the final assertion saw
    /// `"working tree content\n"` in the index instead of the preserved
    /// `"operator staged\n"`. Mirrors `sync.rs`'s
    /// `local_history_preserves_staged_vault_content`, the analogous
    /// already-correct test for the Local-history path.
    #[test]
    fn two_way_commit_preserves_an_operators_staged_vault_content_distinct_from_head_and_worktree()
    {
        let (_root, config) = fixture(ManagedSyncMode::TwoWay);
        let checkout = Repository::open(&config.repository_path).expect("checkout");

        // Operator stages B without committing.
        std::fs::write(config.vault_path.join("Home.md"), "operator staged\n")
            .expect("staged content");
        let mut operator_index = checkout.index().expect("operator index");
        operator_index
            .add_path(Path::new("vault/Home.md"))
            .expect("stage Vault content");
        operator_index.write().expect("persist operator staging");

        // Working tree is edited again after staging, to C, distinct from
        // both HEAD (A) and the staged content (B).
        std::fs::write(config.vault_path.join("Home.md"), "working tree content\n")
            .expect("working tree edit after staging");

        let outcome =
            synchronize_managed_checkout(&config, &WriteLedger::new()).expect("two-way sync");

        assert_eq!(
            outcome,
            ManagedSyncOutcome::TwoWaySynchronized {
                committed: true,
                integrated: false,
            }
        );

        // The commit reflects the working tree, matching the already-correct
        // existing behavior for the no-staged-content case.
        let checkout = Repository::open(&config.repository_path).expect("checkout");
        assert_eq!(
            file_at_head(&checkout, "vault/Home.md"),
            "working tree content\n"
        );

        // The operator's staged content must survive: it must not have been
        // silently replaced by the working-tree content during the
        // post-commit index refresh.
        let staged_entry = checkout
            .index()
            .expect("index after sync")
            .get_path(Path::new("vault/Home.md"), 0)
            .expect("preserved staged entry");
        assert_eq!(
            checkout
                .find_blob(staged_entry.id)
                .expect("preserved staged blob")
                .content(),
            b"operator staged\n",
            "two-way sync must not silently discard the operator's staged Vault content"
        );
    }

    #[test]
    fn an_outside_subtree_conflict_is_aborted_without_stranding_merge_state() {
        let (root, config) = fixture(ManagedSyncMode::TwoWay);
        let checkout = Repository::open(&config.repository_path).expect("checkout");
        commit(&checkout, "outside.md", "local\n", "local outside change");
        remote_commit(
            root.path(),
            "outside.md",
            "remote\n",
            "remote outside change",
        );

        let error = synchronize_managed_checkout(&config, &WriteLedger::new())
            .expect_err("outside conflict");

        assert!(matches!(error, ManagedSyncError::Conflict { .. }));
        let checkout = Repository::open(&config.repository_path).expect("checkout");
        assert_eq!(checkout.state(), git2::RepositoryState::Clean);
        assert_eq!(file_at_head(&checkout, "outside.md"), "local\n");
    }

    fn assert_no_conflict_markers(root: &Path) {
        for entry in walkdir_files(root) {
            let contents = std::fs::read(&entry).expect("read checkout file");
            let text = String::from_utf8_lossy(&contents);
            assert!(
                !text.contains("<<<<<<<") && !text.contains(">>>>>>>"),
                "conflict markers left on disk in {}",
                entry.display()
            );
        }
    }

    fn walkdir_files(root: &Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        let mut pending = vec![root.to_path_buf()];
        while let Some(directory) = pending.pop() {
            for entry in std::fs::read_dir(&directory).expect("read directory") {
                let path = entry.expect("directory entry").path();
                if path.file_name().is_some_and(|name| name == ".git") {
                    continue;
                }
                if path.is_dir() {
                    pending.push(path);
                } else {
                    files.push(path);
                }
            }
        }
        files
    }

    fn assert_checkout_is_consistent(repository: &Repository) {
        assert_eq!(repository.state(), git2::RepositoryState::Clean);
        assert!(
            !repository.path().join("MERGE_HEAD").exists(),
            "MERGE_HEAD must not survive an aborted merge"
        );
        assert!(
            !repository.index().expect("index").has_conflicts(),
            "the index must hold no conflict entries"
        );
        assert_no_conflict_markers(repository.workdir().expect("workdir"));
    }

    /// #323: a conflicted merge whose remote side also changed a path outside
    /// the Vault subtree used to return before resetting, stranding
    /// MERGE_HEAD, the conflicted index, and conflict markers in the note.
    #[test]
    fn a_vault_conflict_alongside_an_outside_change_is_fully_aborted() {
        let (root, config) = fixture(ManagedSyncMode::TwoWay);
        let actor = root.path().join("actor-mixed");
        let repository = Repository::clone(
            root.path()
                .join("remote.git")
                .to_str()
                .expect("remote path"),
            &actor,
        )
        .expect("actor checkout");
        commit(
            &repository,
            "outside.md",
            "remote outside\n",
            "remote outside",
        );
        commit(
            &repository,
            "vault/Home.md",
            "remote change\n",
            "remote note",
        );
        commit(
            &repository,
            "vault/New.md",
            "remote new\n",
            "remote new note",
        );
        repository
            .find_remote("origin")
            .expect("origin")
            .push(&["refs/heads/master:refs/heads/master"], None)
            .expect("actor push");
        std::fs::write(config.vault_path.join("Home.md"), "local change\n").expect("local edit");

        let error =
            synchronize_managed_checkout(&config, &WriteLedger::new()).expect_err("merge conflict");

        assert_eq!(
            error,
            ManagedSyncError::Conflict {
                files: vec!["vault/Home.md".to_string()]
            }
        );
        let checkout = Repository::open(&config.repository_path).expect("checkout");
        assert_checkout_is_consistent(&checkout);
        assert_eq!(
            std::fs::read_to_string(config.vault_path.join("Home.md")).expect("note"),
            "local change\n"
        );
        assert!(
            !config.repository_path.join("outside.md").exists(),
            "the remote's outside file must not be left behind"
        );
        assert!(
            !config.vault_path.join("New.md").exists(),
            "the remote's new note must not be left behind"
        );
        assert!(
            changed_paths(&checkout).expect("status").is_empty(),
            "the checkout is back on its local commit with nothing pending"
        );
        // And the next turn is not wedged: it reports the same conflict.
        assert!(matches!(
            synchronize_managed_checkout(&config, &WriteLedger::new()),
            Err(ManagedSyncError::Conflict { .. })
        ));
    }

    /// #323: the abort used to hard-reset the whole working tree, reverting a
    /// note an external editor saved while the turn was running.
    #[test]
    fn aborting_a_conflicted_merge_keeps_an_external_edit_the_merge_did_not_touch() {
        let (root, config) = fixture(ManagedSyncMode::TwoWay);
        let checkout = Repository::open(&config.repository_path).expect("checkout");
        commit(&checkout, "vault/Other.md", "# Other\n", "add other note");
        commit(&checkout, "vault/Home.md", "local change\n", "local note");
        remote_commit(
            root.path(),
            "vault/Home.md",
            "remote change\n",
            "remote conflict",
        );
        fetch(&checkout, &config).expect("fetch");
        let relation = graph(&checkout, &config).expect("graph");
        // Saved by an editor between the turn's own commit and the merge.
        std::fs::write(
            config.vault_path.join("Other.md"),
            "# Other\n\nsaved in the editor mid-turn\n",
        )
        .expect("external edit");

        let error =
            merge_remote(&checkout, &config, relation.remote_oid).expect_err("merge conflict");

        assert!(matches!(error, ManagedSyncError::Conflict { .. }));
        let checkout = Repository::open(&config.repository_path).expect("checkout");
        assert_checkout_is_consistent(&checkout);
        assert_eq!(
            std::fs::read_to_string(config.vault_path.join("Other.md")).expect("edited note"),
            "# Other\n\nsaved in the editor mid-turn\n"
        );
        assert_eq!(
            std::fs::read_to_string(config.vault_path.join("Home.md")).expect("note"),
            "local change\n"
        );
    }

    /// Leave `config`'s checkout mid-merge with a conflict in `vault/Home.md`,
    /// the way a process killed between `merge` and its abort would.
    fn strand_a_conflicted_merge(root: &Path, config: &ManagedSyncConfig) -> git2::Oid {
        let checkout = Repository::open(&config.repository_path).expect("checkout");
        commit(&checkout, "vault/Home.md", "local change\n", "local note");
        remote_commit(root, "vault/Home.md", "remote change\n", "remote conflict");
        fetch(&checkout, config).expect("fetch");
        let relation = graph(&checkout, config).expect("graph");
        let remote = checkout
            .find_annotated_commit(relation.remote_oid)
            .expect("remote commit");
        checkout.merge(&[&remote], None, None).expect("merge");
        assert_eq!(checkout.state(), git2::RepositoryState::Merge);
        assert!(
            std::fs::read_to_string(config.vault_path.join("Home.md"))
                .expect("note")
                .contains("<<<<<<<"),
            "fixture must leave markers on disk"
        );
        checkout
            .head()
            .expect("head")
            .target()
            .expect("head commit")
    }

    /// #323: a checkout left mid-merge used to pass validation, so the next
    /// commit turn committed the conflict markers as ordinary drift (dropping
    /// the merge's second parent) and the sync after it pushed them.
    #[test]
    fn a_checkout_left_mid_merge_is_refused_rather_than_committed_or_pushed() {
        let (root, config) = fixture(ManagedSyncMode::TwoWay);
        let before = strand_a_conflicted_merge(root.path(), &config);
        let remote_before = Repository::open_bare(root.path().join("remote.git"))
            .expect("remote")
            .head()
            .expect("remote head")
            .target();

        let expected = ManagedSyncError::OperationInProgress {
            files: vec!["vault/Home.md".to_string()],
        };
        assert_eq!(
            commit_managed_checkout(&config, &WriteLedger::new()),
            Err(expected.clone())
        );
        assert_eq!(
            synchronize_managed_checkout(&config, &WriteLedger::new()),
            Err(expected)
        );

        let checkout = Repository::open(&config.repository_path).expect("checkout");
        assert_eq!(checkout.head().expect("head").target(), Some(before));
        assert_eq!(
            checkout.state(),
            git2::RepositoryState::Merge,
            "the operator's evidence is left exactly as found"
        );
        let remote = Repository::open_bare(root.path().join("remote.git")).expect("remote");
        assert_eq!(remote.head().expect("remote head").target(), remote_before);
        assert!(!file_at_head(&remote, "vault/Home.md").contains("<<<<<<<"));
    }

    #[test]
    fn a_pull_only_checkout_left_mid_merge_is_refused() {
        let (root, mut config) = fixture(ManagedSyncMode::TwoWay);
        strand_a_conflicted_merge(root.path(), &config);
        config.mode = ManagedSyncMode::PullOnly;

        assert!(matches!(
            synchronize_managed_checkout(&config, &WriteLedger::new()),
            Err(ManagedSyncError::OperationInProgress { .. })
        ));
    }

    /// #323: libgit2 discards a server-side per-ref rejection unless a
    /// `push_update_reference` callback is installed, so a protected branch
    /// or a refusing hook read as a successful sync forever. The local
    /// transport reports a ref it cannot lock the same way a remote
    /// receive-pack reports a hook's refusal.
    #[test]
    fn a_push_the_remote_refuses_to_apply_fails_the_turn() {
        let (root, config) = fixture(ManagedSyncMode::TwoWay);
        std::fs::write(config.vault_path.join("Home.md"), "never lands\n").expect("local edit");
        let remote_git = root.path().join("remote.git");
        let remote_before = Repository::open_bare(&remote_git)
            .expect("remote")
            .head()
            .expect("remote head")
            .target();
        std::fs::write(remote_git.join("refs/heads/master.lock"), "held\n")
            .expect("hold the remote branch lock");

        let error = synchronize_managed_checkout(&config, &WriteLedger::new())
            .expect_err("a refused push is not a sync");

        let ManagedSyncError::PushRejected { reason } = &error else {
            panic!("expected a push rejection, got {error:?}");
        };
        assert!(!reason.is_empty());
        let remote = Repository::open_bare(&remote_git).expect("remote");
        assert_eq!(remote.head().expect("remote head").target(), remote_before);
    }

    #[test]
    fn a_push_rejection_reason_is_one_short_printable_line() {
        assert_eq!(
            push_rejection_reason("pre-receive hook\ndeclined\u{1b}[31m"),
            "pre-receive hook declined [31m"
        );
        assert_eq!(push_rejection_reason(" \n "), "no reason given");
        let long = push_rejection_reason(&"x".repeat(500));
        assert_eq!(long.chars().count(), 201);
        assert!(long.ends_with('…'));
    }

    fn recovery_vault_id() -> VaultId {
        "00000000-0000-4000-8000-0000000000aa"
            .parse()
            .expect("test Vault ID")
    }

    /// Put `config`'s checkout into the state a conflicting sync leaves: the
    /// local and remote sides both changed `vault/Home.md`, the merge was
    /// aborted, and the local commit is the checkout's head.
    fn conflicted(root: &Path, config: &ManagedSyncConfig) {
        conflicted_as(root, config, "remote side");
    }

    fn conflicted_as(root: &Path, config: &ManagedSyncConfig, label: &str) {
        remote_commit(root, "vault/Home.md", "remote change\n", label);
        std::fs::write(config.vault_path.join("Home.md"), "local change\n").expect("local edit");
        let error =
            synchronize_managed_checkout(config, &WriteLedger::new()).expect_err("merge conflict");
        assert!(matches!(error, ManagedSyncError::Conflict { .. }));
    }

    fn remote_ref(root: &Path, reference: &str) -> Option<git2::Oid> {
        Repository::open_bare(root.join("remote.git"))
            .expect("remote")
            .refname_to_id(reference)
            .ok()
    }

    fn local_head(config: &ManagedSyncConfig) -> git2::Oid {
        Repository::open(&config.repository_path)
            .expect("checkout")
            .refname_to_id("refs/heads/master")
            .expect("local branch")
    }

    const RECOVERY_REF: &str =
        "refs/heads/hatchdoor-recovery/master/00000000-0000-4000-8000-0000000000aa";

    #[test]
    fn a_recovery_branch_is_named_by_configured_branch_and_vault_id() {
        assert_eq!(
            recovery_branch_name("main", recovery_vault_id()),
            "hatchdoor-recovery/main/00000000-0000-4000-8000-0000000000aa"
        );
    }

    #[test]
    fn publishing_puts_the_local_head_on_the_recovery_branch_and_leaves_the_configured_branch_alone()
     {
        let (root, config) = fixture(ManagedSyncMode::TwoWay);
        conflicted(root.path(), &config);
        let remote_master = remote_ref(root.path(), "refs/heads/master").expect("remote master");

        let published = publish_recovery_branch(&config, recovery_vault_id(), &WriteLedger::new())
            .expect("publish");

        assert_eq!(
            published.branch,
            "hatchdoor-recovery/master/00000000-0000-4000-8000-0000000000aa"
        );
        assert_eq!(published.published_commit, local_head(&config).to_string());
        assert_eq!(
            published.conflicting_commit,
            Some(remote_master.to_string()),
            "the remote side of the conflict is the configured branch's fetched tip"
        );
        assert_eq!(
            remote_ref(root.path(), RECOVERY_REF),
            Some(local_head(&config))
        );
        assert_eq!(
            remote_ref(root.path(), "refs/heads/master"),
            Some(remote_master),
            "the configured branch on the remote is untouched"
        );
        let checkout = Repository::open(&config.repository_path).expect("checkout");
        assert_eq!(file_at_head(&checkout, "vault/Home.md"), "local change\n");
        assert_eq!(checkout.state(), git2::RepositoryState::Clean);
    }

    #[test]
    fn publishing_again_commits_pending_saves_and_fast_forwards_the_same_branch() {
        let (root, config) = fixture(ManagedSyncMode::TwoWay);
        conflicted(root.path(), &config);
        let first = publish_recovery_branch(&config, recovery_vault_id(), &WriteLedger::new())
            .expect("first publish");
        std::fs::write(config.vault_path.join("Later.md"), "later save\n").expect("later save");

        let second = publish_recovery_branch(&config, recovery_vault_id(), &WriteLedger::new())
            .expect("second publish");

        assert_eq!(first.branch, second.branch);
        assert_ne!(first.published_commit, second.published_commit);
        let head = local_head(&config);
        assert_eq!(second.published_commit, head.to_string());
        assert_eq!(remote_ref(root.path(), RECOVERY_REF), Some(head));
        let checkout = Repository::open(&config.repository_path).expect("checkout");
        assert_eq!(file_at_head(&checkout, "vault/Later.md"), "later save\n");
        assert!(
            checkout
                .graph_descendant_of(head, first.published_commit.parse().expect("oid"))
                .expect("graph"),
            "the second publish extends the first"
        );
    }

    #[test]
    fn a_recovery_branch_someone_added_to_is_refused_and_left_as_it_is() {
        let (root, config) = fixture(ManagedSyncMode::TwoWay);
        conflicted(root.path(), &config);
        publish_recovery_branch(&config, recovery_vault_id(), &WriteLedger::new())
            .expect("first publish");

        // Someone starts resolving on the recovery branch itself.
        let actor_path = root.path().join("actor-on-recovery");
        let actor = Repository::clone(
            root.path()
                .join("remote.git")
                .to_str()
                .expect("remote path"),
            &actor_path,
        )
        .expect("actor checkout");
        let recovery = actor
            .refname_to_id(
                "refs/remotes/origin/hatchdoor-recovery/master/00000000-0000-4000-8000-0000000000aa",
            )
            .expect("recovery branch fetched");
        actor
            .branch("work", &actor.find_commit(recovery).expect("commit"), false)
            .expect("branch");
        actor.set_head("refs/heads/work").expect("switch");
        actor
            .checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .expect("checkout");
        commit(&actor, "vault/Home.md", "half resolved\n", "resolving");
        actor
            .find_remote("origin")
            .expect("origin")
            .push(
                &[
                    "refs/heads/work:refs/heads/hatchdoor-recovery/master/00000000-0000-4000-8000-0000000000aa",
                ],
                None,
            )
            .expect("actor push");
        let theirs = remote_ref(root.path(), RECOVERY_REF).expect("recovery tip");

        std::fs::write(config.vault_path.join("Later.md"), "later save\n").expect("later save");
        let error = publish_recovery_branch(&config, recovery_vault_id(), &WriteLedger::new())
            .expect_err("diverged");

        assert_eq!(error, ManagedSyncError::RecoveryDiverged);
        assert_eq!(remote_ref(root.path(), RECOVERY_REF), Some(theirs));
    }

    #[test]
    fn once_the_configured_branch_holds_the_resolution_the_next_sync_succeeds_and_the_branch_stays()
    {
        let (root, config) = fixture(ManagedSyncMode::TwoWay);
        conflicted(root.path(), &config);
        publish_recovery_branch(&config, recovery_vault_id(), &WriteLedger::new())
            .expect("publish");

        // Resolve on the Git host: merge the recovery branch into master.
        let actor_path = root.path().join("actor-resolving");
        let actor = Repository::clone(
            root.path()
                .join("remote.git")
                .to_str()
                .expect("remote path"),
            &actor_path,
        )
        .expect("actor checkout");
        let ours = actor.refname_to_id("refs/heads/master").expect("master");
        let theirs = actor
            .refname_to_id(
                "refs/remotes/origin/hatchdoor-recovery/master/00000000-0000-4000-8000-0000000000aa",
            )
            .expect("recovery branch");
        std::fs::write(actor_path.join("vault/Home.md"), "resolved\n").expect("resolve");
        let mut index = actor.index().expect("index");
        index.add_path(Path::new("vault/Home.md")).expect("stage");
        index.write().expect("write index");
        let tree = actor
            .find_tree(index.write_tree().expect("tree"))
            .expect("tree");
        let signature = Signature::now("Resolver", "resolver@example.test").expect("signature");
        actor
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                "Merge recovery branch",
                &tree,
                &[
                    &actor.find_commit(ours).expect("ours"),
                    &actor.find_commit(theirs).expect("theirs"),
                ],
            )
            .expect("merge commit");
        actor
            .find_remote("origin")
            .expect("origin")
            .push(&["refs/heads/master:refs/heads/master"], None)
            .expect("push resolution");
        // A save made after publishing, on a note the resolution did not touch.
        std::fs::write(config.vault_path.join("Later.md"), "later save\n").expect("later save");

        synchronize_managed_checkout(&config, &WriteLedger::new()).expect("sync resumes");

        let checkout = Repository::open(&config.repository_path).expect("checkout");
        assert_eq!(file_at_head(&checkout, "vault/Home.md"), "resolved\n");
        assert_eq!(file_at_head(&checkout, "vault/Later.md"), "later save\n");
        assert_eq!(
            remote_ref(root.path(), "refs/heads/master"),
            Some(local_head(&config))
        );
        assert_eq!(
            remote_ref(root.path(), RECOVERY_REF),
            Some(theirs),
            "Hatchdoor never deletes or moves the recovery branch on its own"
        );

        // The next conflict publishes to the same branch as a fast-forward.
        conflicted_as(root.path(), &config, "second remote side");
        publish_recovery_branch(&config, recovery_vault_id(), &WriteLedger::new())
            .expect("a later conflict reuses the branch");
        assert_eq!(
            remote_ref(root.path(), RECOVERY_REF),
            Some(local_head(&config))
        );
    }

    #[test]
    fn a_recovery_branch_the_remote_refuses_reports_its_reason_and_lands_nothing() {
        let (root, config) = fixture(ManagedSyncMode::TwoWay);
        conflicted(root.path(), &config);
        let refs = root
            .path()
            .join("remote.git/refs/heads/hatchdoor-recovery/master");
        std::fs::create_dir_all(&refs).expect("recovery ref directory");
        std::fs::write(
            refs.join("00000000-0000-4000-8000-0000000000aa.lock"),
            "held\n",
        )
        .expect("hold the recovery branch lock");
        let remote_master = remote_ref(root.path(), "refs/heads/master");

        let error = publish_recovery_branch(&config, recovery_vault_id(), &WriteLedger::new())
            .expect_err("a refused branch is not published");

        let ManagedSyncError::RecoveryRejected { reason } = &error else {
            panic!("expected a recovery rejection, got {error:?}");
        };
        assert!(!reason.is_empty());
        assert_eq!(remote_ref(root.path(), RECOVERY_REF), None);
        assert_eq!(remote_ref(root.path(), "refs/heads/master"), remote_master);
    }

    #[test]
    fn a_pull_only_checkout_never_publishes() {
        let (root, config) = fixture(ManagedSyncMode::PullOnly);
        assert_eq!(
            publish_recovery_branch(&config, recovery_vault_id(), &WriteLedger::new()),
            Err(ManagedSyncError::Validation)
        );
        assert_eq!(remote_ref(root.path(), RECOVERY_REF), None);
    }
}
