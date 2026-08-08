//! Safe first-acquisition and restart-reuse primitives for managed HTTPS Vaults.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::vault_registry::VaultId;

const RECEIPT_FILE: &str = ".hatchdoor-managed-checkout.json";

/// Write-only HTTPS credentials used only by libgit2 authentication callbacks.
#[derive(Clone, PartialEq, Eq)]
pub struct ManagedHttpsCredentials {
    pub username: String,
    pub token: String,
}

impl std::fmt::Debug for ManagedHttpsCredentials {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ManagedHttpsCredentials")
            .field("username", &"[REDACTED]")
            .field("token", &"[REDACTED]")
            .finish()
    }
}

/// The identity-bearing input required to acquire or reuse one managed checkout.
#[derive(Clone, PartialEq, Eq)]
pub struct ManagedCheckoutRequest {
    pub state_directory: PathBuf,
    pub vault_id: VaultId,
    pub repository_url: String,
    pub branch: Option<String>,
    pub vault_subdirectory: Option<PathBuf>,
    pub credentials: Option<ManagedHttpsCredentials>,
}

impl std::fmt::Debug for ManagedCheckoutRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ManagedCheckoutRequest")
            .field("state_directory", &self.state_directory)
            .field("vault_id", &self.vault_id)
            .field("repository_url", &"[REDACTED]")
            .field("branch", &self.branch)
            .field("vault_subdirectory", &self.vault_subdirectory)
            .field("credentials", &self.credentials)
            .finish()
    }
}

/// A held, per-Vault ownership boundary for the managed checkout lifetime.
pub struct ManagedCheckoutLease {
    vault_directory: PathBuf,
    _lock_file: File,
}

/// A validated managed repository and its contained Vault root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedCheckout {
    pub repository_path: PathBuf,
    pub vault_path: PathBuf,
    pub resolved_branch: String,
    pub reused: bool,
}

/// Safe, credential-free checkout acquisition and reuse failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManagedCheckoutError {
    StateDirectoryUnavailable,
    OwnershipUnavailable,
    UnsafeRepositoryUrl,
    DestinationInvalid,
    CloneFailed,
    /// The remote rejected the supplied (or absent) credentials. Distinct from
    /// `CloneFailed` so a caller can wait for a credential change or manual
    /// retry instead of retrying blindly on a schedule.
    AuthenticationFailed,
    ValidationFailed,
    AtomicInstallFailed,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckoutReceipt {
    repository_url: String,
    resolved_branch: String,
    vault_subdirectory: Option<PathBuf>,
}

impl std::fmt::Display for ManagedCheckoutError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::StateDirectoryUnavailable => "managed checkout state directory is unavailable",
            Self::OwnershipUnavailable => "managed checkout is already owned by another process",
            Self::UnsafeRepositoryUrl => "managed checkout repository URL is unsafe",
            Self::DestinationInvalid => "managed checkout destination is invalid",
            Self::CloneFailed => "managed checkout clone failed",
            Self::AuthenticationFailed => "managed checkout authentication failed",
            Self::ValidationFailed => "managed checkout validation failed",
            Self::AtomicInstallFailed => "managed checkout could not be installed atomically",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for ManagedCheckoutError {}

impl ManagedCheckoutLease {
    pub fn acquire(
        state_directory: PathBuf,
        vault_id: VaultId,
    ) -> Result<Self, ManagedCheckoutError> {
        let vault_directory = prepare_vault_directory(&state_directory, vault_id)?;
        let lock_path = vault_directory.join(".hatchdoor-checkout.lock");
        let lock_file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(lock_path)
            .map_err(|_| ManagedCheckoutError::OwnershipUnavailable)?;
        lock_exclusively(&lock_file)?;
        Ok(Self {
            vault_directory,
            _lock_file: lock_file,
        })
    }
}

pub fn acquire_or_reuse(
    lease: &ManagedCheckoutLease,
    request: &ManagedCheckoutRequest,
) -> Result<ManagedCheckout, ManagedCheckoutError> {
    if !is_safe_managed_repository_url(&request.repository_url) {
        return Err(ManagedCheckoutError::UnsafeRepositoryUrl);
    }
    let expected_vault_directory =
        prepare_vault_directory(&request.state_directory, request.vault_id)?;
    if expected_vault_directory != lease.vault_directory {
        return Err(ManagedCheckoutError::OwnershipUnavailable);
    }

    let destination = lease.vault_directory.join("repository");
    match fs::symlink_metadata(&destination) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(ManagedCheckoutError::DestinationInvalid)
        }
        Ok(_) => reuse_checkout(&destination, &lease.vault_directory, request),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if has_interrupted_acquisition(&lease.vault_directory)? {
                Err(ManagedCheckoutError::DestinationInvalid)
            } else {
                acquire_new_checkout(&destination, &lease.vault_directory, request)
            }
        }
        Err(_) => Err(ManagedCheckoutError::DestinationInvalid),
    }
}

fn acquire_new_checkout(
    destination: &Path,
    vault_directory: &Path,
    request: &ManagedCheckoutRequest,
) -> Result<ManagedCheckout, ManagedCheckoutError> {
    let temporary = create_temporary_sibling(vault_directory)?;
    clone_repository(request, &temporary)?;
    let checkout = validate_checkout(
        &temporary,
        vault_directory,
        request,
        request.branch.as_deref(),
        false,
    )?;
    atomic_install(&temporary, destination)?;
    write_receipt(vault_directory, request, &checkout.resolved_branch)?;
    validate_checkout(
        destination,
        vault_directory,
        request,
        Some(&checkout.resolved_branch),
        false,
    )
}

fn reuse_checkout(
    destination: &Path,
    vault_directory: &Path,
    request: &ManagedCheckoutRequest,
) -> Result<ManagedCheckout, ManagedCheckoutError> {
    let receipt = read_receipt(vault_directory)?;
    if receipt.repository_url != request.repository_url
        || receipt.vault_subdirectory != request.vault_subdirectory
        || request
            .branch
            .as_deref()
            .is_some_and(|branch| branch != receipt.resolved_branch)
    {
        return Err(ManagedCheckoutError::ValidationFailed);
    }
    validate_checkout(
        destination,
        vault_directory,
        request,
        Some(&receipt.resolved_branch),
        true,
    )
}

fn clone_repository(
    request: &ManagedCheckoutRequest,
    temporary: &Path,
) -> Result<(), ManagedCheckoutError> {
    let mut clone = git2::build::RepoBuilder::new();
    if let Some(branch) = &request.branch {
        clone.branch(branch);
    }
    let mut fetch_options = git2::FetchOptions::new();
    if let Some(credentials) = &request.credentials {
        let credentials = credentials.clone();
        let mut callbacks = git2::RemoteCallbacks::new();
        callbacks.credentials(move |_url, _username_from_url, _allowed| {
            git2::Cred::userpass_plaintext(&credentials.username, &credentials.token)
        });
        fetch_options.remote_callbacks(callbacks);
    }
    clone.fetch_options(fetch_options);
    clone
        .clone(&request.repository_url, temporary)
        .map_err(classify_remote_error)?;
    Ok(())
}

/// Distinguish a credential rejection from any other remote failure, so a
/// caller can wait for a credential change or manual retry rather than
/// backing off and retrying blindly.
fn classify_remote_error(error: git2::Error) -> ManagedCheckoutError {
    if error.code() == git2::ErrorCode::Auth {
        ManagedCheckoutError::AuthenticationFailed
    } else {
        ManagedCheckoutError::CloneFailed
    }
}

fn validate_checkout(
    repository_path: &Path,
    vault_directory: &Path,
    request: &ManagedCheckoutRequest,
    expected_branch: Option<&str>,
    reused: bool,
) -> Result<ManagedCheckout, ManagedCheckoutError> {
    let repository_path = repository_path
        .canonicalize()
        .map_err(|_| ManagedCheckoutError::ValidationFailed)?;
    if !repository_path.starts_with(vault_directory) {
        return Err(ManagedCheckoutError::DestinationInvalid);
    }
    let repository = git2::Repository::open(&repository_path)
        .map_err(|_| ManagedCheckoutError::ValidationFailed)?;
    let workdir = repository
        .workdir()
        .ok_or(ManagedCheckoutError::ValidationFailed)?
        .canonicalize()
        .map_err(|_| ManagedCheckoutError::ValidationFailed)?;
    if workdir != repository_path {
        return Err(ManagedCheckoutError::ValidationFailed);
    }
    validate_remotes(&repository, &request.repository_url)?;
    let head = repository
        .head()
        .map_err(|_| ManagedCheckoutError::ValidationFailed)?;
    if !head.is_branch() {
        return Err(ManagedCheckoutError::ValidationFailed);
    }
    let resolved_branch = head
        .shorthand()
        .map_err(|_| ManagedCheckoutError::ValidationFailed)?
        .to_string();
    if expected_branch.is_some_and(|branch| branch != resolved_branch) {
        return Err(ManagedCheckoutError::ValidationFailed);
    }
    let vault_path = contained_vault_path(&repository_path, request.vault_subdirectory.as_deref())?;
    Ok(ManagedCheckout {
        repository_path,
        vault_path,
        resolved_branch,
        reused,
    })
}

fn read_receipt(vault_directory: &Path) -> Result<CheckoutReceipt, ManagedCheckoutError> {
    let receipt_path = vault_directory.join(RECEIPT_FILE);
    let metadata =
        fs::symlink_metadata(&receipt_path).map_err(|_| ManagedCheckoutError::ValidationFailed)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(ManagedCheckoutError::ValidationFailed);
    }
    let encoded = fs::read(receipt_path).map_err(|_| ManagedCheckoutError::ValidationFailed)?;
    serde_json::from_slice(&encoded).map_err(|_| ManagedCheckoutError::ValidationFailed)
}

fn write_receipt(
    vault_directory: &Path,
    request: &ManagedCheckoutRequest,
    resolved_branch: &str,
) -> Result<(), ManagedCheckoutError> {
    let receipt = CheckoutReceipt {
        repository_url: request.repository_url.clone(),
        resolved_branch: resolved_branch.to_string(),
        vault_subdirectory: request.vault_subdirectory.clone(),
    };
    let encoded =
        serde_json::to_vec(&receipt).map_err(|_| ManagedCheckoutError::AtomicInstallFailed)?;
    let temporary = vault_directory.join(format!(
        "{RECEIPT_FILE}.acquiring-{}",
        VaultId::generate().map_err(|_| ManagedCheckoutError::AtomicInstallFailed)?
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|_| ManagedCheckoutError::AtomicInstallFailed)?;
        file.write_all(&encoded)
            .map_err(|_| ManagedCheckoutError::AtomicInstallFailed)?;
        file.sync_all()
            .map_err(|_| ManagedCheckoutError::AtomicInstallFailed)?;
        fs::rename(&temporary, vault_directory.join(RECEIPT_FILE))
            .map_err(|_| ManagedCheckoutError::AtomicInstallFailed)
    })();
    // A failed receipt write is deliberately left in place as acquisition
    // evidence; a later startup rejects it rather than deleting it.
    result
}

fn validate_remotes(
    repository: &git2::Repository,
    expected_origin: &str,
) -> Result<(), ManagedCheckoutError> {
    let origin = repository
        .find_remote("origin")
        .map_err(|_| ManagedCheckoutError::ValidationFailed)?;
    if origin
        .url()
        .map_err(|_| ManagedCheckoutError::ValidationFailed)?
        != expected_origin
    {
        return Err(ManagedCheckoutError::ValidationFailed);
    }
    for name in repository
        .remotes()
        .map_err(|_| ManagedCheckoutError::ValidationFailed)?
        .iter()
        .flatten()
        .flatten()
    {
        let remote = repository
            .find_remote(name)
            .map_err(|_| ManagedCheckoutError::ValidationFailed)?;
        let remote_url = remote
            .url()
            .map_err(|_| ManagedCheckoutError::ValidationFailed)?;
        let push_url_is_unsafe = match remote.pushurl() {
            Ok(Some(url)) => !is_credential_free_remote_url(url),
            Ok(None) => false,
            Err(error) if error.code() == git2::ErrorCode::NotFound => false,
            Err(_) => return Err(ManagedCheckoutError::ValidationFailed),
        };
        if !is_credential_free_remote_url(remote_url) || push_url_is_unsafe {
            return Err(ManagedCheckoutError::ValidationFailed);
        }
    }
    Ok(())
}

fn contained_vault_path(
    repository_path: &Path,
    vault_subdirectory: Option<&Path>,
) -> Result<PathBuf, ManagedCheckoutError> {
    let vault_path = vault_subdirectory
        .map(|subdirectory| repository_path.join(subdirectory))
        .unwrap_or_else(|| repository_path.to_path_buf())
        .canonicalize()
        .map_err(|_| ManagedCheckoutError::ValidationFailed)?;
    if !vault_path.starts_with(repository_path)
        || !fs::metadata(&vault_path)
            .map_err(|_| ManagedCheckoutError::ValidationFailed)?
            .is_dir()
    {
        return Err(ManagedCheckoutError::ValidationFailed);
    }
    Ok(vault_path)
}

fn prepare_vault_directory(
    state_directory: &Path,
    vault_id: VaultId,
) -> Result<PathBuf, ManagedCheckoutError> {
    let state_directory = state_directory
        .canonicalize()
        .map_err(|_| ManagedCheckoutError::StateDirectoryUnavailable)?;
    let vaults_directory =
        ensure_directory_inside(&state_directory, &state_directory.join("vaults"))?;
    let vault_directory = vaults_directory.join(vault_id.to_string());
    ensure_directory_inside(&vaults_directory, &vault_directory)
}

fn ensure_directory_inside(
    parent: &Path,
    directory: &Path,
) -> Result<PathBuf, ManagedCheckoutError> {
    match fs::create_dir(directory) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(ManagedCheckoutError::StateDirectoryUnavailable),
    }
    let directory = directory
        .canonicalize()
        .map_err(|_| ManagedCheckoutError::StateDirectoryUnavailable)?;
    if directory.starts_with(parent) {
        Ok(directory)
    } else {
        Err(ManagedCheckoutError::DestinationInvalid)
    }
}

fn create_temporary_sibling(vault_directory: &Path) -> Result<PathBuf, ManagedCheckoutError> {
    for _ in 0..16 {
        let temporary = vault_directory.join(format!(
            "repository.acquiring-{}",
            VaultId::generate().map_err(|_| ManagedCheckoutError::OwnershipUnavailable)?
        ));
        match fs::create_dir(&temporary) {
            Ok(()) => return Ok(temporary),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(ManagedCheckoutError::OwnershipUnavailable),
        }
    }
    Err(ManagedCheckoutError::OwnershipUnavailable)
}

fn has_interrupted_acquisition(vault_directory: &Path) -> Result<bool, ManagedCheckoutError> {
    let entries =
        fs::read_dir(vault_directory).map_err(|_| ManagedCheckoutError::DestinationInvalid)?;
    Ok(entries.flatten().any(|entry| {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        name.starts_with("repository.acquiring-")
            || name.starts_with(&format!("{RECEIPT_FILE}.acquiring-"))
    }))
}

#[cfg(target_os = "linux")]
fn atomic_install(temporary: &Path, destination: &Path) -> Result<(), ManagedCheckoutError> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let temporary = CString::new(temporary.as_os_str().as_bytes())
        .map_err(|_| ManagedCheckoutError::AtomicInstallFailed)?;
    let destination = CString::new(destination.as_os_str().as_bytes())
        .map_err(|_| ManagedCheckoutError::AtomicInstallFailed)?;
    // SAFETY: both C strings are live and null-terminated for this call only.
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            temporary.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(ManagedCheckoutError::AtomicInstallFailed)
    }
}

#[cfg(not(target_os = "linux"))]
fn atomic_install(_temporary: &Path, _destination: &Path) -> Result<(), ManagedCheckoutError> {
    Err(ManagedCheckoutError::AtomicInstallFailed)
}

fn is_safe_managed_repository_url(url: &str) -> bool {
    if crate::vault_registry::is_safe_https_repository_url(url) {
        return true;
    }
    // Local remotes are test fixtures only. Production configuration reaches
    // this boundary through the same strict registry validator.
    #[cfg(test)]
    return url.starts_with('/') && is_credential_free_remote_url(url);
    #[cfg(not(test))]
    false
}

fn is_credential_free_remote_url(url: &str) -> bool {
    if url.is_empty() || url.contains(['?', '#']) {
        return false;
    }
    let Some((_, authority_and_path)) = url.split_once("://") else {
        return true;
    };
    !authority_and_path
        .split_once('/')
        .map_or(authority_and_path, |(authority, _)| authority)
        .contains('@')
}

#[cfg(unix)]
fn lock_exclusively(lock_file: &File) -> Result<(), ManagedCheckoutError> {
    // SAFETY: flock only observes the open file descriptor and does not retain it.
    let result = unsafe {
        libc::flock(
            std::os::fd::AsRawFd::as_raw_fd(lock_file),
            libc::LOCK_EX | libc::LOCK_NB,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(ManagedCheckoutError::OwnershipUnavailable)
    }
}

#[cfg(not(unix))]
fn lock_exclusively(_lock_file: &File) -> Result<(), ManagedCheckoutError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use git2::{Repository, Signature};
    use tempfile::tempdir;

    use super::*;

    fn remote_with_default_branch(root: &std::path::Path, branch: &str) -> PathBuf {
        let source = root.join("source");
        let repository = Repository::init(&source).expect("initialize source repository");
        fs::write(source.join("note.md"), "# note").expect("write note");
        let mut index = repository.index().expect("index");
        index
            .add_path(std::path::Path::new("note.md"))
            .expect("stage note");
        index.write().expect("write index");
        let tree_id = index.write_tree().expect("write tree");
        let tree = repository.find_tree(tree_id).expect("tree");
        let signature = Signature::now("Test", "test@example.test").expect("signature");
        let commit_id = repository
            .commit(Some("HEAD"), &signature, &signature, "initial", &tree, &[])
            .expect("commit");
        let commit = repository.find_commit(commit_id).expect("initial commit");
        repository
            .branch(branch, &commit, true)
            .expect("create default branch");
        repository
            .set_head(&format!("refs/heads/{branch}"))
            .expect("set branch");

        let bare = root.join("remote.git");
        let bare_repository = Repository::init_bare(&bare).expect("initialize bare remote");
        let mut remote = repository
            .remote("origin", bare.to_str().expect("remote path"))
            .expect("configure remote");
        remote
            .push(&[&format!("refs/heads/{branch}:refs/heads/{branch}")], None)
            .expect("push branch");
        bare_repository
            .set_head(&format!("refs/heads/{branch}"))
            .expect("set remote default branch");
        bare
    }

    fn request(
        state_directory: PathBuf,
        vault_id: VaultId,
        remote: &Path,
    ) -> ManagedCheckoutRequest {
        ManagedCheckoutRequest {
            state_directory,
            vault_id,
            repository_url: remote.to_string_lossy().into_owned(),
            branch: None,
            vault_subdirectory: None,
            credentials: None,
        }
    }

    #[test]
    fn first_public_clone_installs_atomically_and_resolves_the_remote_default_branch() {
        let root = tempdir().expect("temporary state");
        let remote = remote_with_default_branch(root.path(), "trunk");
        let vault_id = VaultId::generate().expect("Vault ID");
        let state_directory = root.path().join("state");
        fs::create_dir(&state_directory).expect("state directory");
        let request = request(state_directory.clone(), vault_id, &remote);
        let lease = ManagedCheckoutLease::acquire(state_directory, vault_id).expect("lease");

        let checkout = acquire_or_reuse(&lease, &request).expect("public clone");

        assert_eq!(checkout.resolved_branch, "trunk");
        assert!(!checkout.reused);
        assert_eq!(checkout.repository_path, checkout.vault_path);
        assert!(checkout.repository_path.join("note.md").is_file());
        assert!(checkout.repository_path.join(".git").is_dir());
    }

    #[test]
    fn reuse_keeps_the_resolved_branch_when_the_remote_default_changes() {
        let root = tempdir().expect("temporary state");
        let remote = remote_with_default_branch(root.path(), "trunk");
        let vault_id = VaultId::generate().expect("Vault ID");
        let state_directory = root.path().join("state");
        fs::create_dir(&state_directory).expect("state directory");
        let request = request(state_directory.clone(), vault_id, &remote);
        let lease =
            ManagedCheckoutLease::acquire(state_directory.clone(), vault_id).expect("lease");
        let first = acquire_or_reuse(&lease, &request).expect("first checkout");
        drop(lease);

        Repository::open_bare(&remote)
            .expect("open bare remote")
            .set_head("refs/heads/master")
            .expect("change remote default");
        let lease = ManagedCheckoutLease::acquire(state_directory, vault_id).expect("reuse lease");
        let reused = acquire_or_reuse(&lease, &request).expect("reuse checkout");

        assert_eq!(first.resolved_branch, "trunk");
        assert_eq!(reused.resolved_branch, "trunk");
        assert!(reused.reused);
    }

    #[test]
    fn unknown_destination_is_preserved_and_rejected_without_clone_or_overwrite() {
        let root = tempdir().expect("temporary state");
        let remote = remote_with_default_branch(root.path(), "trunk");
        let vault_id = VaultId::generate().expect("Vault ID");
        let state_directory = root.path().join("state");
        fs::create_dir(&state_directory).expect("state directory");
        let destination = state_directory
            .join("vaults")
            .join(vault_id.to_string())
            .join("repository");
        fs::create_dir_all(&destination).expect("unknown destination");
        fs::write(destination.join("evidence.txt"), "do not touch").expect("evidence");
        let request = request(state_directory.clone(), vault_id, &remote);
        let lease = ManagedCheckoutLease::acquire(state_directory, vault_id).expect("lease");

        let error = acquire_or_reuse(&lease, &request).expect_err("unknown directory reused");

        assert_eq!(error, ManagedCheckoutError::ValidationFailed);
        assert_eq!(
            fs::read_to_string(destination.join("evidence.txt")).unwrap(),
            "do not touch"
        );
    }

    #[test]
    fn containment_escape_in_an_existing_checkout_is_rejected_without_touching_it() {
        let root = tempdir().expect("temporary state");
        let remote = remote_with_default_branch(root.path(), "trunk");
        let vault_id = VaultId::generate().expect("Vault ID");
        let state_directory = root.path().join("state");
        fs::create_dir(&state_directory).expect("state directory");
        let request = request(state_directory.clone(), vault_id, &remote);
        let lease = ManagedCheckoutLease::acquire(state_directory, vault_id).expect("lease");
        let checkout = acquire_or_reuse(&lease, &request).expect("checkout");
        let outside = checkout.repository_path.parent().unwrap().join("outside");
        fs::create_dir(&outside).expect("outside directory");
        #[cfg(unix)]
        std::os::unix::fs::symlink("../outside", checkout.repository_path.join("notes"))
            .expect("escaping symlink");
        let escaped_request = ManagedCheckoutRequest {
            vault_subdirectory: Some(PathBuf::from("notes")),
            ..request
        };

        let error = acquire_or_reuse(&lease, &escaped_request).expect_err("escaping path reused");

        assert_eq!(error, ManagedCheckoutError::ValidationFailed);
        assert!(checkout.repository_path.join("notes").is_symlink());
    }

    #[test]
    fn failed_clone_leaves_only_an_application_owned_temporary_sibling() {
        let root = tempdir().expect("temporary state");
        let vault_id = VaultId::generate().expect("Vault ID");
        let state_directory = root.path().join("state");
        fs::create_dir(&state_directory).expect("state directory");
        let request = ManagedCheckoutRequest {
            state_directory: state_directory.clone(),
            vault_id,
            repository_url: root
                .path()
                .join("missing.git")
                .to_string_lossy()
                .into_owned(),
            branch: None,
            vault_subdirectory: None,
            credentials: None,
        };
        let lease = ManagedCheckoutLease::acquire(state_directory, vault_id).expect("lease");

        let error = acquire_or_reuse(&lease, &request).expect_err("missing remote cloned");

        assert_eq!(error, ManagedCheckoutError::CloneFailed);
        let entries = fs::read_dir(&lease.vault_directory).expect("Vault state entries");
        assert!(entries.flatten().any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("repository.acquiring-")
        }));
        assert!(!lease.vault_directory.join("repository").exists());

        let second = acquire_or_reuse(&lease, &request).expect_err("interrupted clone retried");

        assert_eq!(second, ManagedCheckoutError::DestinationInvalid);
        assert_eq!(
            fs::read_dir(&lease.vault_directory)
                .expect("Vault state entries")
                .flatten()
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with("repository.acquiring-")
                })
                .count(),
            1
        );
    }

    #[test]
    fn atomic_install_never_replaces_a_destination_created_after_clone() {
        let root = tempdir().expect("temporary directory");
        let temporary = root.path().join("repository.acquiring");
        let destination = root.path().join("repository");
        fs::create_dir(&temporary).expect("temporary checkout");
        fs::write(temporary.join("candidate"), "new").expect("candidate evidence");
        fs::create_dir(&destination).expect("competing checkout");
        fs::write(destination.join("evidence"), "existing").expect("existing evidence");

        let error = atomic_install(&temporary, &destination).expect_err("destination replaced");

        assert_eq!(error, ManagedCheckoutError::AtomicInstallFailed);
        assert_eq!(
            fs::read_to_string(destination.join("evidence")).unwrap(),
            "existing"
        );
        assert_eq!(
            fs::read_to_string(temporary.join("candidate")).unwrap(),
            "new"
        );
    }

    #[test]
    fn official_destination_symlink_is_preserved_and_rejected() {
        let root = tempdir().expect("temporary state");
        let remote = remote_with_default_branch(root.path(), "trunk");
        let vault_id = VaultId::generate().expect("Vault ID");
        let state_directory = root.path().join("state");
        fs::create_dir(&state_directory).expect("state directory");
        let request = request(state_directory.clone(), vault_id, &remote);
        let lease = ManagedCheckoutLease::acquire(state_directory, vault_id).expect("lease");
        let checkout = acquire_or_reuse(&lease, &request).expect("checkout");
        let moved = lease.vault_directory.join("other-contained-directory");
        fs::rename(&checkout.repository_path, &moved).expect("move official checkout");
        #[cfg(unix)]
        std::os::unix::fs::symlink("other-contained-directory", &checkout.repository_path)
            .expect("official symlink");

        let error = acquire_or_reuse(&lease, &request).expect_err("symlink adopted");

        assert_eq!(error, ManagedCheckoutError::DestinationInvalid);
        assert!(checkout.repository_path.is_symlink());
        assert!(moved.join("note.md").is_file());
    }

    #[test]
    fn remote_errors_are_classified_as_authentication_or_generic_clone_failure() {
        let auth_error = git2::Error::new(
            git2::ErrorCode::Auth,
            git2::ErrorClass::Http,
            "authentication required",
        );
        assert_eq!(
            classify_remote_error(auth_error),
            ManagedCheckoutError::AuthenticationFailed
        );

        let network_error = git2::Error::new(
            git2::ErrorCode::GenericError,
            git2::ErrorClass::Net,
            "could not resolve host",
        );
        assert_eq!(
            classify_remote_error(network_error),
            ManagedCheckoutError::CloneFailed
        );
    }

    #[test]
    fn production_managed_urls_require_credential_free_https() {
        assert!(crate::vault_registry::is_safe_https_repository_url(
            "https://example.test/owner/notes.git"
        ));
        for unsafe_url in [
            "http://example.test/owner/notes.git",
            "ssh://example.test/owner/notes.git",
            "https://user:token@example.test/owner/notes.git",
            "https://example.test/owner/notes.git?token=secret",
        ] {
            assert!(
                !crate::vault_registry::is_safe_https_repository_url(unsafe_url),
                "unsafe managed URL accepted: {unsafe_url}"
            );
        }
    }

    #[test]
    fn credentials_are_never_written_to_checkout_configuration_or_error_text() {
        let root = tempdir().expect("temporary state");
        let remote = remote_with_default_branch(root.path(), "trunk");
        let vault_id = VaultId::generate().expect("Vault ID");
        let state_directory = root.path().join("state");
        fs::create_dir(&state_directory).expect("state directory");
        let secret = "not-in-config-or-error";
        let request = ManagedCheckoutRequest {
            credentials: Some(ManagedHttpsCredentials {
                username: "credential-user".to_string(),
                token: secret.to_string(),
            }),
            ..request(state_directory.clone(), vault_id, &remote)
        };
        let lease = ManagedCheckoutLease::acquire(state_directory, vault_id).expect("lease");

        let checkout = acquire_or_reuse(&lease, &request).expect("clone with unused credentials");

        assert!(!format!("{request:?}").contains(secret));
        assert!(
            !fs::read_to_string(checkout.repository_path.join(".git/config"))
                .expect("git config")
                .contains(secret)
        );
    }
}
