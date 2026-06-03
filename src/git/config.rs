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
        let username = non_empty_env("HATCHDOOR_GIT_HTTPS_USERNAME")
            .unwrap_or_else(|| "hatchdoor".to_string());
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
