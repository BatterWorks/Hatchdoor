use std::sync::{Arc, RwLock};

use serde::Serialize;

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

#[derive(Clone, Debug)]
enum StartupPhase {
    TermsRequired,
    Downloading {
        model: &'static str,
        downloaded_bytes: Option<u64>,
        total_bytes: Option<u64>,
    },
    Scanning,
    Indexing(IndexingProgressSnapshot),
    Ready,
    Failed {
        message: String,
    },
}

#[derive(Clone, Debug)]
pub struct StartupTracker(Arc<RwLock<StartupPhase>>);

impl StartupTracker {
    pub fn terms_required() -> Self {
        Self(Arc::new(RwLock::new(StartupPhase::TermsRequired)))
    }

    pub fn scanning() -> Self {
        Self(Arc::new(RwLock::new(StartupPhase::Scanning)))
    }

    pub fn ready() -> Self {
        Self(Arc::new(RwLock::new(StartupPhase::Ready)))
    }

    pub fn set_scanning(&self) {
        *self.0.write().expect("startup tracker poisoned") = StartupPhase::Scanning;
    }

    pub fn set_terms_required(&self) {
        *self.0.write().expect("startup tracker poisoned") = StartupPhase::TermsRequired;
    }

    pub fn set_downloading(
        &self,
        model: &'static str,
        downloaded_bytes: Option<u64>,
        total_bytes: Option<u64>,
    ) {
        *self.0.write().expect("startup tracker poisoned") = StartupPhase::Downloading {
            model,
            downloaded_bytes,
            total_bytes,
        };
    }

    pub fn set_indexing(&self, progress: IndexingProgressSnapshot) {
        *self.0.write().expect("startup tracker poisoned") = StartupPhase::Indexing(progress);
    }

    pub fn set_ready(&self) {
        *self.0.write().expect("startup tracker poisoned") = StartupPhase::Ready;
    }

    pub fn set_failed(&self) {
        self.set_failed_with_message("Indexing could not be completed.");
    }

    pub fn set_model_setup_failed(&self) {
        self.set_failed_with_message(
            "The search model could not be downloaded or loaded. Check the Hatchdoor logs, then retry setup.",
        );
    }

    fn set_failed_with_message(&self, message: impl Into<String>) {
        *self.0.write().expect("startup tracker poisoned") = StartupPhase::Failed {
            message: message.into(),
        };
    }

    pub fn is_ready(&self) -> bool {
        matches!(
            *self.0.read().expect("startup tracker poisoned"),
            StartupPhase::Ready
        )
    }

    pub fn status(&self) -> StartupStatusResponse {
        match *self.0.read().expect("startup tracker poisoned") {
            StartupPhase::TermsRequired => StartupStatusResponse::simple("terms_required", None),
            StartupPhase::Downloading {
                model,
                downloaded_bytes,
                total_bytes,
            } => StartupStatusResponse {
                state: "downloading",
                model: Some(model),
                downloaded_bytes,
                total_bytes,
                notes_completed: None,
                notes_total: None,
                chunks_completed: None,
                chunks_total: None,
                tokens_completed: None,
                tokens_total: None,
                percent: download_percent(downloaded_bytes, total_bytes),
                eta_seconds: None,
                message: None,
            },
            StartupPhase::Scanning => StartupStatusResponse::simple("scanning", None),
            StartupPhase::Indexing(progress) => StartupStatusResponse {
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
            },
            StartupPhase::Ready => StartupStatusResponse::simple("ready", None),
            StartupPhase::Failed { ref message } => {
                StartupStatusResponse::simple("failed", Some(message.clone()))
            }
        }
    }
}

#[derive(Clone, Debug, Serialize)]
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
        assert!(!tracker.is_ready());
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
}
