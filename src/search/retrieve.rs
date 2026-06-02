//! Phase 2 retrieval stage. Dispatches by mode, applies the per-note cap.

use std::collections::HashMap;

use crate::cache::SqliteCache;
use crate::embed::Embedder;

use super::{SearchMode, SearchRequest};

const RAW_K_CEILING: usize = 200;

#[derive(Debug, Clone)]
pub struct ChunkHit {
    pub chunk_id: i64,
    pub note_slug: String,
    pub heading_path: Option<String>,
    pub content: String,
    pub score: f32, // normalized: higher = better
}

pub fn retrieve(
    cache: &SqliteCache,
    embedder: &dyn Embedder,
    req: &SearchRequest,
) -> Result<Vec<ChunkHit>, String> {
    let raw_k = (req.limit.saturating_mul(req.per_note_cap)).min(RAW_K_CEILING);
    if raw_k == 0 {
        return Ok(Vec::new());
    }

    let raw_hits: Vec<ChunkHit> = match req.mode {
        SearchMode::Semantic => semantic(cache, embedder, &req.query, raw_k)?,
        SearchMode::Keyword => keyword(cache, &req.query, raw_k)?,
    };

    Ok(apply_per_note_cap(raw_hits, req.per_note_cap, req.limit))
}

fn semantic(
    cache: &SqliteCache,
    embedder: &dyn Embedder,
    query: &str,
    k: usize,
) -> Result<Vec<ChunkHit>, String> {
    let hits = cache.semantic_search(embedder, query, k)?;
    Ok(hits
        .into_iter()
        .map(|h| ChunkHit {
            chunk_id: h.chunk_id,
            note_slug: h.note_slug,
            heading_path: h.heading_path,
            content: h.content,
            score: (1.0 - h.distance).clamp(0.0, 1.0),
        })
        .collect())
}

fn keyword(
    cache: &SqliteCache,
    query: &str,
    k: usize,
) -> Result<Vec<ChunkHit>, String> {
    let hits = cache.fts_search_chunks(query, k)?;
    if hits.is_empty() {
        return Ok(Vec::new());
    }
    // BM25 ascending (lower = better). Normalize to (0.0, 1.0] where higher is better.
    // Single-row case: assign 1.0 to avoid division-by-zero and to give the lone hit the
    // strongest possible score.
    let b_max = hits
        .iter()
        .map(|h| h.bm25.abs())
        .fold(f32::NEG_INFINITY, f32::max);
    Ok(hits
        .into_iter()
        .map(|h| {
            let score = if b_max <= f32::EPSILON {
                1.0
            } else {
                (h.bm25.abs() / b_max).clamp(0.0, 1.0)
            };
            ChunkHit {
                chunk_id: h.chunk_id,
                note_slug: h.note_slug,
                heading_path: h.heading_path,
                content: h.content,
                score,
            }
        })
        .collect())
}

fn apply_per_note_cap(
    raw: Vec<ChunkHit>,
    per_note_cap: usize,
    limit: usize,
) -> Vec<ChunkHit> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut out = Vec::with_capacity(limit.min(raw.len()));
    for h in raw {
        let n = seen.entry(h.note_slug.clone()).or_insert(0);
        if *n < per_note_cap {
            *n += 1;
            out.push(h);
            if out.len() >= limit {
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tempfile::TempDir;

    use crate::cache::SqliteCache;
    use crate::embed::{Embedder, StubEmbedder};
    use crate::search::{SearchMode, SearchRequest};
    use crate::vault::VaultIndex;

    use super::retrieve;

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
    fn semantic_mode_returns_hits_ordered_by_score_desc() {
        let (cache, embedder) = build_cache(&[
            ("a.md", "# A\n\napples and oranges"),
            ("b.md", "# B\n\nspokes and wheels"),
        ]);
        let req = SearchRequest {
            query: "apples".to_string(),
            mode: SearchMode::Semantic,
            limit: 10,
            per_note_cap: 2,
        };
        let hits = retrieve(&cache, embedder.as_ref(), &req).expect("retrieve");
        assert!(!hits.is_empty());
        for w in hits.windows(2) {
            assert!(w[0].score >= w[1].score, "scores must be non-increasing");
        }
        for h in &hits {
            assert!(h.score >= 0.0 && h.score <= 1.0, "score out of range: {}", h.score);
        }
    }

    #[test]
    fn semantic_mode_returns_empty_when_cache_has_no_chunks() {
        let cache = SqliteCache::in_memory(384).expect("open");
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
        let req = SearchRequest {
            query: "anything".to_string(),
            mode: SearchMode::Semantic,
            limit: 10,
            per_note_cap: 2,
        };
        let hits = retrieve(&cache, embedder.as_ref(), &req).expect("retrieve");
        assert!(hits.is_empty());
    }

    #[test]
    fn keyword_mode_returns_hits_with_normalized_scores() {
        let (cache, embedder) = build_cache(&[
            ("a.md", "# A\n\napples and oranges"),
            ("b.md", "# B\n\noranges only"),
            ("c.md", "# C\n\nbananas"),
        ]);
        let req = SearchRequest {
            query: "oranges".to_string(),
            mode: SearchMode::Keyword,
            limit: 10,
            per_note_cap: 2,
        };
        let hits = retrieve(&cache, embedder.as_ref(), &req).expect("retrieve");
        assert!(!hits.is_empty());
        for w in hits.windows(2) {
            assert!(w[0].score >= w[1].score, "scores must be non-increasing");
        }
        for h in &hits {
            assert!(h.score > 0.0 && h.score <= 1.0, "score out of range: {}", h.score);
        }
        // bananas chunk should NOT be in keyword results for "oranges"
        assert!(!hits.iter().any(|h| h.content.contains("bananas")));
    }

    #[test]
    fn keyword_mode_returns_empty_when_query_has_no_tokens() {
        let (cache, embedder) = build_cache(&[("a.md", "# A\n\nbody")]);
        let req = SearchRequest {
            query: "   ".to_string(),
            mode: SearchMode::Keyword,
            limit: 10,
            per_note_cap: 2,
        };
        let hits = retrieve(&cache, embedder.as_ref(), &req).expect("retrieve");
        assert!(hits.is_empty());
    }

    #[test]
    fn keyword_mode_scores_are_non_increasing_with_three_matches() {
        let (cache, embedder) = build_cache(&[
            ("a.md", "# A\n\noranges oranges oranges"),  // best BM25
            ("b.md", "# B\n\noranges oranges"),
            ("c.md", "# C\n\noranges"),                  // worst BM25
        ]);
        let req = SearchRequest {
            query: "oranges".to_string(),
            mode: SearchMode::Keyword,
            limit: 10,
            per_note_cap: 1,
        };
        let hits = retrieve(&cache, embedder.as_ref(), &req).expect("retrieve");
        assert_eq!(hits.len(), 3);
        for w in hits.windows(2) {
            assert!(w[0].score >= w[1].score, "scores must be non-increasing: {} < {}", w[0].score, w[1].score);
        }
        // Best match (most oranges) should have highest score
        assert!((hits[0].score - 1.0).abs() < 0.01, "best hit should have score near 1.0, got {}", hits[0].score);
    }

    #[test]
    fn keyword_mode_single_result_gets_max_score() {
        let (cache, embedder) = build_cache(&[
            ("a.md", "# A\n\nuniquetoken-xyzzy"),
            ("b.md", "# B\n\nirrelevant"),
        ]);
        let req = SearchRequest {
            query: "uniquetoken-xyzzy".to_string(),
            mode: SearchMode::Keyword,
            limit: 10,
            per_note_cap: 2,
        };
        let hits = retrieve(&cache, embedder.as_ref(), &req).expect("retrieve");
        assert_eq!(hits.len(), 1);
        assert!((hits[0].score - 1.0).abs() < f32::EPSILON);
    }
}
