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
    Scanning,
    Indexing(IndexingProgressSnapshot),
    Ready,
    Failed,
}

#[derive(Clone, Debug)]
pub struct StartupTracker(Arc<RwLock<StartupPhase>>);

impl StartupTracker {
    pub fn scanning() -> Self {
        Self(Arc::new(RwLock::new(StartupPhase::Scanning)))
    }

    pub fn ready() -> Self {
        Self(Arc::new(RwLock::new(StartupPhase::Ready)))
    }

    pub fn set_scanning(&self) {
        *self.0.write().expect("startup tracker poisoned") = StartupPhase::Scanning;
    }

    pub fn set_indexing(&self, progress: IndexingProgressSnapshot) {
        *self.0.write().expect("startup tracker poisoned") = StartupPhase::Indexing(progress);
    }

    pub fn set_ready(&self) {
        *self.0.write().expect("startup tracker poisoned") = StartupPhase::Ready;
    }

    pub fn set_failed(&self) {
        *self.0.write().expect("startup tracker poisoned") = StartupPhase::Failed;
    }

    pub fn is_ready(&self) -> bool {
        matches!(
            *self.0.read().expect("startup tracker poisoned"),
            StartupPhase::Ready
        )
    }

    pub fn status(&self) -> StartupStatusResponse {
        match *self.0.read().expect("startup tracker poisoned") {
            StartupPhase::Scanning => StartupStatusResponse::simple("scanning", None),
            StartupPhase::Indexing(progress) => StartupStatusResponse {
                state: "indexing",
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
            StartupPhase::Failed => {
                StartupStatusResponse::simple("failed", Some("Indexing could not be completed."))
            }
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct StartupStatusResponse {
    pub state: &'static str,
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
    pub message: Option<&'static str>,
}

impl StartupStatusResponse {
    fn simple(state: &'static str, message: Option<&'static str>) -> Self {
        Self {
            state,
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
