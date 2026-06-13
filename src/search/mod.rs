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
    cache: &SqliteCache,
    embedder: &dyn Embedder,
    req: SearchRequest,
) -> Result<SearchResponse, String> {
    let trimmed = req.query.trim();
    if trimmed.is_empty() {
        return Err("query cannot be empty".to_string());
    }
    let req = SearchRequest {
        query: trimmed.to_string(),
        ..req
    };
    let mode = req.mode;
    let hits = retrieve::retrieve(cache, embedder, &req)?;
    let results = assemble::assemble(cache, hits)?;
    Ok(SearchResponse { mode, results })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tempfile::TempDir;

    use crate::cache::SqliteCache;
    use crate::embed::{Embedder, StubEmbedder};
    use crate::vault::VaultIndex;

    use super::{SearchMode, SearchRequest, run};

    fn build_cache(files: &[(&str, &str)]) -> (SqliteCache, Arc<dyn Embedder>) {
        let dir = TempDir::new().expect("tempdir");
        for (name, body) in files {
            std::fs::write(dir.path().join(name), body).expect("write");
        }
        let cache = SqliteCache::in_memory(384).expect("open");
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
        let index = VaultIndex::build(dir.path()).expect("build");
        cache
            .replace_from_index_with_embedder(&index, embedder.as_ref())
            .expect("index");
        (cache, embedder)
    }

    #[test]
    fn semantic_path_end_to_end() {
        let (cache, embedder) = build_cache(&[
            ("Alpha.md", "# Alpha\n\napples and oranges"),
            ("Bravo.md", "# Bravo\n\nbody"),
        ]);
        let resp = run(
            &cache,
            embedder.as_ref(),
            SearchRequest {
                query: "apples".to_string(),
                mode: SearchMode::Semantic,
                limit: 10,
                per_note_cap: 2,
            },
        )
        .expect("run");
        assert_eq!(resp.mode, SearchMode::Semantic);
        assert!(!resp.results.is_empty());
        assert!(resp.results[0].note_title == "Alpha" || resp.results[0].note_title == "Bravo");
    }

    #[test]
    fn keyword_path_end_to_end() {
        let (cache, embedder) = build_cache(&[
            ("Alpha.md", "# Alpha\n\noranges"),
            ("Bravo.md", "# Bravo\n\nbody"),
        ]);
        let resp = run(
            &cache,
            embedder.as_ref(),
            SearchRequest {
                query: "oranges".to_string(),
                mode: SearchMode::Keyword,
                limit: 10,
                per_note_cap: 2,
            },
        )
        .expect("run");
        assert_eq!(resp.mode, SearchMode::Keyword);
        assert_eq!(resp.results.len(), 1);
        assert_eq!(resp.results[0].note_slug, "alpha");
    }

    #[test]
    fn empty_query_errors() {
        let (cache, embedder) = build_cache(&[("Alpha.md", "# Alpha\n\nbody")]);
        let err = run(
            &cache,
            embedder.as_ref(),
            SearchRequest {
                query: "   ".to_string(),
                mode: SearchMode::Semantic,
                limit: 10,
                per_note_cap: 2,
            },
        )
        .expect_err("expected empty-query error");
        assert!(err.to_lowercase().contains("empty"));
    }

    #[test]
    fn over_fetch_compensates_for_single_note_flooding() {
        // One note with many distinct chunks (heading-separated). per_note_cap=1 means
        // only one chunk from this note can appear, but limit=3 should still try.
        let body = (0..20)
            .map(|i| format!("# H{i}\n\nsection {i} body text"))
            .collect::<Vec<_>>()
            .join("\n\n");
        let (cache, embedder) = build_cache(&[
            ("Alpha.md", body.as_str()),
            ("Bravo.md", "# Bravo\n\nunrelated"),
        ]);
        let resp = run(
            &cache,
            embedder.as_ref(),
            SearchRequest {
                query: "section".to_string(),
                mode: SearchMode::Keyword,
                limit: 3,
                per_note_cap: 1,
            },
        )
        .expect("run");
        // With per_note_cap=1, at most 1 chunk from Alpha. We may get 1 from Alpha + 0..1 from Bravo.
        let alpha_count = resp
            .results
            .iter()
            .filter(|r| r.note_slug == "alpha")
            .count();
        assert!(alpha_count <= 1);
    }
}
