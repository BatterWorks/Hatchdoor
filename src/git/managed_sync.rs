//! Synchronization graphs for a validated, Vault-ID-owned managed checkout.

use std::path::{Path, PathBuf};

use git2::{
    Cred, FetchOptions, MergeOptions, PushOptions, RemoteCallbacks, Repository, ResetType,
    Signature,
};

use super::managed_checkout::ManagedHttpsCredentials;

const MANAGED_REMOTE: &str = "origin";

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
    DirtyWorkingCopy { files: Vec<String> },
    LocalCommits { ahead: usize },
    Conflict { files: Vec<String> },
    PushRace,
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
pub fn synchronize_managed_checkout(
    config: &ManagedSyncConfig,
) -> Result<ManagedSyncOutcome, ManagedSyncError> {
    let repository = open_validated_repository(config)?;
    match config.mode {
        ManagedSyncMode::PullOnly => synchronize_pull_only(&repository, config),
        ManagedSyncMode::TwoWay => synchronize_two_way(&repository, config),
    }
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
) -> Result<ManagedSyncOutcome, ManagedSyncError> {
    synchronize_two_way_with_push(repository, config, push)
}

fn synchronize_two_way_with_push<F>(
    repository: &Repository,
    config: &ManagedSyncConfig,
    mut push_operation: F,
) -> Result<ManagedSyncOutcome, ManagedSyncError>
where
    F: FnMut(&Repository, &ManagedSyncConfig) -> Result<(), ManagedSyncError>,
{
    let mut committed = prepare_two_way_worktree(repository, config)?;
    let mut integrated = false;

    for attempt in 0..MAX_PUSH_RACE_ATTEMPTS {
        fetch(repository, config)?;
        committed |= prepare_two_way_worktree(repository, config)?;
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

fn open_validated_repository(config: &ManagedSyncConfig) -> Result<Repository, ManagedSyncError> {
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
    {
        let head = repository
            .head()
            .map_err(|_| ManagedSyncError::Validation)?;
        if !head.is_branch()
            || head.shorthand().map_err(|_| ManagedSyncError::Validation)? != config.branch
        {
            return Err(ManagedSyncError::Validation);
        }
    }
    validate_remote_urls(&repository)?;
    Ok(repository)
}

fn validate_remote_urls(repository: &Repository) -> Result<(), ManagedSyncError> {
    let remote_names = repository
        .remotes()
        .map_err(|_| ManagedSyncError::Validation)?;
    let remote_names = remote_names.iter().flatten().flatten().collect::<Vec<_>>();
    if !remote_names.contains(&MANAGED_REMOTE) {
        return Err(ManagedSyncError::Validation);
    }
    for name in remote_names {
        let remote = repository
            .find_remote(name)
            .map_err(|_| ManagedSyncError::Validation)?;
        let url = remote.url().map_err(|_| ManagedSyncError::Validation)?;
        if !safe_managed_remote_url(url) {
            return Err(ManagedSyncError::Validation);
        }
        match remote.pushurl() {
            Ok(Some(url)) if !safe_managed_remote_url(url) => {
                return Err(ManagedSyncError::Validation);
            }
            Ok(_) => {}
            Err(error) if error.code() == git2::ErrorCode::NotFound => {}
            Err(_) => return Err(ManagedSyncError::Validation),
        }
    }
    Ok(())
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
    commit_vault_drift(repository, config, &vault_relative)
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
) -> Result<bool, ManagedSyncError> {
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
    repository
        .commit(
            Some("HEAD"),
            &signature,
            &signature,
            "hatchdoor: record local Vault changes",
            &tree,
            &parents,
        )
        .map_err(|_| ManagedSyncError::Validation)?;

    let mut worktree_index = repository
        .index()
        .map_err(|_| ManagedSyncError::Validation)?;
    stage_vault_drift(repository, &mut worktree_index, vault_relative)?;
    worktree_index
        .write()
        .map_err(|_| ManagedSyncError::Validation)?;
    Ok(true)
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
    let mut remote = repository
        .find_remote(MANAGED_REMOTE)
        .map_err(|_| ManagedSyncError::Validation)?;
    let mut options = FetchOptions::new();
    if let Some(callbacks) = managed_remote_callbacks(config.credentials.as_ref()) {
        options.remote_callbacks(callbacks);
    }
    remote
        .fetch(&[&config.branch], Some(&mut options), None)
        .map_err(|_| ManagedSyncError::Remote)
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
    let local = repository
        .refname_to_id(&format!("refs/heads/{}", config.branch))
        .map_err(|_| ManagedSyncError::Validation)?;
    let remote = repository
        .refname_to_id(&format!("refs/remotes/{MANAGED_REMOTE}/{}", config.branch))
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
            &format!("Merge remote {MANAGED_REMOTE}/{}", config.branch),
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

fn abort_merge(
    repository: &Repository,
    config: &ManagedSyncConfig,
    local_oid: git2::Oid,
) -> Result<(), ManagedSyncError> {
    let vault_relative = vault_relative_path(repository, config)?;
    let outside = non_conflict_changed_paths(repository)?
        .into_iter()
        .filter(|path| !path.starts_with(&vault_relative))
        .collect::<Vec<_>>();
    if !outside.is_empty() {
        return Err(dirty_worktree_error(outside));
    }
    let local = repository
        .find_commit(local_oid)
        .map_err(|_| ManagedSyncError::Validation)?;
    repository
        .reset(local.as_object(), ResetType::Hard, None)
        .map_err(|_| ManagedSyncError::Validation)?;
    repository
        .cleanup_state()
        .map_err(|_| ManagedSyncError::Validation)?;
    open_validated_repository(config).map(|_| ())
}

fn non_conflict_changed_paths(repository: &Repository) -> Result<Vec<PathBuf>, ManagedSyncError> {
    let mut options = git2::StatusOptions::new();
    options.include_untracked(true).recurse_untracked_dirs(true);
    repository
        .statuses(Some(&mut options))
        .map_err(|_| ManagedSyncError::Validation)?
        .iter()
        .filter(|entry| !entry.status().contains(git2::Status::CONFLICTED))
        .map(|entry| {
            entry
                .path()
                .map(PathBuf::from)
                .map_err(|_| ManagedSyncError::Validation)
        })
        .collect()
}

fn push(repository: &Repository, config: &ManagedSyncConfig) -> Result<(), ManagedSyncError> {
    let mut remote = repository
        .find_remote(MANAGED_REMOTE)
        .map_err(|_| ManagedSyncError::Validation)?;
    let mut options = PushOptions::new();
    if let Some(callbacks) = managed_remote_callbacks(config.credentials.as_ref()) {
        options.remote_callbacks(callbacks);
    }
    remote
        .push(
            &[&format!("refs/heads/{0}:refs/heads/{0}", config.branch)],
            Some(&mut options),
        )
        .map_err(|error| {
            if error.code() == git2::ErrorCode::NotFastForward {
                ManagedSyncError::PushRace
            } else {
                ManagedSyncError::Remote
            }
        })
}

fn managed_remote_callbacks(
    credentials: Option<&ManagedHttpsCredentials>,
) -> Option<RemoteCallbacks<'static>> {
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
    fn pull_only_preserves_dirty_local_work_without_fetching_or_overwriting() {
        let (_root, config) = fixture(ManagedSyncMode::PullOnly);
        std::fs::write(config.vault_path.join("Home.md"), "local edit\n").expect("local edit");

        let error = synchronize_managed_checkout(&config).expect_err("dirty pull-only work");

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

        let outcome = synchronize_managed_checkout(&config).expect("pull-only sync");

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

        let error =
            synchronize_managed_checkout(&config).expect_err("local history is unsupported");

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

        let outcome = synchronize_managed_checkout(&config).expect("two-way sync");

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
    fn two_way_merges_diverged_histories_and_pushes_the_merge() {
        let (root, config) = fixture(ManagedSyncMode::TwoWay);
        remote_commit(root.path(), "vault/Remote.md", "remote\n", "remote change");
        std::fs::write(config.vault_path.join("Local.md"), "local\n").expect("local edit");

        let outcome = synchronize_managed_checkout(&config).expect("diverged sync");

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

        let outcome = synchronize_two_way_with_push(&checkout, &config, |repository, config| {
            if !injected_race {
                injected_race = true;
                remote_commit(root.path(), "vault/Race.md", "race\n", "push race");
            }
            push(repository, config)
        })
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

        let error = synchronize_managed_checkout(&config).expect_err("merge conflict");

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

        let error = synchronize_managed_checkout(&config).expect_err("outside dirty work");

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

        let error = synchronize_managed_checkout(&config).expect_err("unsafe origin");

        assert_eq!(error, ManagedSyncError::Validation);
        assert!(!error.to_string().contains("private-token"));
    }

    #[test]
    fn credential_bearing_secondary_remote_is_rejected_without_revealing_it() {
        let (_root, config) = fixture(ManagedSyncMode::PullOnly);
        let checkout = Repository::open(&config.repository_path).expect("checkout");
        checkout
            .remote("backup", "https://private-token@example.test/vault.git")
            .expect("secondary remote");

        let error = synchronize_managed_checkout(&config).expect_err("unsafe secondary remote");

        assert_eq!(error, ManagedSyncError::Validation);
        assert!(!error.to_string().contains("private-token"));
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

        let error = synchronize_managed_checkout(&config).expect_err("invalid Vault root");

        assert_eq!(error, ManagedSyncError::Validation);
    }

    #[test]
    fn malformed_https_origin_is_rejected_by_the_shared_registry_policy() {
        let (_root, config) = fixture(ManagedSyncMode::PullOnly);
        let checkout = Repository::open(&config.repository_path).expect("checkout");
        checkout
            .remote_set_url("origin", "https://example.test/not valid.git")
            .expect("tamper origin");

        let error = synchronize_managed_checkout(&config).expect_err("malformed origin");

        assert_eq!(error, ManagedSyncError::Validation);
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

        let error = synchronize_managed_checkout(&config).expect_err("outside conflict");

        assert!(matches!(error, ManagedSyncError::Conflict { .. }));
        let checkout = Repository::open(&config.repository_path).expect("checkout");
        assert_eq!(checkout.state(), git2::RepositoryState::Clean);
        assert_eq!(file_at_head(&checkout, "outside.md"), "local\n");
    }
}
