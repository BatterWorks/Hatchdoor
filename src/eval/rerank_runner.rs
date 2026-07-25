use std::time::Instant;

use crate::cache::SqliteCache;
use crate::embed::Embedder;
use crate::eval::metrics::RerankQueryResult;
use crate::eval::query::Query;
use crate::rerank::Reranker;

#[allow(dead_code)]
pub fn run_rerank_eval(
    cache: &SqliteCache,
    embedder: &dyn Embedder,
    reranker: &dyn Reranker,
    queries: &[Query],
    initial_k: usize,
) -> Result<Vec<RerankQueryResult>, String> {
    let mut out = Vec::with_capacity(queries.len());
    for q in queries {
        let e2e_start = Instant::now();
        let candidates = cache.semantic_search(embedder, &q.query, initial_k)?;
        let top_k_pre: Vec<String> = candidates.iter().map(|c| c.note_slug.clone()).collect();

        let rerank_start = Instant::now();
        let reranked = reranker.rerank(&q.query, candidates)?;
        let rerank_latency_ms = rerank_start.elapsed().as_secs_f64() * 1000.0;

        let top_k_post: Vec<String> = reranked.iter().map(|c| c.note_slug.clone()).collect();
        let e2e_latency_ms = e2e_start.elapsed().as_secs_f64() * 1000.0;

        out.push(RerankQueryResult {
            query_id: q.id.clone(),
            top_k_pre,
            top_k_post,
            rerank_latency_ms,
            e2e_latency_ms,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::SqliteCache;
    use crate::embed::StubEmbedder;
    use crate::rerank::StubReranker;
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
    fn runner_emits_one_result_per_query_with_pre_and_post_orderings() {
        let dir = tempfile::tempdir().expect("tempdir");
        let vault = dir.path().join("vault");
        std::fs::create_dir_all(&vault).unwrap();
        write_vault(
            &vault,
            &[
                (
                    "alpha.md",
                    "# Alpha\n\nalpha content about flying with babies",
                ),
                ("beta.md", "# Beta\n\nbeta content unrelated topic"),
                ("gamma.md", "# Gamma\n\ngamma flying baby plane"),
            ],
        );

        let embedder = Arc::new(StubEmbedder::new(64));
        let cache = SqliteCache::in_memory(embedder.embedding_dim()).expect("open");

        let index = VaultIndex::build(&vault).expect("index");
        cache
            .replace_from_index_with_embedder(&index, embedder.as_ref())
            .expect("populate");

        let reranker = StubReranker::new();
        let queries = vec![Query {
            id: "Q1".to_string(),
            query: "flying baby plane".to_string(),
            expected_notes: vec!["gamma".to_string()],
            expected_heading_path: None,
            category: None,
            language: None,
            tier: None,
            anti_expected: vec![],
        }];
        let out = run_rerank_eval(&cache, embedder.as_ref(), &reranker, &queries, 5).expect("run");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].query_id, "Q1");
        assert!(!out[0].top_k_pre.is_empty());
        assert!(!out[0].top_k_post.is_empty());
        assert!(out[0].rerank_latency_ms.is_finite() && out[0].rerank_latency_ms >= 0.0);
        assert!(out[0].e2e_latency_ms >= out[0].rerank_latency_ms);
    }
}
