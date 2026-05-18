use serde::{Deserialize, Serialize};

use crate::vault::{ModifiedNote, Note, NoteLinks, SearchHit};

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Debug, Serialize)]
pub struct NoteResponse {
    pub note: Note,
}

#[derive(Debug, Serialize)]
pub struct NoteLinksResponse {
    pub links: NoteLinks,
}

#[derive(Debug, Deserialize)]
pub struct ResolveQuery {
    pub target: String,
}

#[derive(Debug, Serialize)]
pub struct ResolveResponse {
    pub slug: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ResolveBatchRequest {
    pub targets: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ResolveBatchResponse {
    pub results: Vec<ResolveTargetResult>,
}

#[derive(Debug, Serialize)]
pub struct ResolveTargetResult {
    pub target: String,
    pub slug: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RefreshResponse {
    pub refreshed: bool,
}

#[derive(Debug, Serialize)]
pub struct VaultEventResponse {
    pub revision: u64,
}

#[derive(Debug, Deserialize)]
pub struct RecentlyModifiedQuery {
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct RecentlyModifiedResponse {
    pub notes: Vec<ModifiedNote>,
}

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: String,
    pub content: Option<bool>,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct SearchResponse {
    pub results: Vec<SearchHit>,
}
