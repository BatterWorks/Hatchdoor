use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, JsonSchema, Deserialize)]
pub struct TagStat {
    pub tag: String,
    pub note_count: i64,
}

#[derive(Debug, Serialize, JsonSchema, Deserialize)]
pub struct NoteRef {
    pub title: String,
    pub slug: String,
}

#[derive(Debug, Serialize, JsonSchema, Deserialize)]
pub struct NoteWordRef {
    pub title: String,
    pub slug: String,
    pub word_count: usize,
}

#[derive(Debug, Serialize, JsonSchema, Deserialize)]
pub struct LinkedNoteRef {
    pub title: String,
    pub slug: String,
    pub backlink_count: i64,
}

#[derive(Debug, Serialize, JsonSchema, Deserialize)]
pub struct MonthActivity {
    pub month: String,
    pub modified_count: i64,
}

#[derive(Debug, Serialize, JsonSchema, Deserialize)]
pub struct FolderStat {
    pub folder: String,
    pub note_count: i64,
}

#[derive(Debug, Serialize, JsonSchema, Deserialize)]
pub struct NoteList {
    pub count: i64,
    pub notes: Vec<NoteRef>,
}

#[derive(Debug, Serialize, JsonSchema, Deserialize)]
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
    /// The node's layer (`None` = default surface).
    pub layer: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct GraphEdge {
    pub source: String,
    pub target: String,
    /// Layers of the edge's endpoints (`None` = default surface).
    pub source_layer: Option<String>,
    pub target_layer: Option<String>,
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

#[derive(Debug, Deserialize)]
pub struct ResolveQuery {
    pub target: String,
}

#[derive(Debug, Deserialize)]
pub struct ResolveBatchRequest {
    pub targets: Vec<String>,
    /// Embed and PDF wikilink targets, which resolve to a path rather than a
    /// slug (#158). Defaulted so a client that only resolves note links keeps
    /// working unchanged.
    #[serde(default)]
    pub asset_targets: Vec<String>,
    /// Vault-relative path of the note the targets were written in, without the
    /// `.md` suffix. Asset resolution is relative to its folder; absent, the
    /// targets are read from the Vault root.
    #[serde(default)]
    pub note_path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ResolveTargetResult {
    pub target: String,
    pub slug: Option<String>,
    pub archived: bool,
}

/// One asset target's resolution. `path` is Vault-relative and servable by the
/// Vault asset route; `None` means nothing matched, which the client renders as
/// a missing link rather than a URL that would 404.
#[derive(Debug, Serialize)]
pub struct ResolveAssetResult {
    pub target: String,
    pub path: Option<String>,
}

/// Still consumed by `src/mcp/tools/read.rs`'s own refresh tool over the
/// legacy shared-core call (#103, unaffected by #101's HTTP route removal).
#[derive(Debug, Serialize)]
pub struct RefreshResponse {
    pub refreshed: bool,
}

#[derive(Debug, Deserialize)]
pub struct RecentlyModifiedQuery {
    pub limit: Option<usize>,
}
