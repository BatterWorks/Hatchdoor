//! Phase 2 retrieval stage. Dispatches by mode, applies the per-note cap.

use crate::cache::SqliteCache;
use crate::embed::Embedder;

use super::SearchRequest;

#[derive(Debug, Clone)]
pub struct ChunkHit {
    pub chunk_id: i64,
    pub note_slug: String,
    pub heading_path: Option<String>,
    pub content: String,
    pub score: f32, // normalized: higher = better
}

pub fn retrieve(
    _cache: &SqliteCache,
    _embedder: &dyn Embedder,
    _req: &SearchRequest,
) -> Result<Vec<ChunkHit>, String> {
    unimplemented!("filled in by later tasks")
}
