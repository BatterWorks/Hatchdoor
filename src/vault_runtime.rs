use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use serde::Serialize;

use crate::startup::IndexingProgressSnapshot;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VaultSource {
    Local { vault_path: PathBuf },
    ManagedGit(ManagedGitSource),
}

impl VaultSource {
    pub fn kind(&self) -> VaultSourceKind {
        match self {
            Self::Local { .. } => VaultSourceKind::Local,
            Self::ManagedGit(_) => VaultSourceKind::ManagedGit,
        }
    }

    pub fn mode(&self) -> VaultSourceMode {
        match self {
            Self::Local { .. } => VaultSourceMode::Local,
            Self::ManagedGit(source) => source.mode.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManagedGitSource {
    pub repository_url: String,
    pub checkout_path: PathBuf,
    pub branch: Option<String>,
    pub vault_subdirectory: Option<PathBuf>,
    pub mode: ManagedGitMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManagedGitMode {
    PullOnly,
    Bidirectional,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum VaultSourceKind {
    Local,
    ManagedGit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum VaultSourceMode {
    Local,
    PullOnly,
    Bidirectional,
}

impl From<ManagedGitMode> for VaultSourceMode {
    fn from(value: ManagedGitMode) -> Self {
        match value {
            ManagedGitMode::PullOnly => Self::PullOnly,
            ManagedGitMode::Bidirectional => Self::Bidirectional,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultPhase {
    TermsRequired,
    Downloading,
    Validating,
    Scanning,
    Indexing,
    Ready,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct VaultCapabilities {
    pub browse: bool,
    pub search: bool,
    pub mutate: bool,
    pub pull: bool,
    pub push: bool,
    pub retry: bool,
}

impl VaultCapabilities {
    fn derive(source_mode: VaultSourceMode, phase: VaultPhase) -> Self {
        let ready = phase == VaultPhase::Ready;
        Self {
            browse: ready,
            search: ready,
            mutate: ready
                && matches!(
                    source_mode,
                    VaultSourceMode::Local | VaultSourceMode::Bidirectional
                ),
            pull: ready
                && matches!(
                    source_mode,
                    VaultSourceMode::PullOnly | VaultSourceMode::Bidirectional
                ),
            push: ready && source_mode == VaultSourceMode::Bidirectional,
            retry: false,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct VaultRuntimeSnapshot {
    pub phase: VaultPhase,
    pub source: VaultSourceKind,
    pub mode: VaultSourceMode,
    pub capabilities: VaultCapabilities,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub downloaded_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indexing: Option<IndexingProgressSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<VaultRuntimeError>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VaultRuntimeError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

impl VaultRuntimeSnapshot {
    fn new(source: &VaultSource, phase: VaultPhase) -> Self {
        let mode = source.mode();
        Self {
            phase,
            source: source.kind(),
            mode,
            capabilities: VaultCapabilities::derive(mode, phase),
            model: None,
            downloaded_bytes: None,
            total_bytes: None,
            indexing: None,
            error: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct VaultRuntime {
    source: Arc<VaultSource>,
    snapshot: Arc<RwLock<VaultRuntimeSnapshot>>,
}

impl VaultRuntime {
    pub fn new(source: VaultSource) -> Self {
        let snapshot = VaultRuntimeSnapshot::new(&source, VaultPhase::Validating);
        Self {
            source: Arc::new(source),
            snapshot: Arc::new(RwLock::new(snapshot)),
        }
    }

    pub fn ready(source: VaultSource) -> Self {
        let runtime = Self::new(source);
        runtime.set_phase(VaultPhase::Ready);
        runtime
    }

    pub fn source(&self) -> &VaultSource {
        &self.source
    }

    pub fn snapshot(&self) -> VaultRuntimeSnapshot {
        self.snapshot
            .read()
            .expect("vault runtime snapshot poisoned")
            .clone()
    }

    pub fn is_ready(&self) -> bool {
        self.snapshot().phase == VaultPhase::Ready
    }

    pub fn set_scanning(&self) {
        self.set_phase(VaultPhase::Scanning);
    }

    pub fn set_terms_required(&self) {
        self.set_phase(VaultPhase::TermsRequired);
    }

    pub fn set_downloading(
        &self,
        model: &'static str,
        downloaded_bytes: Option<u64>,
        total_bytes: Option<u64>,
    ) {
        let mut snapshot = self
            .snapshot
            .write()
            .expect("vault runtime snapshot poisoned");
        snapshot.phase = VaultPhase::Downloading;
        snapshot.capabilities = VaultCapabilities::derive(snapshot.mode, snapshot.phase);
        snapshot.model = Some(model);
        snapshot.downloaded_bytes = downloaded_bytes;
        snapshot.total_bytes = total_bytes;
        snapshot.indexing = None;
        snapshot.error = None;
    }

    pub fn set_indexing(&self, progress: IndexingProgressSnapshot) {
        let mut snapshot = self
            .snapshot
            .write()
            .expect("vault runtime snapshot poisoned");
        snapshot.phase = VaultPhase::Indexing;
        snapshot.capabilities = VaultCapabilities::derive(snapshot.mode, snapshot.phase);
        snapshot.model = None;
        snapshot.downloaded_bytes = None;
        snapshot.total_bytes = None;
        snapshot.indexing = Some(progress);
        snapshot.error = None;
    }

    pub fn set_ready(&self) {
        self.set_phase(VaultPhase::Ready);
    }

    pub fn set_unavailable(&self, code: impl Into<String>, message: impl Into<String>) {
        let mut snapshot = self
            .snapshot
            .write()
            .expect("vault runtime snapshot poisoned");
        snapshot.phase = VaultPhase::Unavailable;
        snapshot.capabilities = VaultCapabilities::derive(snapshot.mode, snapshot.phase);
        snapshot.model = None;
        snapshot.downloaded_bytes = None;
        snapshot.total_bytes = None;
        snapshot.indexing = None;
        snapshot.error = Some(VaultRuntimeError {
            code: code.into(),
            message: message.into(),
            retryable: false,
        });
    }

    fn set_phase(&self, phase: VaultPhase) {
        let mut snapshot = self
            .snapshot
            .write()
            .expect("vault runtime snapshot poisoned");
        snapshot.phase = phase;
        snapshot.capabilities = VaultCapabilities::derive(snapshot.mode, phase);
        snapshot.model = None;
        snapshot.downloaded_bytes = None;
        snapshot.total_bytes = None;
        snapshot.indexing = None;
        snapshot.error = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn managed(mode: ManagedGitMode) -> VaultSource {
        VaultSource::ManagedGit(ManagedGitSource {
            repository_url: "https://example.test/vault.git".to_string(),
            checkout_path: PathBuf::from("/data/vault"),
            branch: None,
            vault_subdirectory: None,
            mode,
        })
    }

    #[test]
    fn pull_only_ready_state_never_allows_mutation_or_push() {
        let runtime = VaultRuntime::ready(managed(ManagedGitMode::PullOnly));
        let capabilities = runtime.snapshot().capabilities;
        assert!(capabilities.browse);
        assert!(capabilities.search);
        assert!(capabilities.pull);
        assert!(!capabilities.mutate);
        assert!(!capabilities.push);
    }

    #[test]
    fn unavailable_state_has_no_ready_vault_capabilities() {
        let runtime = VaultRuntime::new(managed(ManagedGitMode::Bidirectional));
        runtime.set_unavailable("not_acquired", "Vault has not been acquired");
        let snapshot = runtime.snapshot();
        assert_eq!(snapshot.phase, VaultPhase::Unavailable);
        assert!(!snapshot.capabilities.browse);
        assert!(!snapshot.capabilities.search);
        assert!(!snapshot.capabilities.mutate);
        assert!(!snapshot.capabilities.retry);
    }
}
