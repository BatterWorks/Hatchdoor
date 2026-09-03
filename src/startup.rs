use schemars::JsonSchema;
use serde::Serialize;

use crate::vault_runtime::{VaultPhase, VaultRuntime, VaultSource};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct IndexingProgressSnapshot {
    pub notes_completed: usize,
    pub notes_total: usize,
    pub chunks_completed: usize,
    pub chunks_total: usize,
    pub tokens_completed: usize,
    pub tokens_total: usize,
    pub elapsed_seconds: u64,
}

impl IndexingProgressSnapshot {
    fn percent(self) -> u8 {
        if self.tokens_total == 0 {
            return 0;
        }
        ((self.tokens_completed.saturating_mul(100) / self.tokens_total).min(100)) as u8
    }

    fn eta_seconds(self) -> Option<u64> {
        if self.tokens_completed == 0 || self.tokens_completed >= self.tokens_total {
            return None;
        }
        let remaining = self.tokens_total - self.tokens_completed;
        Some(self.elapsed_seconds.saturating_mul(remaining as u64) / self.tokens_completed as u64)
    }
}

/// The code [`StartupTracker::set_model_setup_failed`] publishes. It is what
/// tells a failed model setup apart from every other reason this tracker can
/// be `Unavailable`, which is the distinction
/// [`StartupTracker::model_setup_pending`] turns on.
const MODEL_SETUP_FAILED: &str = "model_setup_failed";

#[derive(Clone, Debug)]
pub struct StartupTracker(VaultRuntime);

impl StartupTracker {
    pub fn new(runtime: VaultRuntime) -> Self {
        Self(runtime)
    }

    pub fn terms_required() -> Self {
        let runtime = VaultRuntime::new(VaultSource::Local {
            vault_path: "./vault".into(),
        });
        runtime.set_terms_required();
        Self(runtime)
    }

    pub fn scanning() -> Self {
        let runtime = VaultRuntime::new(VaultSource::Local {
            vault_path: "./vault".into(),
        });
        runtime.set_scanning();
        Self(runtime)
    }

    pub fn ready() -> Self {
        Self(VaultRuntime::ready(VaultSource::Local {
            vault_path: "./vault".into(),
        }))
    }

    pub fn set_scanning(&self) {
        self.0.set_scanning();
    }

    pub fn set_terms_required(&self) {
        self.0.set_terms_required();
    }

    pub fn set_downloading(
        &self,
        model: &'static str,
        downloaded_bytes: Option<u64>,
        total_bytes: Option<u64>,
    ) {
        self.0.set_downloading(model, downloaded_bytes, total_bytes);
    }

    pub fn set_indexing(&self, progress: IndexingProgressSnapshot) {
        self.0.set_indexing(progress);
    }

    pub fn set_ready(&self) {
        self.0.set_ready();
    }

    pub fn set_failed(&self) {
        self.0
            .set_unavailable("vault_index_failed", "Indexing could not be completed.");
    }

    pub fn set_model_setup_failed(&self) {
        self.0.set_unavailable(
            MODEL_SETUP_FAILED,
            "The search model could not be downloaded or loaded. Check the Hatchdoor logs, then retry setup.",
        );
    }

    /// Whether every active Vault's Index turn has settled `Ready`, which is
    /// the condition `VaultWorkExecutor::publish_outcome` latches here through
    /// `collection_indexes_ready`. It falls back to false for the duration of
    /// each subsequent rebuild.
    ///
    /// Named for what it measures rather than for `Ready`, because the shorter
    /// `is_ready` invited a question it cannot answer: three callers read it as
    /// "has first-run setup finished", and so reported a routine post-write
    /// reindex as incomplete setup (#191). Ask
    /// [`Self::model_setup_pending`] for that.
    pub fn collection_indexes_ready(&self) -> bool {
        self.0.is_ready()
    }

    /// Whether first-run model setup is genuinely what stands between a caller
    /// and the Vault collection.
    ///
    /// Deliberately not the negation of [`Self::collection_indexes_ready`].
    /// This tracker's phase does double duty: it carries the first-run setup
    /// lifecycle, and it is also the channel `VaultWorkExecutor` reports live
    /// indexing progress on, for whichever Vault currently has an Index turn.
    /// So a routine post-write reindex leaves `Ready` for `Indexing` on an
    /// instance whose setup finished long ago, and reading that as a setup
    /// answer is what #191 was.
    ///
    /// Only a pending terms choice, a download in flight, and a failed setup
    /// are conditions the setup tools can act on. Validating, scanning and
    /// indexing are not: each Vault reports those itself, per Vault and
    /// accurately, through its own `VaultSearchStatus`.
    pub fn model_setup_pending(&self) -> bool {
        let snapshot = self.0.snapshot();
        match snapshot.phase {
            VaultPhase::TermsRequired | VaultPhase::Downloading => true,
            VaultPhase::Unavailable => snapshot
                .error
                .is_some_and(|error| error.code == MODEL_SETUP_FAILED),
            VaultPhase::Validating
            | VaultPhase::Scanning
            | VaultPhase::Indexing
            | VaultPhase::Ready => false,
        }
    }

    pub fn runtime(&self) -> &VaultRuntime {
        &self.0
    }

    pub fn status(&self) -> StartupStatusResponse {
        let snapshot = self.0.snapshot();
        match snapshot.phase {
            VaultPhase::TermsRequired => StartupStatusResponse::simple("terms_required", None),
            VaultPhase::Downloading => StartupStatusResponse {
                state: "downloading",
                model: snapshot.model,
                downloaded_bytes: snapshot.downloaded_bytes,
                total_bytes: snapshot.total_bytes,
                notes_completed: None,
                notes_total: None,
                chunks_completed: None,
                chunks_total: None,
                tokens_completed: None,
                tokens_total: None,
                percent: download_percent(snapshot.downloaded_bytes, snapshot.total_bytes),
                eta_seconds: None,
                message: None,
            },
            VaultPhase::Validating | VaultPhase::Scanning => {
                StartupStatusResponse::simple("scanning", None)
            }
            VaultPhase::Indexing => {
                let progress = snapshot.indexing.unwrap_or_default();
                StartupStatusResponse {
                    state: "indexing",
                    model: None,
                    downloaded_bytes: None,
                    total_bytes: None,
                    notes_completed: Some(progress.notes_completed),
                    notes_total: Some(progress.notes_total),
                    chunks_completed: Some(progress.chunks_completed),
                    chunks_total: Some(progress.chunks_total),
                    tokens_completed: Some(progress.tokens_completed),
                    tokens_total: Some(progress.tokens_total),
                    percent: Some(progress.percent()),
                    eta_seconds: progress.eta_seconds(),
                    message: None,
                }
            }
            VaultPhase::Ready => StartupStatusResponse::simple("ready", None),
            VaultPhase::Unavailable => {
                StartupStatusResponse::simple("failed", snapshot.error.map(|error| error.message))
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct StartupStatusResponse {
    pub state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub downloaded_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes_completed: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes_total: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunks_completed: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunks_total: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_completed: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_total: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percent: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eta_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl StartupStatusResponse {
    fn simple(state: &'static str, message: Option<String>) -> Self {
        Self {
            state,
            model: None,
            downloaded_bytes: None,
            total_bytes: None,
            notes_completed: None,
            notes_total: None,
            chunks_completed: None,
            chunks_total: None,
            tokens_completed: None,
            tokens_total: None,
            percent: None,
            eta_seconds: None,
            message,
        }
    }
}

fn download_percent(downloaded: Option<u64>, total: Option<u64>) -> Option<u8> {
    let (Some(downloaded), Some(total)) = (downloaded, total) else {
        return None;
    };
    (total > 0).then(|| ((downloaded.saturating_mul(100) / total).min(100)) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terms_required_is_not_ready_and_is_exposed_to_the_ui() {
        let tracker = StartupTracker::terms_required();
        assert!(!tracker.collection_indexes_ready());
        let status = tracker.status();
        assert_eq!(status.state, "terms_required");
        assert!(status.percent.is_none());
    }

    #[test]
    fn download_status_carries_model_and_byte_progress() {
        let tracker = StartupTracker::terms_required();
        tracker.set_downloading("EmbeddingGemma 300M Q4", Some(25), Some(100));
        let status = tracker.status();
        assert_eq!(status.state, "downloading");
        assert_eq!(status.model, Some("EmbeddingGemma 300M Q4"));
        assert_eq!(status.downloaded_bytes, Some(25));
        assert_eq!(status.total_bytes, Some(100));
        assert_eq!(status.percent, Some(25));
    }

    #[test]
    fn unknown_download_size_does_not_invent_a_percentage() {
        let tracker = StartupTracker::terms_required();
        tracker.set_downloading("Nomic Embed Text v1.5", None, None);
        assert_eq!(tracker.status().percent, None);
    }

    /// Setup is pending only while the setup tools can still change something:
    /// a terms choice is outstanding, a download is in flight, or a setup
    /// failed and can be retried.
    #[test]
    fn only_the_setup_phases_report_setup_as_pending() {
        let tracker = StartupTracker::terms_required();
        assert!(tracker.model_setup_pending());

        tracker.set_downloading("EmbeddingGemma 300M Q4", Some(1), Some(2));
        assert!(tracker.model_setup_pending());

        tracker.set_model_setup_failed();
        assert!(tracker.model_setup_pending());
    }

    /// A collection that is rebuilding has finished setup, so the setup tools
    /// have nothing to offer it. This is #191: a post-write Index turn reports
    /// its progress on this very tracker, and treating that as `!is_ready()`
    /// sent every MCP caller to the model-setup tools for the duration.
    #[test]
    fn rebuilding_is_not_pending_setup() {
        let tracker = StartupTracker::ready();
        assert!(!tracker.model_setup_pending());

        tracker.set_scanning();
        assert!(!tracker.model_setup_pending());
        assert!(
            !tracker.collection_indexes_ready(),
            "still not Ready, just not a setup problem"
        );

        tracker.set_indexing(IndexingProgressSnapshot::default());
        assert!(!tracker.model_setup_pending());
    }

    /// `Unavailable` is not one condition: a failed index and a registry
    /// awaiting operator recovery both land here, and neither is answered by
    /// accepting a model licence. Only the error code tells them apart.
    #[test]
    fn unavailable_for_a_non_setup_reason_is_not_pending_setup() {
        let tracker = StartupTracker::ready();
        tracker.set_failed();
        assert!(!tracker.model_setup_pending());

        tracker.runtime().set_unavailable(
            "startup_recovery_required",
            "Startup recovery is required before Vaults can be activated",
        );
        assert!(!tracker.model_setup_pending());
    }
}
