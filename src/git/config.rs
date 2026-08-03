/// Runtime-selected versioning mode. `off` is represented by the absence of a
/// [`GitConfig`]; a running task is always either local or remote.
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

/// Static configuration for one running versioning task.
#[derive(Clone, PartialEq, Eq)]
pub struct GitConfig {
    /// Absolute path to the vault, which must be the git repository root.
    pub vault_path: std::path::PathBuf,
    /// Local commits only, or local commits plus safe remote synchronization.
    pub mode: GitMode,
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

impl std::fmt::Debug for GitConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitConfig")
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

impl GitConfig {
    /// Returns `Ok(None)` when versioning is off, `Ok(Some(_))` when the selected
    /// mode is fully configured, and `Err(_)` when a required value is missing.
    pub fn from_snapshot(
        vault_path: std::path::PathBuf,
        snapshot: &crate::runtime_config::ConfigSnapshot,
    ) -> Result<Option<Self>, String> {
        let mode = parse_mode(snapshot.required("HATCHDOOR_GIT_SYNC_ENABLED")?)?;
        let Some(mode) = mode else {
            return Ok(None);
        };

        let token = if mode == GitMode::Remote {
            non_empty_setting(snapshot, "HATCHDOOR_GIT_HTTPS_TOKEN")
                .ok_or("Remote versioning needs HATCHDOOR_GIT_HTTPS_TOKEN before it can start")?
        } else {
            String::new()
        };
        let remote = non_empty_setting(snapshot, "HATCHDOOR_GIT_REMOTE")
            .unwrap_or_else(|| "origin".to_string());
        let branch = non_empty_setting(snapshot, "HATCHDOOR_GIT_BRANCH")
            .unwrap_or_else(|| "main".to_string());
        let username = non_empty_setting(snapshot, "HATCHDOOR_GIT_HTTPS_USERNAME")
            .unwrap_or_else(|| "hatchdoor".to_string());
        let debounce_seconds = snapshot
            .required("HATCHDOOR_GIT_DEBOUNCE_SECONDS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(30);
        let author_name = non_empty_setting(snapshot, "HATCHDOOR_GIT_AUTHOR_NAME")
            .unwrap_or_else(|| "Hatchdoor".to_string());
        let author_email = non_empty_setting(snapshot, "HATCHDOOR_GIT_AUTHOR_EMAIL")
            .unwrap_or_else(|| "hatchdoor@localhost".to_string());

        Ok(Some(Self {
            vault_path,
            mode,
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

fn non_empty_setting(
    snapshot: &crate::runtime_config::ConfigSnapshot,
    key: &str,
) -> Option<String> {
    snapshot
        .required(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn parse_mode(value: &str) -> Result<Option<GitMode>, String> {
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "" | "off" | "false" | "0" | "no" => Ok(None),
        "local" => Ok(Some(GitMode::Local)),
        "remote" | "true" | "1" | "yes" | "on" => Ok(Some(GitMode::Remote)),
        _ => Err("Choose versioning mode off, local, or remote.".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn snapshot(
        values: impl IntoIterator<Item = (&'static str, &'static str)>,
    ) -> std::sync::Arc<crate::runtime_config::ConfigSnapshot> {
        let dir = tempfile::tempdir().expect("tempdir");
        crate::runtime_config::RuntimeConfig::load(
            dir.path().join("settings.json"),
            crate::runtime_config::Environment::from_values(
                values
                    .into_iter()
                    .map(|(key, value)| (key.to_string(), value.to_string())),
            ),
            crate::runtime_config::live_settings_defaults(),
        )
        .expect("runtime config")
        .snapshot()
    }

    #[test]
    fn disabled_when_flag_unset() {
        let snapshot = snapshot([]);
        let cfg = GitConfig::from_snapshot(PathBuf::from("/vault"), &snapshot).expect("ok");
        assert_eq!(cfg, None);
    }

    #[test]
    fn enabled_requires_token() {
        let snapshot = snapshot([("HATCHDOOR_GIT_SYNC_ENABLED", "true")]);
        let result = GitConfig::from_snapshot(PathBuf::from("/vault"), &snapshot);
        assert!(result.is_err());
    }

    #[test]
    fn applies_defaults_when_enabled_with_token() {
        let snapshot = snapshot([
            ("HATCHDOOR_GIT_SYNC_ENABLED", "1"),
            ("HATCHDOOR_GIT_HTTPS_TOKEN", "secret"),
        ]);
        let cfg = GitConfig::from_snapshot(PathBuf::from("/vault"), &snapshot)
            .expect("ok")
            .expect("enabled");
        assert_eq!(cfg.remote, "origin");
        assert_eq!(cfg.branch, "main");
        assert_eq!(cfg.username, "hatchdoor");
        assert_eq!(cfg.debounce_seconds, 30);
        assert_eq!(cfg.author_email, "hatchdoor@localhost");
        assert_eq!(cfg.token, "secret");
    }

    #[test]
    fn local_mode_needs_no_remote_credential_and_legacy_true_means_remote() {
        let local = snapshot([("HATCHDOOR_GIT_SYNC_ENABLED", "local")]);
        let local = GitConfig::from_snapshot(PathBuf::from("/vault"), &local)
            .expect("local mode parses")
            .expect("local mode enabled");
        assert_eq!(local.mode, GitMode::Local);
        assert!(local.token.is_empty());

        let legacy = snapshot([
            ("HATCHDOOR_GIT_SYNC_ENABLED", "true"),
            ("HATCHDOOR_GIT_HTTPS_TOKEN", "secret"),
        ]);
        let legacy = GitConfig::from_snapshot(PathBuf::from("/vault"), &legacy)
            .expect("legacy true parses")
            .expect("legacy true enabled");
        assert_eq!(legacy.mode, GitMode::Remote);
    }
}
