use schemars::JsonSchema;
use std::io;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteOutcome {
    pub slug: Option<String>,
    pub relative_path: Option<String>,
    pub content_hash: Option<String>,
    pub quality_warnings: Vec<String>,
    pub rewritten_notes: usize,
    pub moved_assets: usize,
    pub trashed_path: Option<String>,
    /// Absolute paths created, modified, or removed by this operation
    /// (primary file, rewritten backlink notes, moved assets, rename source).
    pub affected_paths: Vec<std::path::PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, JsonSchema)]
pub struct AttachmentInfo {
    pub relative_path: String,
    pub size_bytes: u64,
    pub content_hash: String,
    /// The containing folder's layer (`None` = default surface). Additive and
    /// always present, like the `layer` field on note responses.
    pub layer: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentOutcome {
    pub attachment: AttachmentInfo,
    pub rewritten_notes: usize,
    pub trashed_path: Option<String>,
    pub cleanup_warning: Option<String>,
    /// Absolute paths created, modified, or removed by this operation.
    pub affected_paths: Vec<std::path::PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteError {
    Conflict(String),
    InvalidInput(String),
    Io(String),
}

impl WriteError {
    const RECOVERY_REQUIRED_PREFIX: &'static str = "recovery required: ";

    pub(crate) fn recovery_required(details: String) -> Self {
        Self::Io(format!("{}{details}", Self::RECOVERY_REQUIRED_PREFIX))
    }

    pub(crate) fn recovery_message(&self) -> Option<&str> {
        let Self::Io(message) = self else {
            return None;
        };
        message
            .strip_prefix(Self::RECOVERY_REQUIRED_PREFIX)
            .map(|_| message.as_str())
    }
}

#[derive(Debug, Clone)]
pub(super) struct TextRewrite {
    pub(super) path: PathBuf,
    pub(super) content: String,
}

#[derive(Debug, Clone)]
pub(super) struct AssetMove {
    pub(super) source: PathBuf,
    pub(super) destination: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MutationPhase {
    Note,
    Asset,
    Rewrite,
}

impl From<io::Error> for WriteError {
    fn from(error: io::Error) -> Self {
        Self::Io(error.to_string())
    }
}
