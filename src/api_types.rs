use serde::{Deserialize, Serialize};

use crate::vault::{ModifiedNote, Note, NoteLinks, SearchHit};

#[derive(Debug, Serialize)]
pub(crate) struct ErrorResponse {
    pub(crate) error: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct NoteResponse {
    pub(crate) note: Note,
}

#[derive(Debug, Serialize)]
pub(crate) struct NoteLinksResponse {
    pub(crate) links: NoteLinks,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResolveQuery {
    pub(crate) target: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct ResolveResponse {
    pub(crate) slug: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResolveBatchRequest {
    pub(crate) targets: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ResolveBatchResponse {
    pub(crate) results: Vec<ResolveTargetResult>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ResolveTargetResult {
    pub(crate) target: String,
    pub(crate) slug: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct RefreshResponse {
    pub(crate) refreshed: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RecentlyModifiedQuery {
    pub(crate) limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub(crate) struct RecentlyModifiedResponse {
    pub(crate) notes: Vec<ModifiedNote>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SearchQuery {
    pub(crate) q: String,
    pub(crate) content: Option<bool>,
    pub(crate) limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SearchResponse {
    pub(crate) results: Vec<SearchHit>,
}
