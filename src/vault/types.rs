use std::collections::HashMap;
use std::path::PathBuf;

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteEntry {
    pub title: String,
    pub slug: String,
    pub path: PathBuf,
    pub relative_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Note {
    pub title: String,
    pub slug: String,
    pub relative_path: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct VaultIndex {
    pub(crate) by_slug: HashMap<String, NoteEntry>,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) by_title: HashMap<String, String>,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) by_path_title: HashMap<String, String>,
    pub(crate) ordered_slugs: Vec<String>,
    pub(crate) outgoing_by_slug: HashMap<String, Vec<String>>,
    pub(crate) backlinks_by_slug: HashMap<String, Vec<String>>,
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
