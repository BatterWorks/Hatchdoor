/// Versioning mode of one Git operation. Since #185 deleted the instance-wide
/// lane, a per-Vault Git turn selects its operation from
/// `vault_registry::VaultGitMode`; nothing parses the retired
/// `HATCHDOOR_GIT_SYNC_ENABLED` into this any more (#644).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitMode {
    Local,
    Remote,
}

impl GitMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Remote => "remote",
        }
    }
}

/// Static configuration for one Git operation on one Vault directory.
///
/// Every field except `repository_path`, `vault_path`, `author_name`, and
/// `author_email` is a remnant of the instance-wide lane #185 deleted:
/// `run_local_history_git_turn` fills them with placeholders its callees
/// never read.
#[derive(Clone, PartialEq, Eq)]
pub struct GitConfig {
    /// The repository folder this Vault's history lives in: the Vault folder
    /// itself, or the checkout the Vault is a subfolder of. Local history
    /// opens exactly this repository and never searches upward from it, so
    /// a Vault that lost its own `.git` cannot commit into an enclosing
    /// repository instead (#678).
    pub repository_path: std::path::PathBuf,
    /// Absolute path to the Vault. Remote mode requires it to be the Git
    /// repository root; Local history may use an existing enclosing checkout.
    pub vault_path: std::path::PathBuf,
    /// Local commits only, or local commits plus safe remote synchronization.
    pub mode: GitMode,
    /// Remote name (e.g. "origin"). Only `validate_repo` still reads it.
    pub remote: String,
    /// Branch the legacy remote lane committed and pushed. Only
    /// `validate_repo` still reads it.
    pub branch: String,
    /// HTTPS auth username. No Git operation in this module reads it.
    pub username: String,
    /// HTTPS auth token. Never logged or surfaced. No Git operation in this
    /// module reads it.
    pub token: String,
    /// Quiet window the legacy debounced task waited before committing.
    /// Nothing reads it.
    pub debounce_seconds: u64,
    /// Commit author/committer name.
    pub author_name: String,
    /// Commit author/committer email.
    pub author_email: String,
}

impl std::fmt::Debug for GitConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitConfig")
            .field("repository_path", &self.repository_path)
            .field("vault_path", &self.vault_path)
            .field("mode", &self.mode)
            .field("remote", &self.remote)
            .field("branch", &self.branch)
            .field("username", &self.username)
            .field("token", &"***")
            .field("debounce_seconds", &self.debounce_seconds)
            .field("author_name", &self.author_name)
            .field("author_email", &self.author_email)
            .finish()
    }
}

/// Resolve the commit author identity for one Vault's Git operation: its own
/// configured identity if set (`vault_registry::VaultDefinition::commit_identity`),
/// else the instance-wide `HATCHDOOR_GIT_AUTHOR_NAME`/`HATCHDOOR_GIT_AUTHOR_EMAIL`
/// defaults (#120's decision to keep those server-wide with an optional
/// per-Vault override).
pub fn resolve_commit_identity(
    vault_identity: Option<&crate::vault_registry::VaultCommitIdentity>,
    default_name: &str,
    default_email: &str,
) -> (String, String) {
    match vault_identity {
        Some(identity) => (identity.name.clone(), identity.email.clone()),
        None => (default_name.to_string(), default_email.to_string()),
    }
}

/// The instance-wide default commit identity, read from one settings
/// snapshot. A Vault's own configured identity still overrides it (see
/// [`resolve_commit_identity`]).
pub(crate) fn author_defaults(
    snapshot: &crate::runtime_config::ConfigSnapshot,
) -> (String, String) {
    (
        non_empty_setting(snapshot, "HATCHDOOR_GIT_AUTHOR_NAME")
            .unwrap_or_else(|| "Hatchdoor".to_string()),
        non_empty_setting(snapshot, "HATCHDOOR_GIT_AUTHOR_EMAIL")
            .unwrap_or_else(|| "hatchdoor@localhost".to_string()),
    )
}

pub(crate) fn non_empty_setting(
    snapshot: &crate::runtime_config::ConfigSnapshot,
    key: &str,
) -> Option<String> {
    snapshot
        .required(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_commit_identity_prefers_the_vaults_own_identity_when_set() {
        let vault_identity = crate::vault_registry::VaultCommitIdentity {
            name: "Work Bot".to_string(),
            email: "work-bot@example.test".to_string(),
        };

        let (name, email) =
            resolve_commit_identity(Some(&vault_identity), "Hatchdoor", "hatchdoor@localhost");

        assert_eq!(name, "Work Bot");
        assert_eq!(email, "work-bot@example.test");
    }

    #[test]
    fn resolve_commit_identity_falls_back_to_the_instance_default_when_absent() {
        let (name, email) = resolve_commit_identity(None, "Hatchdoor", "hatchdoor@localhost");

        assert_eq!(name, "Hatchdoor");
        assert_eq!(email, "hatchdoor@localhost");
    }
}
