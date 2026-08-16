use serde::Serialize;

/// Shared, observable state of the git-sync subsystem.
#[derive(Debug, Clone, Default, Serialize)]
pub struct GitSyncStatus {
    /// Kept for existing MCP consumers; `state` is the richer lifecycle.
    pub enabled: bool,
    /// `disabled`, `starting`, `running`, or `stopping`.
    pub state: String,
    /// `off`, `local`, or `remote`.
    pub mode: String,
    /// RFC3339 timestamp of the last completed sync attempt, if any.
    pub last_sync_at: Option<String>,
    /// True when the last attempt succeeded (pushed or no-op).
    pub last_ok: bool,
    /// Human-readable error from the last failed attempt (token redacted upstream).
    pub last_error: Option<String>,
    /// Machine-readable category of the last error, so clients can distinguish a
    /// merge conflict (local commit kept, needs human resolution) from a
    /// transient remote error (retried on the next batch). One of
    /// "validation" | "conflict" | "dirty_tree" | "manual_recovery" |
    /// "remote" | "other".
    pub last_error_kind: Option<String>,
    /// Write records waiting for the next debounced sync.
    pub pending: usize,
    /// Local commits on the branch not yet pushed to the remote. Non-zero after
    /// a conflict abort or an outage; zero after a successful push.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unpushed: Option<usize>,
}

impl GitSyncStatus {
    pub fn starting(mode: &str) -> Self {
        Self {
            enabled: true,
            state: "starting".to_string(),
            mode: mode.to_string(),
            ..Default::default()
        }
    }

    pub fn disabled() -> Self {
        Self {
            enabled: false,
            state: "disabled".to_string(),
            mode: "off".to_string(),
            ..Default::default()
        }
    }
}
