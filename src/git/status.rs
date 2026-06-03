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
