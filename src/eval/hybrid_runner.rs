use std::collections::{HashMap, HashSet};
use std::time::Instant;

use crate::cache::SqliteCache;
use crate::embed::Embedder;
use crate::eval::metrics::QueryResult;
use crate::eval::query::Query;

/// One hybrid eval record: the fused top-K plus the e2e latency for that query.
#[derive(Debug, Clone)]
pub struct HybridQueryResult {
    pub query_result: QueryResult,
    pub latency_ms: f64,
}

/// Run hybrid retrieval (semantic + FTS5 BM25, fused by Reciprocal Rank Fusion)
/// over a query set.
///
/// - `initial_k`: how many notes to take from each retriever before fusion.
/// - `rrf_k`: RRF smoothing constant (60 is the published default).
/// - `out_k`: how many fused results to retain per query (≥10 is needed for the
///   eval metrics, which look at top-5 and top-10).
#[allow(dead_code)]
pub fn run_hybrid_eval(
    cache: &SqliteCache,
    embedder: &dyn Embedder,
    queries: &[Query],
    initial_k: usize,
    rrf_k: usize,
    out_k: usize,
) -> Result<Vec<HybridQueryResult>, String> {
    let mut out = Vec::with_capacity(queries.len());
    for q in queries {
        let start = Instant::now();

        // Semantic side: pull chunks, dedupe up to notes (first occurrence per
        // note wins, which is also its best-ranked chunk).
        // Over-fetch chunks so that we can still reach `initial_k` distinct
        // notes when several top chunks belong to the same note.
        let chunk_fetch = initial_k.saturating_mul(4).max(initial_k);
        let sem_hits = cache.semantic_search(embedder, &q.query, chunk_fetch)?;
        let mut sem_notes: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for h in sem_hits {
            if seen.insert(h.note_slug.clone()) {
                sem_notes.push(h.note_slug);
                if sem_notes.len() >= initial_k {
                    break;
                }
            }
        }

        // Lexical side: note-level FTS5 BM25.
        let fts_notes = cache.fts_search_notes(&q.query, initial_k)?;

        // Reciprocal Rank Fusion.
        let mut scores: HashMap<String, f64> = HashMap::new();
        let k = rrf_k as f64;
        for (i, slug) in sem_notes.iter().enumerate() {
            *scores.entry(slug.clone()).or_insert(0.0) += 1.0 / (k + (i + 1) as f64);
        }
        for (i, slug) in fts_notes.iter().enumerate() {
            *scores.entry(slug.clone()).or_insert(0.0) += 1.0 / (k + (i + 1) as f64);
        }
        let mut fused: Vec<(String, f64)> = scores.into_iter().collect();
        fused.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let top_k: Vec<String> = fused.into_iter().take(out_k).map(|(s, _)| s).collect();

        let latency_ms = start.elapsed().as_secs_f64() * 1000.0;
        out.push(HybridQueryResult {
            query_result: QueryResult {
                query_id: q.id.clone(),
                top_k,
            },
            latency_ms,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::SqliteCache;
    use crate::embed::StubEmbedder;
    use crate::vault::VaultIndex;
    use std::sync::Arc;

    fn write_vault(dir: &std::path::Path, files: &[(&str, &str)]) {
        for (name, body) in files {
            let path = dir.join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, body).unwrap();
        }
    }

    #[test]
    fn hybrid_runner_emits_one_result_per_query() {
        let dir = tempfile::tempdir().expect("tempdir");
        let vault = dir.path().join("vault");
        std::fs::create_dir_all(&vault).unwrap();
        write_vault(
            &vault,
            &[
                (
                    "alpha.md",
                    "# Alpha\n\nalpha content about flying with babies on planes",
                ),
                (
                    "beta.md",
                    "# Beta\n\nbeta content unrelated topic gardening",
                ),
                ("gamma.md", "# Gamma\n\ngamma flying baby plane trip notes"),
            ],
        );

        let embedder = Arc::new(StubEmbedder::new(64));
        let cache = SqliteCache::in_memory(embedder.embedding_dim()).expect("open");
        let index = VaultIndex::build(&vault).expect("index");
        cache
            .replace_from_index_with_embedder(&index, embedder.as_ref())
            .expect("populate");

        let queries = vec![Query {
            id: "Q1".to_string(),
            query: "flying baby plane".to_string(),
            expected_notes: vec!["gamma".to_string()],
            expected_heading_path: None,
            anti_expected: vec![],
        }];

        let out = run_hybrid_eval(&cache, embedder.as_ref(), &queries, 5, 60, 10).expect("run");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].query_result.query_id, "Q1");
        assert!(!out[0].query_result.top_k.is_empty());
        assert!(out[0].latency_ms.is_finite() && out[0].latency_ms >= 0.0);
    }

    #[test]
    fn rrf_fusion_promotes_notes_present_in_both_lists() {
        // Build a small cache where FTS will find 'gamma' on a clear lexical match,
        // and confirm that the fused output ranks it at #1 even though the stub
        // embedder won't necessarily.
        let dir = tempfile::tempdir().expect("tempdir");
        let vault = dir.path().join("vault");
        std::fs::create_dir_all(&vault).unwrap();
        write_vault(
            &vault,
            &[
                ("alpha.md", "# Alpha\n\nlongish content with various words"),
                ("beta.md", "# Beta\n\nanother body of text"),
                (
                    "gamma.md",
                    "# Gamma\n\nstorage pool layout for the archive server",
                ),
            ],
        );

        let embedder = Arc::new(StubEmbedder::new(64));
        let cache = SqliteCache::in_memory(embedder.embedding_dim()).expect("open");
        let index = VaultIndex::build(&vault).expect("index");
        cache
            .replace_from_index_with_embedder(&index, embedder.as_ref())
            .expect("populate");

        let queries = vec![Query {
            id: "Q1".to_string(),
            query: "storage pool layout archive server".to_string(),
            expected_notes: vec!["gamma".to_string()],
            expected_heading_path: None,
            anti_expected: vec![],
        }];
        let out = run_hybrid_eval(&cache, embedder.as_ref(), &queries, 5, 60, 10).expect("run");
        assert_eq!(
            out[0].query_result.top_k.first().map(String::as_str),
            Some("gamma")
        );
    }
}
