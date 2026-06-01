use serde::{Deserialize, Serialize};

use crate::search::SearchMode;
use crate::vault::{ModifiedNote, Note, NoteLinks};

#[derive(Debug, Serialize)]
pub struct TagStat {
    pub tag: String,
    pub note_count: i64,
}

#[derive(Debug, Serialize)]
pub struct NoteRef {
    pub title: String,
    pub slug: String,
}

#[derive(Debug, Serialize)]
pub struct NoteWordRef {
    pub title: String,
    pub slug: String,
    pub word_count: usize,
}

#[derive(Debug, Serialize)]
pub struct LinkedNoteRef {
    pub title: String,
    pub slug: String,
    pub backlink_count: i64,
}

#[derive(Debug, Serialize)]
pub struct MonthActivity {
    pub month: String,
    pub modified_count: i64,
}

#[derive(Debug, Serialize)]
pub struct FolderStat {
    pub folder: String,
    pub note_count: i64,
}

#[derive(Debug, Serialize)]
pub struct NoteList {
    pub count: i64,
    pub notes: Vec<NoteRef>,
}

#[derive(Debug, Serialize)]
pub struct VaultStatsResponse {
    pub note_count: i64,
    pub word_count: usize,
    pub tag_count: i64,
    pub link_count: i64,
    pub image_count: usize,
    pub avg_word_count: usize,
    pub vault_size_bytes: i64,
    pub total_outgoing_links: i64,
    pub total_backlinks: i64,
    pub top_tags: Vec<TagStat>,
    pub most_linked: Vec<LinkedNoteRef>,
    pub activity_by_month: Vec<MonthActivity>,
    pub notes_per_folder: Vec<FolderStat>,
    pub longest_notes: Vec<NoteWordRef>,
    pub shortest_notes: Vec<NoteWordRef>,
    pub orphan_notes: Vec<NoteRef>,
    pub no_tag_notes: Vec<NoteRef>,
    pub modified_this_week: NoteList,
    pub modified_this_month: NoteList,
}

#[derive(Debug, Serialize)]
pub struct GraphNode {
    pub slug: String,
    pub title: String,
    pub primary_tag: Option<String>,
    pub backlink_count: i64,
}

#[derive(Debug, Serialize)]
pub struct GraphEdge {
    pub source: String,
    pub target: String,
}

#[derive(Debug, Serialize)]
pub struct GraphResponse {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

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
    pub archived: bool,
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
    #[serde(default)]
    pub mode: Option<SearchMode>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub per_note_cap: Option<usize>,
}
