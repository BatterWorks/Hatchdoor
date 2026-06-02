use std::sync::Arc;

use crate::cache::SemanticHit;

/// One reranked candidate. `rerank_score` is higher-is-better; the
/// embedding distance is preserved purely for diagnostics.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct RerankedHit {
    pub chunk_id: i64,
    pub note_slug: String,
    pub heading_path: Option<String>,
    pub content: String,
    pub embedding_distance: f32,
    pub rerank_score: f32,
}

/// Second-stage cross-encoder. Re-scores a small set of candidates
/// returned from `SqliteCache::semantic_search` against the raw query
/// text. The input order is the embedding retrieval order; the output
/// is sorted by descending `rerank_score`.
#[allow(dead_code)]
pub trait Reranker: Send + Sync {
    fn rerank(
        &self,
        query: &str,
        candidates: Vec<SemanticHit>,
    ) -> Result<Vec<RerankedHit>, String>;

    fn id(&self) -> &'static str;
}

/// Deterministic test double. Scores each candidate by the count of
/// whitespace-separated query tokens that appear (case-insensitively)
/// as substrings in `candidate.content`. Ties broken by lower original
/// distance, then by ascending chunk_id for full determinism.
#[allow(dead_code)]
pub struct StubReranker;

impl StubReranker {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self
    }
}

impl Default for StubReranker {
    fn default() -> Self {
        Self::new()
    }
}

impl Reranker for StubReranker {
    fn rerank(
        &self,
        query: &str,
        candidates: Vec<SemanticHit>,
    ) -> Result<Vec<RerankedHit>, String> {
        let query_lower = query.to_lowercase();
        let query_tokens: Vec<&str> = query_lower
            .split_whitespace()
            .filter(|t| !t.is_empty())
            .collect();

        let mut scored: Vec<RerankedHit> = candidates
            .into_iter()
            .map(|hit| {
                let content_lower = hit.content.to_lowercase();
                let overlap = query_tokens
                    .iter()
                    .filter(|t| content_lower.contains(*t))
                    .count();
                RerankedHit {
                    chunk_id: hit.chunk_id,
                    note_slug: hit.note_slug,
                    heading_path: hit.heading_path,
                    content: hit.content,
                    embedding_distance: hit.distance,
                    rerank_score: overlap as f32,
                }
            })
            .collect();

        scored.sort_by(|a, b| {
            b.rerank_score
                .partial_cmp(&a.rerank_score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(
                    a.embedding_distance
                        .partial_cmp(&b.embedding_distance)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
                .then(a.chunk_id.cmp(&b.chunk_id))
        });

        Ok(scored)
    }

    fn id(&self) -> &'static str {
        "StubReranker"
    }
}

/// Allow `Arc<dyn Reranker>` through generic call sites in the eval.
impl<T: Reranker + ?Sized> Reranker for Arc<T> {
    fn rerank(
        &self,
        query: &str,
        candidates: Vec<SemanticHit>,
    ) -> Result<Vec<RerankedHit>, String> {
        (**self).rerank(query, candidates)
    }

    fn id(&self) -> &'static str {
        (**self).id()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::SemanticHit;

    fn hit(chunk_id: i64, slug: &str, content: &str, distance: f32) -> SemanticHit {
        SemanticHit {
            chunk_id,
            note_slug: slug.to_string(),
            heading_path: None,
            content: content.to_string(),
            distance,
        }
    }

    #[test]
    fn stub_orders_by_query_token_overlap_descending() {
        let r = StubReranker::new();
        let candidates = vec![
            hit(1, "a", "nothing relevant here", 0.10),
            hit(2, "b", "flying with the baby on a plane", 0.20),
            hit(3, "c", "baby tummy time", 0.30),
        ];
        let out = r
            .rerank("flying with the baby on a plane", candidates)
            .expect("rerank");
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].note_slug, "b");
        assert!(out[0].rerank_score >= out[1].rerank_score);
        assert!(out[1].rerank_score >= out[2].rerank_score);
    }

    #[test]
    fn stub_preserves_input_size_and_carries_embedding_distance() {
        let r = StubReranker::new();
        let candidates = vec![
            hit(1, "a", "alpha", 0.10),
            hit(2, "b", "alpha beta", 0.20),
        ];
        let out = r.rerank("alpha beta gamma", candidates.clone()).expect("rerank");
        assert_eq!(out.len(), candidates.len());
        let mut slugs: Vec<&str> = out.iter().map(|h| h.note_slug.as_str()).collect();
        slugs.sort();
        assert_eq!(slugs, vec!["a", "b"]);
        let b = out.iter().find(|h| h.note_slug == "b").unwrap();
        assert!((b.embedding_distance - 0.20).abs() < 1e-6);
    }

    #[test]
    fn stub_returns_empty_for_empty_input() {
        let r = StubReranker::new();
        let out = r.rerank("anything", vec![]).expect("rerank");
        assert!(out.is_empty());
    }

    #[test]
    fn stub_ties_broken_by_distance_then_chunk_id() {
        let r = StubReranker::new();
        let candidates = vec![
            hit(2, "b", "no overlap", 0.30),
            hit(1, "a", "no overlap", 0.20),
            hit(3, "c", "no overlap", 0.20),
        ];
        let out = r.rerank("xyzzy", candidates).expect("rerank");
        assert_eq!(out[0].note_slug, "a"); // distance 0.20, chunk_id 1
        assert_eq!(out[1].note_slug, "c"); // distance 0.20, chunk_id 3
        assert_eq!(out[2].note_slug, "b"); // distance 0.30
    }

    #[test]
    fn id_returns_stub_marker() {
        let r = StubReranker::new();
        assert_eq!(r.id(), "StubReranker");
    }
}
