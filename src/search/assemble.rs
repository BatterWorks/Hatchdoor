//! Phase 2 context assembly stage.

use crate::cache::SqliteCache;

use super::{ChunkHit, SearchResult};

pub fn assemble(
    _cache: &SqliteCache,
    _hits: Vec<ChunkHit>,
) -> Result<Vec<SearchResult>, String> {
    unimplemented!("filled in by later tasks")
}
