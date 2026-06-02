use std::collections::HashSet;

use crate::cache::SqliteCache;
use crate::embed::Embedder;
use crate::eval::hybrid_runner::{run_hybrid_eval, HybridQueryResult};
use crate::eval::metrics::first_expected_rank;
use crate::eval::query::Query;

/// Per-query comparison between pure-semantic and hybrid retrieval.
#[derive(Debug, Clone)]
pub struct CompareQueryResult {
    pub query_id: String,
    pub query_text: String,
    /// 1-based rank of first expected note in pure-semantic top-10, None if absent.
    pub rank_pure: Option<usize>,
    /// 1-based rank of first expected note in hybrid top-10, None if absent.
    pub rank_hybrid: Option<usize>,
    /// Whether any anti-expected note appeared in top-5 for pure semantic.
    /// None if the query has no anti list.
    pub anti_pure: Option<bool>,
    /// Whether any anti-expected note appeared in top-5 for hybrid.
    /// None if the query has no anti list.
    pub anti_hybrid: Option<bool>,
}

/// Outcome counts over the full compare run.
#[derive(Debug, Clone, Default)]
pub struct CompareSummary {
    /// Hybrid rank < pure rank (hybrid wins).
    pub hybrid_wins: usize,
    /// Both ranks equal (or both None).
    pub ties: usize,
    /// Hybrid rank > pure rank (pure wins).
    pub pure_wins: usize,
    /// Hybrid dropped an anti that pure had in top-5 (improvement).
    pub anti_improvements: usize,
    /// Hybrid added an anti that pure did not have in top-5 (regression).
    pub anti_regressions: usize,
}

/// Compute per-query win/loss/tie counts from a slice of compare results.
pub fn compare_summary(results: &[CompareQueryResult]) -> CompareSummary {
    let mut s = CompareSummary::default();
    for r in results {
        // rank comparison: None = miss (treat as rank 11 for comparison purposes)
        let r_pure = r.rank_pure.unwrap_or(11);
        let r_hybrid = r.rank_hybrid.unwrap_or(11);
        match r_hybrid.cmp(&r_pure) {
            std::cmp::Ordering::Less => s.hybrid_wins += 1,
            std::cmp::Ordering::Equal => s.ties += 1,
            std::cmp::Ordering::Greater => s.pure_wins += 1,
        }
        // anti flip analysis
        match (r.anti_pure, r.anti_hybrid) {
            (Some(true), Some(false)) => s.anti_improvements += 1,
            (Some(false), Some(true)) => s.anti_regressions += 1,
            _ => {}
        }
    }
    s
}

/// Run both pure-semantic and hybrid retrieval on all queries and return per-query comparison.
pub fn run_compare_eval(
    cache: &SqliteCache,
    embedder: &dyn Embedder,
    queries: &[Query],
    initial_k: usize,
    rrf_k: usize,
) -> Result<(Vec<CompareQueryResult>, CompareSummary), String> {
    // --- Pure semantic side (matching hybrid's note-collapse logic) ---
    let mut pure_results: Vec<(String, Vec<String>)> = Vec::with_capacity(queries.len());
    for q in queries {
        let chunk_fetch = initial_k.saturating_mul(4).max(initial_k);
        let sem_hits = cache.semantic_search(embedder, &q.query, chunk_fetch)?;
        let mut sem_notes: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for h in sem_hits {
            if seen.insert(h.note_slug.clone()) {
                sem_notes.push(h.note_slug);
                if sem_notes.len() >= 10 {
                    break;
                }
            }
        }
        pure_results.push((q.id.clone(), sem_notes));
    }

    // --- Hybrid side ---
    let hybrid: Vec<HybridQueryResult> =
        run_hybrid_eval(cache, embedder, queries, initial_k, rrf_k, 10)?;
    let hybrid_by_id: std::collections::HashMap<&str, &HybridQueryResult> =
        hybrid.iter().map(|h| (h.query_result.query_id.as_str(), h)).collect();
    let pure_by_id: std::collections::HashMap<&str, &Vec<String>> =
        pure_results.iter().map(|(id, v)| (id.as_str(), v)).collect();

    // --- Build per-query comparison ---
    let mut compare = Vec::with_capacity(queries.len());
    for q in queries {
        let pure_top = pure_by_id.get(q.id.as_str()).map(|v| v.as_slice()).unwrap_or(&[]);
        let hybrid_top = hybrid_by_id
            .get(q.id.as_str())
            .map(|h| h.query_result.top_k.as_slice())
            .unwrap_or(&[]);

        let pure_top10: Vec<String> = pure_top.iter().take(10).cloned().collect();
        let hybrid_top10: Vec<String> = hybrid_top.iter().take(10).cloned().collect();
        let pure_top5: Vec<String> = pure_top10.iter().take(5).cloned().collect();
        let hybrid_top5: Vec<String> = hybrid_top10.iter().take(5).cloned().collect();

        let rank_pure = first_expected_rank(&q.expected_notes, &pure_top10);
        let rank_hybrid = first_expected_rank(&q.expected_notes, &hybrid_top10);

        let (anti_pure, anti_hybrid) = if q.anti_expected.is_empty() {
            (None, None)
        } else {
            let ap = crate::eval::metrics::any_anti_expected_in_top_k(&q.anti_expected, &pure_top5);
            let ah =
                crate::eval::metrics::any_anti_expected_in_top_k(&q.anti_expected, &hybrid_top5);
            (Some(ap), Some(ah))
        };

        compare.push(CompareQueryResult {
            query_id: q.id.clone(),
            query_text: q.query.clone(),
            rank_pure,
            rank_hybrid,
            anti_pure,
            anti_hybrid,
        });
    }

    let summary = compare_summary(&compare);
    Ok((compare, summary))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(
        rank_pure: Option<usize>,
        rank_hybrid: Option<usize>,
        anti_pure: Option<bool>,
        anti_hybrid: Option<bool>,
    ) -> CompareQueryResult {
        CompareQueryResult {
            query_id: "X".to_string(),
            query_text: "test".to_string(),
            rank_pure,
            rank_hybrid,
            anti_pure,
            anti_hybrid,
        }
    }

    #[test]
    fn hybrid_win_when_rank_lower() {
        let results = vec![mk(Some(3), Some(1), None, None)];
        let s = compare_summary(&results);
        assert_eq!(s.hybrid_wins, 1);
        assert_eq!(s.ties, 0);
        assert_eq!(s.pure_wins, 0);
    }

    #[test]
    fn pure_win_when_hybrid_rank_higher() {
        let results = vec![mk(Some(1), Some(4), None, None)];
        let s = compare_summary(&results);
        assert_eq!(s.pure_wins, 1);
        assert_eq!(s.hybrid_wins, 0);
    }

    #[test]
    fn tie_when_same_rank() {
        let results = vec![mk(Some(2), Some(2), None, None)];
        let s = compare_summary(&results);
        assert_eq!(s.ties, 1);
    }

    #[test]
    fn tie_when_both_miss() {
        let results = vec![mk(None, None, None, None)];
        let s = compare_summary(&results);
        assert_eq!(s.ties, 1);
    }

    #[test]
    fn hybrid_wins_miss_vs_hit() {
        // pure misses (None → 11), hybrid hits at rank 5
        let results = vec![mk(None, Some(5), None, None)];
        let s = compare_summary(&results);
        assert_eq!(s.hybrid_wins, 1);
    }

    #[test]
    fn anti_improvement_counted() {
        // pure had anti in top-5, hybrid does not
        let results = vec![mk(Some(1), Some(1), Some(true), Some(false))];
        let s = compare_summary(&results);
        assert_eq!(s.anti_improvements, 1);
        assert_eq!(s.anti_regressions, 0);
    }

    #[test]
    fn anti_regression_counted() {
        let results = vec![mk(Some(1), Some(1), Some(false), Some(true))];
        let s = compare_summary(&results);
        assert_eq!(s.anti_regressions, 1);
        assert_eq!(s.anti_improvements, 0);
    }

    #[test]
    fn mixed_batch_counts() {
        let results = vec![
            mk(Some(3), Some(1), None, None),  // hybrid wins
            mk(Some(1), Some(2), None, None),  // pure wins
            mk(Some(2), Some(2), Some(false), Some(true)), // tie + anti regression
        ];
        let s = compare_summary(&results);
        assert_eq!(s.hybrid_wins, 1);
        assert_eq!(s.pure_wins, 1);
        assert_eq!(s.ties, 1);
        assert_eq!(s.anti_regressions, 1);
    }
}
