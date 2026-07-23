use std::collections::HashMap;
use std::path::PathBuf;

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteEntry {
    pub title: String,
    pub slug: String,
    pub path: PathBuf,
    pub relative_path: String,
    /// `None` is the default surface.
    pub layer: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Note {
    pub title: String,
    pub slug: String,
    pub relative_path: String,
    pub content: String,
    pub content_hash: String,
    pub metadata: NoteMetadata,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct NoteMetadata {
    pub tags: Vec<String>,
    pub aliases: Vec<String>,
    pub properties: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NoteSummary {
    pub title: String,
    pub slug: String,
    pub relative_path: String,
    pub metadata: NoteMetadata,
}

#[derive(Debug, Clone)]
pub struct VaultIndex {
    pub by_slug: HashMap<String, NoteEntry>,
    #[cfg_attr(not(test), allow(dead_code))]
    pub by_title: HashMap<String, String>,
    #[cfg_attr(not(test), allow(dead_code))]
    pub by_path_title: HashMap<String, String>,
    pub ordered_slugs: Vec<String>,
    pub outgoing_by_slug: HashMap<String, Vec<String>>,
    pub backlinks_by_slug: HashMap<String, Vec<String>>,
    pub layers: super::layers::LayerMap,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExplorerFolder {
    pub name: String,
    pub folders: Vec<ExplorerFolder>,
    pub notes: Vec<ExplorerNote>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExplorerNote {
    pub title: String,
    pub slug: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModifiedNote {
    pub title: String,
    pub slug: String,
    pub relative_path: String,
    pub mtime_ns: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchHit {
    pub title: String,
    pub slug: String,
    pub relative_path: String,
    pub match_kind: String,
    pub snippet: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NoteLink {
    pub title: String,
    pub slug: String,
    pub relative_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NoteLinks {
    pub outgoing: Vec<NoteLink>,
    pub backlinks: Vec<NoteLink>,
}

/// Deployment-side scan configuration. Layer classification comes from the
/// vault itself and is not represented here.
#[derive(Debug, Default)]
pub struct VaultScanConfig {
    pub exclude: super::exclude::ExcludeMatcher,
}
