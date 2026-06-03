# MCP Git Sync Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Automatically commit and push vault changes to a git remote after MCP write tools run, so edits made by remote agents propagate to every synced device.

**Architecture:** A new opt-in `src/git/` module wraps the `git2` crate to stage-commit-fetch-merge-push the vault repo. MCP write tools push a `WriteRecord` (affected paths + optional summary) onto a channel; a single background task debounces these, then runs one sync under a shared vault-mutation lock. A merge conflict aborts cleanly and keeps the local commit. Sync results are surfaced via a `get_git_sync_status` MCP tool and a warning field on write responses. Off by default; fails fast at startup when enabled-but-misconfigured.

**Tech Stack:** Rust 2024, `git2` (vendored libgit2 + OpenSSL, distroless-safe), `tokio` (mpsc channel + background task + `Mutex`), `axum`, existing `serde_json` MCP plumbing.

**Spec:** `docs/superpowers/specs/2026-06-03-mcp-git-sync-design.md`

---

## Design notes the implementer must keep in mind

- **Repository handle lifetime.** `git2::Repository` is `Send` but not `Sync`. To avoid lifetime/threading friction, **open the repository fresh inside each sync call** (cheap). The long-lived background task holds only the `GitConfig` (plain `Send + Sync` data), the channel receiver, the status `Arc`, and the vault lock `Arc`.
- **Blocking work.** `git2` is synchronous and blocking. Run every git operation inside `tokio::task::spawn_blocking`. Hold the async `vault_write_lock` guard across that `.await` (a `tokio::sync::MutexGuard` is `Send`, so this compiles).
- **Affected paths are absolute.** `WriteOutcome`/`AttachmentOutcome` will carry `affected_paths: Vec<PathBuf>` as absolute filesystem paths (every write function already has these). The git layer strips the repo workdir prefix to get index-relative paths.
- **Staging precision.** Stage only the batch's reported paths. A path that still exists on disk is `add_path`; a path that no longer exists (delete / rename source) is `remove_path`.
- **Secret hygiene.** The token is only ever passed to `Cred::userpass_plaintext`. Never log it, never write it into `.git/config`, never put it in `GitSyncStatus`.

---

## Task 1: Add the `git2` dependency (vendored, distroless-safe)

**Files:**
- Modify: `Cargo.toml:7-30` (the `[dependencies]` table)

- [ ] **Step 1: Add the dependency**

In `Cargo.toml`, add to `[dependencies]` (keep the list alphabetical — insert after `fastembed`):

```toml
git2 = { version = "0.20", default-features = false, features = ["vendored-libgit2", "vendored-openssl"] }
```

- [ ] **Step 2: Verify it builds**

Run: `cargo build`
Expected: PASS. First build is slow (compiles vendored libgit2 + OpenSSL). If it fails on a missing C toolchain, confirm `g++`/`pkg-config` are present (they are in the Dockerfile builder stage and on the dev machine).

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "build: add git2 (vendored) for MCP git sync"
```

---

## Task 2: Git configuration (`GitConfig::from_env`)

**Files:**
- Create: `src/git/mod.rs`
- Create: `src/git/config.rs`
- Modify: `src/lib.rs:8` (register the module)

- [ ] **Step 1: Register the module**

In `src/lib.rs`, add `pub mod git;` after `pub mod embed;` (keep alphabetical):

```rust
pub mod embed;
pub mod eval;
pub mod git;
pub mod handlers;
```

- [ ] **Step 2: Create the module root**

Create `src/git/mod.rs`:

```rust
pub mod config;
pub mod sync;

pub use config::GitConfig;
pub use sync::{GitError, SyncOutcome, SyncReport, validate_repo};
```

- [ ] **Step 3: Write the failing config test**

Create `src/git/config.rs`:

```rust
use std::env;

/// Static configuration for the git-sync subsystem, read once at startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitConfig {
    /// Absolute path to the vault, which must be the git repository root.
    pub vault_path: std::path::PathBuf,
    /// Remote name to fetch/push (e.g. "origin").
    pub remote: String,
    /// Branch to commit and push.
    pub branch: String,
    /// HTTPS auth username. Many providers accept any non-empty value with a token.
    pub username: String,
    /// HTTPS auth token. Never logged or surfaced.
    pub token: String,
    /// Quiet window before a batch is committed and pushed.
    pub debounce_seconds: u64,
    /// Commit author/committer name.
    pub author_name: String,
    /// Commit author/committer email.
    pub author_email: String,
}

impl GitConfig {
    /// Returns `Ok(None)` when git sync is disabled, `Ok(Some(_))` when enabled and
    /// fully configured, and `Err(_)` when enabled but a required value is missing.
    pub fn from_env(vault_path: std::path::PathBuf) -> Result<Option<Self>, String> {
        let enabled = env::var("HATCHDOOR_GIT_SYNC_ENABLED")
            .map(|v| is_truthy(&v))
            .unwrap_or(false);
        if !enabled {
            return Ok(None);
        }

        let token = non_empty_env("HATCHDOOR_GIT_HTTPS_TOKEN")
            .ok_or("HATCHDOOR_GIT_SYNC_ENABLED is set but HATCHDOOR_GIT_HTTPS_TOKEN is missing")?;
        let remote = non_empty_env("HATCHDOOR_GIT_REMOTE").unwrap_or_else(|| "origin".to_string());
        let branch = non_empty_env("HATCHDOOR_GIT_BRANCH").unwrap_or_else(|| "main".to_string());
        let username =
            non_empty_env("HATCHDOOR_GIT_HTTPS_USERNAME").unwrap_or_else(|| "hatchdoor".to_string());
        let debounce_seconds = env::var("HATCHDOOR_GIT_DEBOUNCE_SECONDS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(30);
        let author_name =
            non_empty_env("HATCHDOOR_GIT_AUTHOR_NAME").unwrap_or_else(|| "Hatchdoor".to_string());
        let author_email = non_empty_env("HATCHDOOR_GIT_AUTHOR_EMAIL")
            .unwrap_or_else(|| "hatchdoor@localhost".to_string());

        Ok(Some(Self {
            vault_path,
            remote,
            branch,
            username,
            token,
            debounce_seconds,
            author_name,
            author_email,
        }))
    }
}

fn non_empty_env(key: &str) -> Option<String> {
    env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn is_truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Mutex;

    // Env access is process-global; serialize these tests.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn clear_env() {
        for key in [
            "HATCHDOOR_GIT_SYNC_ENABLED",
            "HATCHDOOR_GIT_HTTPS_TOKEN",
            "HATCHDOOR_GIT_REMOTE",
            "HATCHDOOR_GIT_BRANCH",
            "HATCHDOOR_GIT_HTTPS_USERNAME",
            "HATCHDOOR_GIT_DEBOUNCE_SECONDS",
            "HATCHDOOR_GIT_AUTHOR_NAME",
            "HATCHDOOR_GIT_AUTHOR_EMAIL",
        ] {
            unsafe { env::remove_var(key) };
        }
    }

    #[test]
    fn disabled_when_flag_unset() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_env();
        let cfg = GitConfig::from_env(PathBuf::from("/vault")).expect("ok");
        assert_eq!(cfg, None);
    }

    #[test]
    fn enabled_requires_token() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_env();
        unsafe { env::set_var("HATCHDOOR_GIT_SYNC_ENABLED", "true") };
        let result = GitConfig::from_env(PathBuf::from("/vault"));
        assert!(result.is_err());
        clear_env();
    }

    #[test]
    fn applies_defaults_when_enabled_with_token() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_env();
        unsafe {
            env::set_var("HATCHDOOR_GIT_SYNC_ENABLED", "1");
            env::set_var("HATCHDOOR_GIT_HTTPS_TOKEN", "secret");
        }
        let cfg = GitConfig::from_env(PathBuf::from("/vault"))
            .expect("ok")
            .expect("enabled");
        assert_eq!(cfg.remote, "origin");
        assert_eq!(cfg.branch, "main");
        assert_eq!(cfg.username, "hatchdoor");
        assert_eq!(cfg.debounce_seconds, 30);
        assert_eq!(cfg.author_email, "hatchdoor@localhost");
        assert_eq!(cfg.token, "secret");
        clear_env();
    }
}
```

> Note: this test file references `pub mod sync;` and `validate_repo` which arrive in Task 3. To compile Task 2 in isolation, the implementer may temporarily stub `src/git/sync.rs` with the types defined in Task 3 Step 1; otherwise implement Task 2 and Task 3 together before running the suite. Prefer implementing both, then running tests once.

- [ ] **Step 4: Run config tests**

Run: `cargo test --lib git::config`
Expected: PASS (after Task 3 types exist).

- [ ] **Step 5: Commit**

```bash
git add src/lib.rs src/git/mod.rs src/git/config.rs
git commit -m "feat(git): add GitConfig env parsing for sync subsystem"
```

---

## Task 3: Git sync core — stage, commit, fetch, merge, push

This is the heart of the feature. It is pure git plumbing with no async or app state, fully testable against a local bare "remote".

**Files:**
- Create: `src/git/sync.rs`
- Test: in-file `#[cfg(test)]` module in `src/git/sync.rs`

- [ ] **Step 1: Write the module skeleton with types and the conflict/credential helpers**

Create `src/git/sync.rs`:

```rust
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
```

- [ ] **Step 2: Write the failing validation test**

Add to the `#[cfg(test)]` module at the bottom of `src/git/sync.rs`:

```rust
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
}
```

- [ ] **Step 3: Run validation test to verify it fails**

Run: `cargo test --lib git::sync::tests::validate`
Expected: FAIL — `validate_repo` not defined.

- [ ] **Step 4: Implement `validate_repo` and `has_unpushed`**

Add to `src/git/sync.rs` (above the test module):

```rust
/// Startup validation: vault is a repo whose root is the vault, HEAD is on the
/// configured branch, and the configured remote exists.
pub fn validate_repo(config: &GitConfig) -> Result<(), GitError> {
    let repo = Repository::open(&config.vault_path)
        .map_err(|e| GitError::Validation(format!("cannot open vault as git repo: {}", e.message())))?;

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

fn same_path(a: &Path, b: &Path) -> bool {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    canon(a) == canon(b)
}
```

- [ ] **Step 5: Run validation test to verify it passes**

Run: `cargo test --lib git::sync::tests::validate`
Expected: PASS.

- [ ] **Step 6: Write the failing sync test (fast-forward push)**

Add to the test module in `src/git/sync.rs`:

```rust
    fn remote_head_message(remote_dir: &Path, branch: &str) -> String {
        let repo = Repository::open_bare(remote_dir).unwrap();
        let oid = repo
            .refname_to_id(&format!("refs/heads/{branch}"))
            .unwrap();
        repo.find_commit(oid).unwrap().message().unwrap().to_string()
    }

    #[test]
    fn sync_commits_and_pushes_new_file() {
        let (_tmp, work, remote) = init_repo_with_remote();
        let config = base_config(&work);
        let new_file = work.join("Note.md");
        fs::write(&new_file, "# Note\n").unwrap();

        let report = sync(&config, &[new_file.clone()], "hatchdoor: add Note").unwrap();
        assert_eq!(report.outcome, SyncOutcome::Pushed { committed: true });
        assert_eq!(remote_head_message(&remote, "main"), "hatchdoor: add Note");
    }

    #[test]
    fn sync_stages_deletion() {
        let (_tmp, work, remote) = init_repo_with_remote();
        let config = base_config(&work);
        let home = work.join("Home.md");
        fs::remove_file(&home).unwrap();

        let report = sync(&config, &[home.clone()], "hatchdoor: delete Home").unwrap();
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
```

- [ ] **Step 7: Run to verify it fails**

Run: `cargo test --lib git::sync::tests::sync_`
Expected: FAIL — `sync` not defined.

- [ ] **Step 8: Implement staging + commit + push (no merge yet)**

Add to `src/git/sync.rs`:

```rust
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
```

- [ ] **Step 9: Implement the merge + conflict-abort + push helpers**

Add to `src/git/sync.rs`:

```rust
fn merge_remote(
    repo: &Repository,
    config: &GitConfig,
    their: &AnnotatedCommit,
    local_oid: git2::Oid,
) -> Result<(), GitError> {
    let mut merge_opts = MergeOptions::new();
    repo.merge(&[their], Some(&mut merge_opts), None)?;

    let mut index = repo.index()?;
    if index.has_conflicts() {
        // Collect conflicting paths for the error, then abort cleanly.
        let mut files = Vec::new();
        if let Ok(conflicts) = index.conflicts() {
            for conflict in conflicts.flatten() {
                if let Some(entry) = conflict.our.or(conflict.their) {
                    if let Ok(path) = std::str::from_utf8(&entry.path) {
                        files.push(path.to_string());
                    }
                }
            }
        }
        // Abort: clear merge state and hard-reset back to our commit.
        repo.cleanup_state()?;
        let our_commit = repo.find_commit(local_oid)?;
        repo.reset(our_commit.as_object(), ResetType::Hard, None)?;
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
        &format!("Merge remote {}/{} into {}", config.remote, config.branch, config.branch),
        &tree,
        &[&our_commit, &their_commit],
    )?;
    repo.cleanup_state()?;
    // Make the working tree match the new HEAD.
    repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))?;
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
```

- [ ] **Step 10: Export the new items**

In `src/git/mod.rs`, update the re-export:

```rust
pub use sync::{GitError, SyncOutcome, SyncReport, has_unpushed, sync, validate_repo};
```

- [ ] **Step 11: Run the push tests**

Run: `cargo test --lib git::sync`
Expected: PASS (fast-forward push, deletion, no-op).

- [ ] **Step 12: Write the failing merge tests (clean + conflict)**

Add to the test module:

```rust
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
        let head_msg = repo.find_commit(head).unwrap().message().unwrap().to_string();
        assert_eq!(head_msg, "hatchdoor: edit Home");
        assert_eq!(remote_head_message(&remote, "main"), "remote edit");
    }
```

- [ ] **Step 13: Run merge tests**

Run: `cargo test --lib git::sync`
Expected: PASS — both clean merge and conflict-abort behave as asserted. If the conflict test fails because the working tree was not restored, confirm the `ResetType::Hard` reset in `merge_remote` runs after `cleanup_state`.

- [ ] **Step 14: Commit**

```bash
git add src/git/mod.rs src/git/sync.rs
git commit -m "feat(git): implement stage/commit/fetch/merge/push sync core"
```

---

## Task 4: Sync status type + commit-message builder

**Files:**
- Create: `src/git/status.rs`
- Create: `src/git/message.rs`
- Modify: `src/git/mod.rs`

- [ ] **Step 1: Write the failing message-builder test**

Create `src/git/message.rs`:

```rust
/// One vault write that contributed to a debounced batch.
#[derive(Debug, Clone)]
pub struct WriteRecord {
    /// Short human label of the operation, e.g. "create", "update", "delete".
    pub op: String,
    /// A display name for the primary target, e.g. "Projects/New".
    pub target: String,
    /// Absolute paths the operation created, modified, or removed.
    pub affected_paths: Vec<std::path::PathBuf>,
    /// Optional agent-supplied summary line.
    pub summary: Option<String>,
}

/// Build a commit message from a batch of write records.
/// Title summarizes ops + file count; body lists agent-supplied summaries.
pub fn build_commit_message(records: &[WriteRecord]) -> String {
    if records.is_empty() {
        return "hatchdoor: vault update".to_string();
    }

    let file_count: usize = {
        let mut paths: Vec<&std::path::Path> =
            records.iter().flat_map(|r| r.affected_paths.iter().map(|p| p.as_path())).collect();
        paths.sort();
        paths.dedup();
        paths.len()
    };

    let mut highlights: Vec<String> = records
        .iter()
        .take(3)
        .map(|r| format!("{} \"{}\"", r.op, r.target))
        .collect();
    if records.len() > 3 {
        highlights.push(format!("+{} more", records.len() - 3));
    }

    let title = format!("hatchdoor: {} ({file_count} files)", highlights.join(", "));

    let body: Vec<String> = records
        .iter()
        .filter_map(|r| r.summary.as_ref())
        .map(|s| format!("- {s}"))
        .collect();

    if body.is_empty() {
        title
    } else {
        format!("{title}\n\n{}", body.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn record(op: &str, target: &str, paths: &[&str], summary: Option<&str>) -> WriteRecord {
        WriteRecord {
            op: op.to_string(),
            target: target.to_string(),
            affected_paths: paths.iter().map(PathBuf::from).collect(),
            summary: summary.map(str::to_string),
        }
    }

    #[test]
    fn title_summarizes_ops_and_unique_file_count() {
        let records = vec![
            record("update", "Project X", &["/v/Project X.md", "/v/Other.md"], None),
            record("create", "Meeting", &["/v/Meeting.md"], None),
        ];
        let msg = build_commit_message(&records);
        assert_eq!(
            msg,
            "hatchdoor: update \"Project X\", create \"Meeting\" (3 files)"
        );
    }

    #[test]
    fn body_lists_agent_summaries() {
        let records = vec![record("update", "Project X", &["/v/Project X.md"], Some("tighten intro"))];
        let msg = build_commit_message(&records);
        assert!(msg.starts_with("hatchdoor: update \"Project X\" (1 files)"));
        assert!(msg.ends_with("- tighten intro"));
    }

    #[test]
    fn empty_batch_has_fallback_message() {
        assert_eq!(build_commit_message(&[]), "hatchdoor: vault update");
    }
}
```

- [ ] **Step 2: Run message test to verify it fails then passes**

Run: `cargo test --lib git::message`
Expected: FAIL (module not registered) → after Step 3 registration, PASS.

- [ ] **Step 3: Create the status type**

Create `src/git/status.rs`:

```rust
use serde::Serialize;

/// Shared, observable state of the git-sync subsystem.
#[derive(Debug, Clone, Default, Serialize)]
pub struct GitSyncStatus {
    /// Whether the subsystem is enabled.
    pub enabled: bool,
    /// RFC3339 timestamp of the last completed sync attempt, if any.
    pub last_sync_at: Option<String>,
    /// True when the last attempt succeeded (pushed or no-op).
    pub last_ok: bool,
    /// Human-readable error from the last failed attempt (token redacted upstream).
    pub last_error: Option<String>,
    /// Write records waiting for the next debounced sync.
    pub pending: usize,
}

impl GitSyncStatus {
    pub fn enabled() -> Self {
        Self {
            enabled: true,
            ..Default::default()
        }
    }
}
```

- [ ] **Step 4: Register both modules**

In `src/git/mod.rs`:

```rust
pub mod config;
pub mod message;
pub mod status;
pub mod sync;

pub use config::GitConfig;
pub use message::{WriteRecord, build_commit_message};
pub use status::GitSyncStatus;
pub use sync::{GitError, SyncOutcome, SyncReport, has_unpushed, sync, validate_repo};
```

- [ ] **Step 5: Run all git tests**

Run: `cargo test --lib git`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/git/mod.rs src/git/message.rs src/git/status.rs
git commit -m "feat(git): add WriteRecord, commit-message builder, and sync status"
```

---

## Task 5: Background sync task (debounce + run under lock)

**Files:**
- Create: `src/git/task.rs`
- Modify: `src/git/mod.rs`

- [ ] **Step 1: Write the failing debounce/coalesce test**

Create `src/git/task.rs`:

```rust
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex, RwLock, mpsc};
use tracing::{error, info, warn};

use super::config::GitConfig;
use super::message::{WriteRecord, build_commit_message};
use super::status::GitSyncStatus;
use super::sync::{GitError, SyncOutcome};

/// Handle stored in AppState so write tools can enqueue records and readers can
/// observe status. `None` everywhere when git sync is disabled.
#[derive(Clone)]
pub struct GitSyncHandle {
    sender: mpsc::UnboundedSender<WriteRecord>,
    status: Arc<RwLock<GitSyncStatus>>,
}

impl GitSyncHandle {
    /// Enqueue a write for the next debounced sync. Never blocks; drops silently
    /// if the background task has stopped (it will be retried by later writes).
    pub fn record(&self, record: WriteRecord) {
        let _ = self.sender.send(record);
    }

    pub fn status(&self) -> Arc<RwLock<GitSyncStatus>> {
        self.status.clone()
    }
}

/// Spawn the background sync task. `vault_lock` is the shared vault-mutation lock,
/// also acquired by MCP write tools. `runner` performs the actual git work; in
/// production this calls `super::sync::sync`, and tests inject a fake.
pub fn spawn_sync_task<R>(
    config: GitConfig,
    vault_lock: Arc<Mutex<()>>,
    runner: R,
) -> GitSyncHandle
where
    R: Fn(&GitConfig, &[std::path::PathBuf], &str) -> Result<SyncOutcome, GitError>
        + Send
        + Sync
        + 'static,
{
    let (sender, receiver) = mpsc::unbounded_channel();
    let status = Arc::new(RwLock::new(GitSyncStatus::enabled()));
    let task_status = status.clone();
    let debounce = Duration::from_secs(config.debounce_seconds.max(1));
    let runner = Arc::new(runner);

    tokio::spawn(async move {
        run_loop(config, debounce, vault_lock, receiver, task_status, runner).await;
    });

    GitSyncHandle { sender, status }
}

async fn run_loop<R>(
    config: GitConfig,
    debounce: Duration,
    vault_lock: Arc<Mutex<()>>,
    mut receiver: mpsc::UnboundedReceiver<WriteRecord>,
    status: Arc<RwLock<GitSyncStatus>>,
    runner: Arc<R>,
) where
    R: Fn(&GitConfig, &[std::path::PathBuf], &str) -> Result<SyncOutcome, GitError>
        + Send
        + Sync
        + 'static,
{
    let mut batch: Vec<WriteRecord> = Vec::new();

    loop {
        // Wait for the first record (or channel close).
        let first = match receiver.recv().await {
            Some(record) => record,
            None => break,
        };
        batch.push(first);
        update_pending(&status, batch.len()).await;

        // Debounce: keep extending the quiet window while records keep arriving.
        loop {
            let timer = tokio::time::sleep(debounce);
            tokio::pin!(timer);
            tokio::select! {
                _ = &mut timer => break,
                maybe = receiver.recv() => match maybe {
                    Some(record) => {
                        batch.push(record);
                        update_pending(&status, batch.len()).await;
                    }
                    None => break,
                }
            }
        }

        run_one_sync(&config, &vault_lock, &status, &runner, std::mem::take(&mut batch)).await;
        update_pending(&status, 0).await;
    }
}

async fn run_one_sync<R>(
    config: &GitConfig,
    vault_lock: &Arc<Mutex<()>>,
    status: &Arc<RwLock<GitSyncStatus>>,
    runner: &Arc<R>,
    batch: Vec<WriteRecord>,
) where
    R: Fn(&GitConfig, &[std::path::PathBuf], &str) -> Result<SyncOutcome, GitError>
        + Send
        + Sync
        + 'static,
{
    let message = build_commit_message(&batch);
    let mut paths: Vec<std::path::PathBuf> =
        batch.iter().flat_map(|r| r.affected_paths.clone()).collect();
    paths.sort();
    paths.dedup();

    // Hold the vault lock across the blocking git work so no MCP write races it.
    let _guard = vault_lock.lock().await;
    let config_clone = config.clone();
    let runner = runner.clone();
    let result = tokio::task::spawn_blocking(move || runner(&config_clone, &paths, &message))
        .await
        .unwrap_or_else(|join_err| Err(GitError::Other(format!("sync task panicked: {join_err}"))));
    drop(_guard);

    let mut guard = status.write().await;
    guard.last_sync_at = Some(now_rfc3339());
    match result {
        Ok(outcome) => {
            guard.last_ok = true;
            guard.last_error = None;
            match outcome {
                SyncOutcome::NoChanges => info!("git sync: no changes"),
                SyncOutcome::Pushed { committed } => {
                    info!(committed, "git sync: pushed")
                }
            }
        }
        Err(err) => {
            guard.last_ok = false;
            let message = err.to_string();
            match &err {
                GitError::Conflict { .. } => warn!("git sync conflict: {message}"),
                _ => error!("git sync failed: {message}"),
            }
            guard.last_error = Some(message);
        }
    }
}

async fn update_pending(status: &Arc<RwLock<GitSyncStatus>>, pending: usize) {
    status.write().await.pending = pending;
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test(start_paused = true)]
    async fn coalesces_records_into_single_sync() {
        let calls = Arc::new(AtomicUsize::new(0));
        let batch_sizes = Arc::new(Mutex::new(Vec::<usize>::new()));
        let calls_for_runner = calls.clone();
        let sizes_for_runner = batch_sizes.clone();

        let config = GitConfig {
            vault_path: std::path::PathBuf::from("/unused"),
            remote: "origin".into(),
            branch: "main".into(),
            username: "u".into(),
            token: "t".into(),
            debounce_seconds: 5,
            author_name: "n".into(),
            author_email: "e".into(),
        };
        let lock = Arc::new(Mutex::new(()));

        let handle = spawn_sync_task(config, lock, move |_cfg, paths, _msg| {
            calls_for_runner.fetch_add(1, Ordering::SeqCst);
            sizes_for_runner.blocking_lock().push(paths.len());
            Ok(SyncOutcome::Pushed { committed: true })
        });

        for i in 0..3 {
            handle.record(WriteRecord {
                op: "update".into(),
                target: format!("n{i}"),
                affected_paths: vec![std::path::PathBuf::from(format!("/v/n{i}.md"))],
                summary: None,
            });
        }

        // Advance past the debounce window and let the task run.
        tokio::time::advance(Duration::from_secs(6)).await;
        tokio::time::sleep(Duration::from_millis(1)).await;

        assert_eq!(calls.load(Ordering::SeqCst), 1, "one coalesced sync");
        assert_eq!(batch_sizes.lock().await.as_slice(), &[3]);
    }

    #[tokio::test(start_paused = true)]
    async fn records_error_in_status() {
        let config = GitConfig {
            vault_path: std::path::PathBuf::from("/unused"),
            remote: "origin".into(),
            branch: "main".into(),
            username: "u".into(),
            token: "t".into(),
            debounce_seconds: 1,
            author_name: "n".into(),
            author_email: "e".into(),
        };
        let lock = Arc::new(Mutex::new(()));
        let handle = spawn_sync_task(config, lock, move |_c, _p, _m| {
            Err(GitError::Remote("boom".into()))
        });
        let status = handle.status();

        handle.record(WriteRecord {
            op: "update".into(),
            target: "n".into(),
            affected_paths: vec![std::path::PathBuf::from("/v/n.md")],
            summary: None,
        });
        tokio::time::advance(Duration::from_secs(2)).await;
        tokio::time::sleep(Duration::from_millis(1)).await;

        let guard = status.read().await;
        assert!(!guard.last_ok);
        assert_eq!(guard.last_error.as_deref(), Some("git remote error: boom"));
    }
}
```

- [ ] **Step 2: Add the `chrono` import note**

`chrono` is already a dependency (`Cargo.toml`). No change needed.

- [ ] **Step 3: Register the module and export the handle**

In `src/git/mod.rs` add:

```rust
pub mod task;
```

and extend the re-exports:

```rust
pub use task::{GitSyncHandle, spawn_sync_task};
```

- [ ] **Step 4: Run the task tests**

Run: `cargo test --lib git::task`
Expected: PASS — coalescing produces exactly one sync of 3 paths; the error path is recorded in status.

- [ ] **Step 5: Commit**

```bash
git add src/git/mod.rs src/git/task.rs
git commit -m "feat(git): add debounced background sync task with status reporting"
```

---

## Task 6: Thread affected paths out of the write layer

`WriteOutcome` and `AttachmentOutcome` currently expose only counts. Add absolute `affected_paths` so the MCP layer can build precise `WriteRecord`s.

**Files:**
- Modify: `src/vault/write/types.rs:4-27`
- Modify: `src/vault/write/rewrites.rs:168-175`
- Modify: `src/vault/write/notes.rs` (every constructor of `WriteOutcome`)
- Modify: `src/vault/write/attachments.rs` (every constructor of `AttachmentOutcome`)

- [ ] **Step 1: Add the field to the outcome types**

In `src/vault/write/types.rs`, add `affected_paths` to both structs:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteOutcome {
    pub slug: Option<String>,
    pub relative_path: Option<String>,
    pub content_hash: Option<String>,
    pub rewritten_notes: usize,
    pub moved_assets: usize,
    pub trashed_path: Option<String>,
    /// Absolute paths created, modified, or removed by this operation
    /// (primary file, rewritten backlink notes, moved assets, rename source).
    pub affected_paths: Vec<std::path::PathBuf>,
}
```

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentOutcome {
    pub attachment: AttachmentInfo,
    pub rewritten_notes: usize,
    pub trashed_path: Option<String>,
    pub cleanup_warning: Option<String>,
    /// Absolute paths created, modified, or removed by this operation.
    pub affected_paths: Vec<std::path::PathBuf>,
}
```

- [ ] **Step 2: Make `apply_rewrites` return the rewritten paths**

In `src/vault/write/rewrites.rs`, change `apply_rewrites` to return the paths it wrote:

```rust
pub(super) fn apply_rewrites(rewrites: Vec<TextRewrite>) -> Result<Vec<std::path::PathBuf>, WriteError> {
    let mut written = Vec::with_capacity(rewrites.len());
    for rewrite in rewrites {
        atomic_write(&rewrite.path, &rewrite.content)?;
        written.push(rewrite.path);
    }
    Ok(written)
}
```

Update the `rollback_rewrites` caller (it ignores the result):

```rust
let _ = apply_rewrites(rewrites);
```

(unchanged — `let _ =` already discards; it now discards a `Vec` instead of a `usize`, which still compiles.)

- [ ] **Step 3: Populate `affected_paths` in `notes.rs`**

Update each `WriteOutcome { .. }` in `src/vault/write/notes.rs`:

`create_note` (after `atomic_write(&path, content)?;`):

```rust
    Ok(WriteOutcome {
        slug: None,
        relative_path: Some(strip_md_extension(&normalized).to_string()),
        content_hash: Some(content_hash(content)),
        rewritten_notes: 0,
        moved_assets: 0,
        trashed_path: None,
        affected_paths: vec![path.clone()],
    })
```

`update_note`, `append_note`, `edit_note`, `replace_section` each write `entry.path`; add to each returned struct:

```rust
        affected_paths: vec![entry.path.clone()],
```

(append it as the final field in all four `WriteOutcome { .. }` literals.)

`move_or_rename_note` — capture rewrite paths and assemble the set:

```rust
    let moved_assets = move_assets(&asset_moves)?;
    let rewritten = apply_rewrites(merge_rewrites(backlink_rewrites, asset_rewrites))?;
    let rewritten_notes = rewritten.len();
    let moved_content = fs::read_to_string(&target_path).map_err(|error| {
        WriteError::Io(format!(
            "failed to read moved note '{}': {error}",
            target_path.display()
        ))
    })?;

    let mut affected_paths = rewritten;
    affected_paths.push(entry.path.clone()); // rename source (now removed)
    affected_paths.push(target_path.clone()); // rename destination
    for asset in &asset_moves {
        affected_paths.push(asset.source.clone());
        affected_paths.push(asset.destination.clone());
    }

    Ok(WriteOutcome {
        slug: None,
        relative_path: Some(target_without_ext),
        content_hash: Some(content_hash(&moved_content)),
        rewritten_notes,
        moved_assets,
        trashed_path: None,
        affected_paths,
    })
```

`delete_note` — similar, but the moved file goes to trash:

```rust
    let moved_assets = move_assets(&asset_moves)?;
    fs::rename(&entry.path, &trash_path).map_err(|error| {
        WriteError::Io(format!(
            "failed to move note '{}' to trash '{}': {error}",
            entry.path.display(),
            trash_path.display()
        ))
    })?;
    let rewritten = apply_rewrites(merge_rewrites(backlink_rewrites, asset_rewrites))?;
    let rewritten_notes = rewritten.len();

    let mut affected_paths = rewritten;
    affected_paths.push(entry.path.clone()); // original (now removed)
    affected_paths.push(trash_path.clone()); // trashed copy
    for asset in &asset_moves {
        affected_paths.push(asset.source.clone());
        affected_paths.push(asset.destination.clone());
    }

    Ok(WriteOutcome {
        slug: Some(entry.slug.clone()),
        relative_path: Some(entry.relative_path.clone()),
        content_hash: None,
        rewritten_notes,
        moved_assets,
        trashed_path: Some(trash_relative),
        affected_paths,
    })
```

> Note: `AssetMove`'s `source`/`destination` fields are `pub(super)` (see `types.rs:42-46`), so `notes.rs` (same `write` module) can read them.

- [ ] **Step 4: Populate `affected_paths` in `attachments.rs`**

`import_attachment` (only the target is touched):

```rust
    Ok(AttachmentOutcome {
        attachment: attachment_info(vault_root, &target_path)?,
        rewritten_notes: 0,
        trashed_path: None,
        cleanup_warning,
        affected_paths: vec![target_path.clone()],
    })
```

`move_attachment` — capture rewrite paths:

```rust
    let rewrites =
        asset_reference_rewrite_plan(vault_root, index, "", &source_path, &target_path, &[])?;
    let rewritten = apply_rewrites(rewrites)?;
    let rewritten_notes = rewritten.len();
    fs::rename(&source_path, &target_path).map_err(|error| {
        rollback_rewrites(vault_root, index, &target_path, &source_path);
        WriteError::Io(format!(
            "failed to move attachment '{}' to '{}': {error}",
            source_path.display(),
            target_path.display()
        ))
    })?;
    let mut affected_paths = rewritten;
    affected_paths.push(source_path.clone());
    affected_paths.push(target_path.clone());
    Ok(AttachmentOutcome {
        attachment: attachment_info(vault_root, &target_path)?,
        rewritten_notes,
        trashed_path: None,
        cleanup_warning: None,
        affected_paths,
    })
```

`delete_attachment` — same shape, destination is `trash_path`:

```rust
    let rewrites =
        asset_reference_rewrite_plan(vault_root, index, "", &source_path, &trash_path, &[])?;
    let rewritten = apply_rewrites(rewrites)?;
    let rewritten_notes = rewritten.len();
    fs::rename(&source_path, &trash_path).map_err(|error| {
        rollback_rewrites(vault_root, index, &trash_path, &source_path);
        WriteError::Io(format!(
            "failed to trash attachment '{}' to '{}': {error}",
            source_path.display(),
            trash_path.display()
        ))
    })?;
    let mut affected_paths = rewritten;
    affected_paths.push(source_path.clone());
    affected_paths.push(trash_path.clone());
    Ok(AttachmentOutcome {
        attachment: attachment_info(vault_root, &trash_path)?,
        rewritten_notes,
        trashed_path: Some(trash_relative),
        cleanup_warning: None,
        affected_paths,
    })
```

`move_attachment_by_paths` (used by `rename_attachment`):

```rust
    let rewrites =
        asset_reference_rewrite_plan(vault_root, index, "", source_path, target_path, &[])?;
    let rewritten = apply_rewrites(rewrites)?;
    let rewritten_notes = rewritten.len();
    fs::rename(source_path, target_path).map_err(|error| {
        rollback_rewrites(vault_root, index, target_path, source_path);
        WriteError::Io(format!(
            "failed to move attachment '{}' to '{}': {error}",
            source_path.display(),
            target_path.display()
        ))
    })?;
    let mut affected_paths = rewritten;
    affected_paths.push(source_path.to_path_buf());
    affected_paths.push(target_path.to_path_buf());
    Ok(AttachmentOutcome {
        attachment: attachment_info(vault_root, target_path)?,
        rewritten_notes,
        trashed_path: None,
        cleanup_warning: None,
        affected_paths,
    })
```

- [ ] **Step 5: Fix existing write-layer tests**

Run: `cargo test --lib vault::write`
Expected: Some `WriteOutcome`/`AttachmentOutcome` literals in `src/vault/write/tests.rs` may fail to compile because they construct these structs directly or compare with `==`. For any direct struct construction in tests, add `affected_paths: vec![]` (or the expected paths). For equality assertions, update expectations to include the new field. Fix until the suite compiles and passes.

- [ ] **Step 6: Commit**

```bash
git add src/vault/write/types.rs src/vault/write/rewrites.rs src/vault/write/notes.rs src/vault/write/attachments.rs src/vault/write/tests.rs
git commit -m "feat(vault): expose affected paths from write outcomes"
```

---

## Task 7: Wire git sync into AppState + startup

**Files:**
- Modify: `src/app_state.rs:64-78` (`AppState` struct)
- Modify: `src/main.rs:99-122` (startup wiring)
- Modify: every `AppState { .. }` literal in tests (`src/main.rs`, `src/app_state.rs`, `src/mcp/routes.rs`)

- [ ] **Step 1: Add fields to `AppState`**

In `src/app_state.rs`, extend the struct and add the new imports at the top:

```rust
use std::sync::Mutex as StdMutexUnused; // (do NOT add; placeholder reminder)
```

(Do not add the line above — it is only a reminder that the lock is a tokio mutex.) Add to the struct:

```rust
#[derive(Clone)]
pub struct AppState {
    pub vault_path: PathBuf,
    pub cache: Arc<RwLock<VaultCache>>,
    pub vault_revision: Arc<AtomicU64>,
    pub vault_events: broadcast::Sender<u64>,
    pub embedder: Arc<dyn Embedder>,
    /// Serializes vault file mutations against git sync tree operations.
    pub vault_write_lock: Arc<tokio::sync::Mutex<()>>,
    /// Present only when git sync is enabled.
    pub git_sync: Option<crate::git::GitSyncHandle>,
}
```

Add a helper method to enqueue writes (sync, non-blocking):

```rust
impl AppState {
    /// Record a vault write for git sync. No-op when sync is disabled.
    pub fn record_vault_write(&self, record: crate::git::WriteRecord) {
        if let Some(handle) = &self.git_sync {
            handle.record(record);
        }
    }
}
```

- [ ] **Step 2: Update all test constructors of `AppState`**

In `src/app_state.rs` (`state_with_vault`), `src/main.rs` (`app_for_tests_with_state`, `resolve_batch_marks_archived_notes`), and `src/mcp/routes.rs` (`test_state`), add to each `AppState { .. }`:

```rust
            vault_write_lock: Arc::new(tokio::sync::Mutex::new(())),
            git_sync: None,
```

Run: `cargo build --tests` and fix any remaining constructor sites the compiler flags.

- [ ] **Step 3: Wire startup in `main.rs`**

In `src/main.rs`, after the `let state = AppState { .. };` block (currently lines 110-116) and before `spawn_vault_watcher`, insert git-sync setup. First add the import near the other `hatchdoor::` imports:

```rust
use hatchdoor::git::{self, GitConfig};
```

Then replace the construction of `state` so the lock exists first, and add git wiring:

```rust
    let (vault_events, _) = tokio::sync::broadcast::channel(64);
    let vault_write_lock = Arc::new(tokio::sync::Mutex::new(()));

    let git_sync = match GitConfig::from_env(config.vault_path.clone()) {
        Ok(None) => None,
        Ok(Some(git_config)) => {
            if let Err(e) = git::validate_repo(&git_config) {
                error!("Git sync configuration invalid: {e}");
                std::process::exit(1);
            }
            let handle = git::spawn_sync_task(
                git_config.clone(),
                vault_write_lock.clone(),
                |cfg, paths, msg| git::sync(cfg, paths, msg).map(|report| report.outcome),
            );
            // Flush commits stranded by an earlier outage.
            match git::has_unpushed(&git_config) {
                Ok(true) => handle.record(hatchdoor::git::WriteRecord {
                    op: "startup".to_string(),
                    target: "flush unpushed".to_string(),
                    affected_paths: vec![],
                    summary: None,
                }),
                Ok(false) => {}
                Err(e) => error!("Git sync startup check failed: {e}"),
            }
            info!("Git sync enabled");
            Some(handle)
        }
        Err(e) => {
            error!("Git sync configuration error: {e}");
            std::process::exit(1);
        }
    };

    let state = AppState {
        vault_path: config.vault_path.clone(),
        cache: Arc::new(RwLock::new(cache)),
        vault_revision: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        vault_events,
        embedder,
        vault_write_lock,
        git_sync,
    };
```

> Note: a `"startup"` record with empty `affected_paths` produces no new commit (nothing staged), but still triggers `integrate_remote` + `push`, which flushes pre-existing unpushed commits. This reuses the normal sync path.

- [ ] **Step 4: Verify build and existing tests**

Run: `cargo test`
Expected: PASS (no behavior change when git sync is disabled, which is the default in all tests).

- [ ] **Step 5: Commit**

```bash
git add src/app_state.rs src/main.rs src/mcp/routes.rs
git commit -m "feat(git): wire sync handle, vault lock, and startup validation into AppState"
```

---

## Task 8: Acquire the vault lock + emit WriteRecords in MCP write tools

**Files:**
- Modify: `src/mcp/tools.rs` (write-tool dispatch, each write tool fn, Args structs, schemas)

- [ ] **Step 1: Acquire the lock centrally for write tools**

In `src/mcp/tools.rs`, `handle_tools_call`, wrap the write-tool arms so the lock is held for the duration of the write. Replace the write-tool match arms (lines 54-75) with a guarded block:

```rust
        // Write tools acquire the vault-mutation lock so a concurrent git sync
        // (merge / reset) never races a filesystem write.
        "create_note" | "update_note" | "append_to_note" | "edit_note" | "replace_section"
        | "rename_note" | "move_note" | "move_rename_note" | "delete_note" | "import_attachment"
        | "move_attachment" | "rename_attachment" | "delete_attachment"
            if config.write_enabled =>
        {
            let _guard = state.vault_write_lock.clone().lock_owned().await;
            match name {
                "create_note" => create_note_tool(state, arguments).await,
                "update_note" => update_note_tool(state, arguments).await,
                "append_to_note" => append_to_note_tool(state, arguments).await,
                "edit_note" => edit_note_tool(state, arguments).await,
                "replace_section" => replace_section_tool(state, arguments).await,
                "rename_note" => rename_note_tool(state, arguments).await,
                "move_note" => move_note_tool(state, arguments).await,
                "move_rename_note" => move_rename_note_tool(state, arguments).await,
                "delete_note" => delete_note_tool(state, arguments).await,
                "import_attachment" => import_attachment_tool(state, arguments, config).await,
                "move_attachment" => move_attachment_tool(state, arguments).await,
                "rename_attachment" => rename_attachment_tool(state, arguments).await,
                "delete_attachment" => delete_attachment_tool(state, arguments).await,
                _ => unreachable!(),
            }
        }
        "list_note_attachments" if config.write_enabled => {
            list_note_attachments_tool(state, arguments).await
        }
```

Keep the existing `get_attachment_import_config` arms and the disabled-write fallback arm (lines 41-53 and 76-91) as-is. `lock_owned()` avoids borrow-vs-move conflicts since `state` is consumed by the tool fns; clone the `Arc` first as shown.

> Implementation note: `state` is moved into each `*_tool` call, so acquire the guard from a clone of the `Arc`: `state.vault_write_lock.clone().lock_owned().await`. The guard lives to the end of the arm.

- [ ] **Step 2: Add an optional `commit_summary` to each write Args struct**

In `src/mcp/tools.rs`, add `#[serde(default)] commit_summary: Option<String>` as the last field of each write Args struct: `CreateNoteArgs`, `UpdateNoteArgs`, `AppendNoteArgs`, `EditNoteArgs`, `ReplaceSectionArgs`, `RenameNoteArgs`, `MoveNoteArgs`, `MoveRenameNoteArgs`, `DeleteNoteArgs`, `ImportAttachmentArgs`, `MoveAttachmentArgs`, `RenameAttachmentArgs`, `DeleteAttachmentArgs`. Example for `CreateNoteArgs`:

```rust
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateNoteArgs {
    relative_path: String,
    content: String,
    #[serde(default)]
    overwrite: Option<bool>,
    #[serde(default)]
    commit_summary: Option<String>,
}
```

Apply the same `#[serde(default)] commit_summary: Option<String>` line to the other twelve structs.

- [ ] **Step 3: Add a `record_write` helper and call it from each write tool**

Add this helper near `write_success` in `src/mcp/tools.rs`:

```rust
/// Build a WriteRecord from an outcome and enqueue it for git sync (no-op when disabled).
fn record_note_write(
    state: &AppState,
    op: &str,
    outcome: &WriteOutcome,
    commit_summary: Option<String>,
) {
    let target = outcome
        .relative_path
        .clone()
        .or_else(|| outcome.slug.clone())
        .unwrap_or_else(|| "note".to_string());
    state.record_vault_write(crate::git::WriteRecord {
        op: op.to_string(),
        target,
        affected_paths: outcome.affected_paths.clone(),
        summary: commit_summary,
    });
}

fn record_attachment_write(
    state: &AppState,
    op: &str,
    outcome: &AttachmentOutcome,
    commit_summary: Option<String>,
) {
    state.record_vault_write(crate::git::WriteRecord {
        op: op.to_string(),
        target: outcome.attachment.relative_path.clone(),
        affected_paths: outcome.affected_paths.clone(),
        summary: commit_summary,
    });
}
```

Then in each write tool fn, after `refresh_after_write(&state).await?;` and before `Ok(write_success(outcome))`, insert the record call. For `create_note_tool`:

```rust
    refresh_after_write(&state).await?;
    record_note_write(&state, "create", &outcome, args.commit_summary);
    Ok(write_success(outcome))
```

Apply analogously with the matching `op` string:
- `update_note_tool` → `"update"`
- `append_to_note_tool` → `"append"`
- `edit_note_tool` → `"edit"`
- `replace_section_tool` → `"replace_section"`
- `rename_note_tool` → `"rename"`
- `move_note_tool` → `"move"`
- `move_rename_note_tool` → `"move_rename"`
- `delete_note_tool` → `"delete"`

For attachment tools, use `record_attachment_write` after `refresh_after_write`:
- `import_attachment_tool` → `"import_attachment"` (note: this fn has no `refresh_after_write`; insert the record call right before `Ok(attachment_success(outcome))`)
- `move_attachment_tool` → `"move_attachment"`
- `rename_attachment_tool` → `"rename_attachment"`
- `delete_attachment_tool` → `"delete_attachment"`

> For tools whose `args` was partially moved (e.g. `args.content`), read `args.commit_summary` before those moves, or bind `let commit_summary = args.commit_summary.take();` after deserialization (make `args` `mut`). Simplest: capture `let commit_summary = args.commit_summary.clone();` immediately after deserialization in each fn.

- [ ] **Step 4: Advertise `commit_summary` in the write tool schemas**

In `write_tools_list()`, add to each write tool's `inputSchema.properties` (not to `required`):

```rust
                    "commit_summary": {"type": "string", "description": "Optional one-line summary of this change for the git commit body."}
```

Add it to all thirteen write tool schemas (create_note, update_note, append_to_note, edit_note, replace_section, rename_note, move_note, move_rename_note, delete_note, import_attachment, move_attachment, rename_attachment, delete_attachment). `list_note_attachments` is read-only — do not add it there.

- [ ] **Step 5: Verify the existing MCP tests still pass**

Run: `cargo test --lib mcp`
Expected: PASS. Existing write-tool tests run with `git_sync: None`, so `record_vault_write` is a no-op; the central lock has no contention. The `deny_unknown_fields` structs now also accept `commit_summary`.

- [ ] **Step 6: Add a test that a write enqueues a record**

Add to the test module in `src/mcp/routes.rs` a helper to build a state with a capturing git handle is heavy; instead assert at the unit level. Add this test to `src/mcp/tools.rs` (new `#[cfg(test)]` module if none exists):

```rust
#[cfg(test)]
mod record_tests {
    use super::*;

    #[test]
    fn record_note_write_prefers_relative_path_target() {
        // Build an outcome and ensure target derivation is correct without a live handle.
        let outcome = WriteOutcome {
            slug: Some("new".to_string()),
            relative_path: Some("Projects/New".to_string()),
            content_hash: Some("h".to_string()),
            rewritten_notes: 0,
            moved_assets: 0,
            trashed_path: None,
            affected_paths: vec![std::path::PathBuf::from("/v/Projects/New.md")],
        };
        let record = crate::git::WriteRecord {
            op: "create".to_string(),
            target: outcome
                .relative_path
                .clone()
                .or_else(|| outcome.slug.clone())
                .unwrap_or_default(),
            affected_paths: outcome.affected_paths.clone(),
            summary: Some("added".to_string()),
        };
        assert_eq!(record.target, "Projects/New");
        assert_eq!(record.affected_paths.len(), 1);
    }
}
```

Run: `cargo test --lib mcp::tools::record_tests`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/mcp/tools.rs
git commit -m "feat(mcp): emit git WriteRecords and hold vault lock on write tools"
```

---

## Task 9: `get_git_sync_status` MCP tool + write-response warning

**Files:**
- Modify: `src/mcp/tools.rs` (dispatch, tools_list, new tool fn, warning helper)

- [ ] **Step 1: Add the read-only status tool to dispatch and the list**

In `handle_tools_call`, add an arm (next to `refresh_index`):

```rust
        "get_git_sync_status" => get_git_sync_status_tool(state).await,
```

In `tools_list`, push a tool definition into the always-available `tools` vec (after `get_attachment_import_config`):

```rust
        json!({
            "name": "get_git_sync_status",
            "description": "Report the status of automatic git sync: whether it is enabled, the last sync time, whether the last attempt succeeded, the last error (if any), and how many writes are pending. Use to check whether your changes have been committed and pushed.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            },
            "annotations": read_only_tool_annotations()
        }),
```

- [ ] **Step 2: Implement the tool fn**

Add to `src/mcp/tools.rs`:

```rust
async fn get_git_sync_status_tool(state: AppState) -> Result<Value, JsonRpcFailure> {
    let status = match &state.git_sync {
        Some(handle) => {
            let guard = handle.status();
            let snapshot = guard.read().await;
            serde_json::to_value(&*snapshot)
                .map_err(|e| JsonRpcFailure::internal(format!("serialize git status: {e}")))?
        }
        None => json!({
            "enabled": false,
            "last_sync_at": null,
            "last_ok": false,
            "last_error": null,
            "pending": 0
        }),
    };
    Ok(tool_success(status))
}
```

- [ ] **Step 3: Attach a `git_sync_warning` to write responses when the last sync failed**

Change `write_success` and `attachment_success` to optionally carry a warning. Add a helper and update the call sites to pass the current status. Add:

```rust
/// Returns the last sync error message when the most recent sync failed.
async fn git_sync_warning(state: &AppState) -> Option<String> {
    let handle = state.git_sync.as_ref()?;
    let guard = handle.status();
    let snapshot = guard.read().await;
    if snapshot.last_ok {
        None
    } else {
        snapshot
            .last_error
            .clone()
            .map(|e| format!("git sync has not succeeded since: {e}"))
    }
}
```

Update `write_success` to accept an optional warning:

```rust
fn write_success(outcome: WriteOutcome, git_sync_warning: Option<String>) -> Value {
    tool_success(json!({
        "ok": true,
        "slug": outcome.slug,
        "relative_path": outcome.relative_path,
        "content_hash": outcome.content_hash,
        "rewritten_notes": outcome.rewritten_notes,
        "moved_assets": outcome.moved_assets,
        "trashed_path": outcome.trashed_path,
        "git_sync_warning": git_sync_warning,
    }))
}
```

and `attachment_success` similarly (add `"git_sync_warning": git_sync_warning,`).

At each write tool call site, replace `Ok(write_success(outcome))` with:

```rust
    record_note_write(&state, "create", &outcome, commit_summary);
    let warning = git_sync_warning(&state).await;
    Ok(write_success(outcome, warning))
```

(and the analogous change for attachment tools using `attachment_success(outcome, warning)`).

> Because the most-recent failure is what matters, reading status *after* enqueuing the new record is correct: it reflects the last *completed* sync, not the one this write will trigger.

- [ ] **Step 4: Update the `tools_list` determinism test**

`src/mcp/routes.rs::tools_list_is_deterministic_and_read_only` asserts the exact read-only tool name list (lines 348-359). Add `"get_git_sync_status"` to the expected vec, after `"get_attachment_import_config"`:

```rust
        assert_eq!(
            names,
            vec![
                "search_notes",
                "get_note",
                "get_note_links",
                "resolve_wikilink",
                "get_tree",
                "refresh_index",
                "get_attachment_import_config",
                "get_git_sync_status"
            ]
        );
```

The loop `for tool in tools.iter().take(5)` is unaffected. The `attachment_config = tools.last()` assertion now points at `get_git_sync_status`; change it to find by name instead:

```rust
        let attachment_config = tools
            .iter()
            .find(|tool| tool["name"] == "get_attachment_import_config")
            .expect("attachment config tool");
```

- [ ] **Step 5: Run MCP tests**

Run: `cargo test --lib mcp`
Expected: PASS — status tool present, write responses carry a (null when healthy) `git_sync_warning`, tool list updated.

- [ ] **Step 6: Commit**

```bash
git add src/mcp/tools.rs src/mcp/routes.rs
git commit -m "feat(mcp): add get_git_sync_status tool and write-response sync warning"
```

---

## Task 10: Documentation + `.env.example`

**Files:**
- Modify: `.env.example`
- Modify: `README.md`

- [ ] **Step 1: Document env vars in `.env.example`**

Append to `.env.example`:

```env
# Automatic git sync of the vault (off by default). When enabled, successful MCP
# write tools commit and push vault changes to the configured remote.
HATCHDOOR_GIT_SYNC_ENABLED=false
# Remote name (URL comes from the repo's existing remote config).
HATCHDOOR_GIT_REMOTE=origin
# Branch to commit and push. Must match the repo's current branch.
HATCHDOOR_GIT_BRANCH=main
# HTTPS auth. Token is required when sync is enabled; never logged.
HATCHDOOR_GIT_HTTPS_USERNAME=hatchdoor
HATCHDOOR_GIT_HTTPS_TOKEN=
# Quiet window (seconds) before a batch of writes is committed and pushed.
HATCHDOOR_GIT_DEBOUNCE_SECONDS=30
# Commit identity.
HATCHDOOR_GIT_AUTHOR_NAME=Hatchdoor
HATCHDOOR_GIT_AUTHOR_EMAIL=hatchdoor@localhost
```

- [ ] **Step 2: Add a README section**

Add a "Git sync" subsection to `README.md` after the MCP write configuration block. Describe: opt-in via `HATCHDOOR_GIT_SYNC_ENABLED`, requires the vault to be a git repo whose root is the vault and whose HEAD is the configured branch, HTTPS token auth, debounced commit+push (default 30s), pull-before-push with clean conflict abort, the `get_git_sync_status` tool, and that conflicts must be resolved by a human on the server. Keep it concise and consistent with the existing tone.

- [ ] **Step 3: Commit**

```bash
git add .env.example README.md
git commit -m "docs: document MCP git sync configuration"
```

---

## Task 11: Full build, format, and Docker verification

**Files:** none (verification only)

- [ ] **Step 1: Format and lint**

Run: `cargo fmt` then `cargo build`
Expected: clean build. (The repo pins Rust 1.96.0 via `rust-toolchain.toml`; `cargo fmt` matches the formatting in recent commits.)

- [ ] **Step 2: Full test suite**

Run: `cargo test`
Expected: PASS.

- [ ] **Step 3: Build the Docker image to confirm distroless runtime works**

Run: `docker build -t hatchdoor:git-sync-test .`
Expected: PASS. The vendored libgit2 + OpenSSL link into the binary in the builder stage.

> Contingency: if the container fails to start with a missing shared library (e.g. `libz.so.1`) when git sync is exercised, the distroless `cc` base lacks zlib. Resolve by enabling libgit2-sys's bundled zlib (it is bundled with `vendored-libgit2`), or as a fallback copy `libz.so.1` from the builder stage in the Dockerfile runtime stage. Verify with a smoke test (Step 4) before concluding.

- [ ] **Step 4: Smoke-test git sync end to end (manual)**

Using a scratch HTTPS remote you control and a local clone as the vault, set the `HATCHDOOR_GIT_*` env vars, enable MCP write mode, run the container, perform a `create_note` via MCP, wait past the debounce window, and confirm the commit appears on the remote and `get_git_sync_status` reports `last_ok: true`. Document the result.

- [ ] **Step 5: Commit any formatting changes**

```bash
git add -A
git commit -m "chore: rustfmt after git sync feature"
```

---

## Self-review checklist (completed by plan author)

- **Spec coverage:** pull→commit→push (Task 3), debounced background sync (Task 5), conflict abort keeping local commit (Task 3 Step 9/12-13), HTTPS token auth (Task 2/Task 3 credentials), agent-supplied optional commit summary + auto fallback (Task 4 message builder + Task 8), 30s debounce default (Task 2), git2 vendored/distroless (Task 1/Task 11), vault-mutation lock (Task 7/Task 8), precise per-path staging incl. deletions/renames/rewrites (Task 6 + Task 3 staging), startup validation incl. branch/HEAD (Task 3/Task 7), startup flush of unpushed commits (Task 7), secret redaction (status omits token; errors carry only libgit2 messages — Task 3), error surfacing via status tool + write warning (Task 9), docs (Task 10). All spec sections map to a task.
- **Placeholder scan:** no TBD/TODO; every code step shows complete code. The only prose-only step is README wording (Task 10 Step 2), which is documentation, not code.
- **Type consistency:** `WriteRecord`, `GitConfig`, `GitSyncStatus`, `GitSyncHandle`, `SyncOutcome`, `SyncReport`, `GitError`, `sync`, `validate_repo`, `has_unpushed`, `spawn_sync_task`, `record_vault_write`, `record_note_write`/`record_attachment_write`, `write_success`/`attachment_success` signatures are used consistently across tasks.
