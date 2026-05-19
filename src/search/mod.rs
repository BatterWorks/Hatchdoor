//! Phase 2 search orchestrator. Consumed by both MCP and HTTP.

use serde::{Deserialize, Serialize};

use crate::cache::SqliteCache;
use crate::embed::Embedder;

pub mod assemble;
pub mod retrieve;

pub use retrieve::ChunkHit;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SearchMode {
    #[default]
    Semantic,
    Keyword,
}

#[derive(Debug, Clone)]
pub struct SearchRequest {
    pub query: String,
    pub mode: SearchMode,
    pub limit: usize,
    pub per_note_cap: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct OutboundLink {
    pub slug: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub chunk_id: i64,
    pub note_slug: String,
    pub note_title: String,
    pub note_path: String,
    pub heading_path: Option<String>,
    pub content: String,
    pub score: f32,
    pub outbound_links: Vec<OutboundLink>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchResponse {
    pub mode: SearchMode,
    pub results: Vec<SearchResult>,
}

pub fn run(
    _cache: &SqliteCache,
    _embedder: &dyn Embedder,
    _req: SearchRequest,
) -> Result<SearchResponse, String> {
    unimplemented!("filled in by later tasks")
}
