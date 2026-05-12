use std::io;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WriteOutcome {
    pub(crate) slug: Option<String>,
    pub(crate) relative_path: Option<String>,
    pub(crate) content_hash: Option<String>,
    pub(crate) rewritten_notes: usize,
    pub(crate) moved_assets: usize,
    pub(crate) trashed_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct AttachmentInfo {
    pub(crate) relative_path: String,
    pub(crate) size_bytes: u64,
    pub(crate) content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AttachmentOutcome {
    pub(crate) attachment: AttachmentInfo,
    pub(crate) rewritten_notes: usize,
    pub(crate) trashed_path: Option<String>,
    pub(crate) cleanup_warning: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WriteError {
    Conflict(String),
    InvalidInput(String),
    Io(String),
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

impl From<io::Error> for WriteError {
    fn from(error: io::Error) -> Self {
        Self::Io(error.to_string())
    }
}
