use crate::eval::query::Query;

/// Per-query result. `top_k` is the ordered list of note slugs returned;
/// `top_k_headings` is the parallel list of each hit's heading path (used to
/// score correct-heading rate). Paths may be empty for retrievers that don't
/// surface headings (e.g. the hybrid path), which simply score no heading hits.
#[derive(Debug, Clone)]
pub struct QueryResult {
    pub query_id: String,
    pub top_k: Vec<String>,
    pub top_k_headings: Vec<Option<String>>,
}

/// Aggregate metrics for one model over the full query set.
#[derive(Debug, Clone)]
pub struct Report {
    pub model_id: String,
    pub reranker_id: Option<String>,
    pub recall_at_5_any: f64,
    pub recall_at_5_all: f64,
    pub recall_at_10_any: f64,
    pub recall_at_10_all: f64,
    pub mrr: f64,
    pub fp_rate_at_5: f64,
    /// Fraction of heading-scoped queries whose expected note+heading appeared
    /// in top-10. `None` when no query declares an `expected_heading_path`.
    pub correct_heading_rate: Option<f64>,
    /// Metrics broken out per `category` (empty when no query is tagged).
    pub per_category: Vec<GroupReport>,
    /// Metrics broken out per `language` (empty when no query is tagged).
    pub per_language: Vec<GroupReport>,
    pub per_query: Vec<PerQueryMetrics>,
    pub rerank_latency_ms: Option<LatencyStats>,
    pub e2e_latency_ms: Option<LatencyStats>,
}

/// Metrics for one subset of queries sharing a grouping label (category or
/// language). Reported alongside the overall numbers so a change that helps on
/// average but hurts a specific slice is visible rather than hidden.
#[derive(Debug, Clone)]
pub struct GroupReport {
    pub label: String,
    pub n: usize,
    pub recall_at_5_any: f64,
    pub recall_at_10_any: f64,
    pub mrr: f64,
    pub correct_heading_rate: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct PerQueryMetrics {
    pub id: String,
    pub query: String,
    /// 1-based rank of the first expected note in top-10. None if not found.
    pub first_expected_rank: Option<usize>,
    /// Whether any anti_expected note appeared in top-5.
    pub anti_expected_hit_at_5: Option<bool>,
    /// 1-based rank before reranking (embed-only retrieval). None for non-rerank runs.
    pub rank_pre_rerank: Option<usize>,
    /// 1-based rank after reranking. None for non-rerank runs.
    pub rank_post_rerank: Option<usize>,
}

pub fn recall_at_k_any(expected: &[String], top_k: &[String]) -> bool {
    expected.iter().any(|e| top_k.iter().any(|t| t == e))
}

pub fn recall_at_k_all(expected: &[String], top_k: &[String]) -> f64 {
    if expected.is_empty() {
        return 0.0;
    }
    let hits = expected
        .iter()
        .filter(|e| top_k.iter().any(|t| t == *e))
        .count();
    hits as f64 / expected.len() as f64
}

pub fn first_expected_rank(expected: &[String], top_k: &[String]) -> Option<usize> {
    top_k.iter().enumerate().find_map(|(i, t)| {
        if expected.iter().any(|e| e == t) {
            Some(i + 1)
        } else {
            None
        }
    })
}

pub fn any_anti_expected_in_top_k(anti: &[String], top_k: &[String]) -> bool {
    anti.iter().any(|a| top_k.iter().any(|t| t == a))
}

/// Whether the query is answered at heading granularity: some hit is both an
/// expected note AND carries the expected heading path. `None` when the query
/// declares no `expected_heading_path` (it isn't a heading-scoped query, so it
/// is excluded from the correct-heading rate rather than counted as a miss).
pub fn heading_hit(
    expected_notes: &[String],
    expected_heading: Option<&str>,
    top_slugs: &[String],
    top_headings: &[Option<String>],
) -> Option<bool> {
    let wanted = expected_heading?;
    let hit = top_slugs.iter().zip(top_headings).any(|(slug, heading)| {
        expected_notes.iter().any(|e| e == slug) && heading.as_deref() == Some(wanted)
    });
    Some(hit)
}

/// Per-query scored data retained for grouping into category/language reports.
struct GroupRow {
    category: Option<String>,
    language: Option<String>,
    hit5: bool,
    hit10: bool,
    rr: f64,
    heading: Option<bool>,
}

fn heading_rate(rows: &[&GroupRow]) -> Option<f64> {
    let scored: Vec<bool> = rows.iter().filter_map(|r| r.heading).collect();
    if scored.is_empty() {
        return None;
    }
    Some(scored.iter().filter(|h| **h).count() as f64 / scored.len() as f64)
}

/// Group rows by a label key (category or language), preserving first-seen order
/// and skipping untagged rows, into one [`GroupReport`] each.
fn group_reports(rows: &[GroupRow], key: impl Fn(&GroupRow) -> Option<&str>) -> Vec<GroupReport> {
    let mut labels: Vec<String> = Vec::new();
    for row in rows {
        if let Some(label) = key(row)
            && !labels.iter().any(|seen| seen == label)
        {
            labels.push(label.to_string());
        }
    }
    labels
        .into_iter()
        .map(|label| {
            let subset: Vec<&GroupRow> = rows
                .iter()
                .filter(|r| key(r) == Some(label.as_str()))
                .collect();
            let denom = subset.len().max(1) as f64;
            GroupReport {
                recall_at_5_any: subset.iter().filter(|r| r.hit5).count() as f64 / denom,
                recall_at_10_any: subset.iter().filter(|r| r.hit10).count() as f64 / denom,
                mrr: subset.iter().map(|r| r.rr).sum::<f64>() / denom,
                correct_heading_rate: heading_rate(&subset),
                n: subset.len(),
                label,
            }
        })
        .collect()
}

pub fn aggregate(model_id: &str, queries: &[Query], results: &[QueryResult]) -> Report {
    let by_id: std::collections::HashMap<&str, &QueryResult> =
        results.iter().map(|r| (r.query_id.as_str(), r)).collect();

    let mut sum_any_5 = 0.0;
    let mut sum_any_10 = 0.0;
    let mut sum_all_5 = 0.0;
    let mut sum_all_10 = 0.0;
    let mut sum_mrr = 0.0;
    let mut anti_denom = 0usize;
    let mut anti_num = 0usize;
    let mut per_query = Vec::with_capacity(queries.len());
    let mut rows: Vec<GroupRow> = Vec::with_capacity(queries.len());

    for q in queries {
        let result = by_id.get(q.id.as_str());
        let top_10: Vec<String> = result
            .map(|r| r.top_k.iter().take(10).cloned().collect())
            .unwrap_or_default();
        let top_10_headings: Vec<Option<String>> = result
            .map(|r| r.top_k_headings.iter().take(10).cloned().collect())
            .unwrap_or_default();
        let top_5: Vec<String> = top_10.iter().take(5).cloned().collect();

        let hit5 = recall_at_k_any(&q.expected_notes, &top_5);
        let hit10 = recall_at_k_any(&q.expected_notes, &top_10);
        if hit5 {
            sum_any_5 += 1.0;
        }
        if hit10 {
            sum_any_10 += 1.0;
        }
        sum_all_5 += recall_at_k_all(&q.expected_notes, &top_5);
        sum_all_10 += recall_at_k_all(&q.expected_notes, &top_10);

        let rank = first_expected_rank(&q.expected_notes, &top_10);
        let rr = rank.map(|r| 1.0 / r as f64).unwrap_or(0.0);
        sum_mrr += rr;

        let heading = heading_hit(
            &q.expected_notes,
            q.expected_heading_path.as_deref(),
            &top_10,
            &top_10_headings,
        );

        let anti_hit = if q.anti_expected.is_empty() {
            None
        } else {
            anti_denom += 1;
            let hit = any_anti_expected_in_top_k(&q.anti_expected, &top_5);
            if hit {
                anti_num += 1;
            }
            Some(hit)
        };

        per_query.push(PerQueryMetrics {
            id: q.id.clone(),
            query: q.query.clone(),
            first_expected_rank: rank,
            anti_expected_hit_at_5: anti_hit,
            rank_pre_rerank: None,
            rank_post_rerank: None,
        });
        rows.push(GroupRow {
            category: q.category.clone(),
            language: q.language.clone(),
            hit5,
            hit10,
            rr,
            heading,
        });
    }

    let n = queries.len().max(1) as f64;
    let fp_rate_at_5 = if anti_denom == 0 {
        0.0
    } else {
        anti_num as f64 / anti_denom as f64
    };
    let all_rows: Vec<&GroupRow> = rows.iter().collect();
    let correct_heading_rate = heading_rate(&all_rows);
    let per_category = group_reports(&rows, |r| r.category.as_deref());
    let per_language = group_reports(&rows, |r| r.language.as_deref());

    Report {
        model_id: model_id.to_string(),
        reranker_id: None,
        recall_at_5_any: sum_any_5 / n,
        recall_at_5_all: sum_all_5 / n,
        recall_at_10_any: sum_any_10 / n,
        recall_at_10_all: sum_all_10 / n,
        mrr: sum_mrr / n,
        fp_rate_at_5,
        correct_heading_rate,
        per_category,
        per_language,
        per_query,
        rerank_latency_ms: None,
        e2e_latency_ms: None,
    }
}

/// Median / p90 / max for a series of latency samples.
#[derive(Debug, Clone, Copy)]
pub struct LatencyStats {
    pub median: f64,
    pub p90: f64,
    pub max: f64,
}

impl LatencyStats {
    /// `samples` must be non-empty.
    pub fn from_samples(samples: &[f64]) -> Self {
        assert!(
            !samples.is_empty(),
            "LatencyStats::from_samples needs ≥1 sample"
        );
        let mut sorted: Vec<f64> = samples.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = sorted.len();
        let median = sorted[n / 2];
        // ceil(0.9 * n) - 1, clamped to [0, n-1]
        let p90_idx = ((0.9 * n as f64).ceil() as usize)
            .saturating_sub(1)
            .min(n - 1);
        let p90 = sorted[p90_idx];
        let max = sorted[n - 1];
        Self { median, p90, max }
    }
}

/// Per-query record emitted by the rerank runner.
#[derive(Debug, Clone)]
pub struct RerankQueryResult {
    pub query_id: String,
    pub top_k_pre: Vec<String>,
    pub top_k_post: Vec<String>,
    pub rerank_latency_ms: f64,
    pub e2e_latency_ms: f64,
}

pub fn aggregate_rerank(
    run_id: &str,
    reranker_id: &str,
    queries: &[Query],
    results: &[RerankQueryResult],
) -> Report {
    let by_id: std::collections::HashMap<&str, &RerankQueryResult> =
        results.iter().map(|r| (r.query_id.as_str(), r)).collect();

    let mut sum_any_5 = 0.0;
    let mut sum_any_10 = 0.0;
    let mut sum_all_5 = 0.0;
    let mut sum_all_10 = 0.0;
    let mut sum_mrr = 0.0;
    let mut anti_denom = 0usize;
    let mut anti_num = 0usize;
    let mut per_query = Vec::with_capacity(queries.len());

    for q in queries {
        let result = by_id.get(q.id.as_str());
        let top_10_post: Vec<String> = result
            .map(|r| r.top_k_post.iter().take(10).cloned().collect())
            .unwrap_or_default();
        let top_5_post: Vec<String> = top_10_post.iter().take(5).cloned().collect();

        if recall_at_k_any(&q.expected_notes, &top_5_post) {
            sum_any_5 += 1.0;
        }
        if recall_at_k_any(&q.expected_notes, &top_10_post) {
            sum_any_10 += 1.0;
        }
        sum_all_5 += recall_at_k_all(&q.expected_notes, &top_5_post);
        sum_all_10 += recall_at_k_all(&q.expected_notes, &top_10_post);

        let rank_post = first_expected_rank(&q.expected_notes, &top_10_post);
        if let Some(r) = rank_post {
            sum_mrr += 1.0 / r as f64;
        }
        let rank_pre = result
            .map(|r| first_expected_rank(&q.expected_notes, &r.top_k_pre))
            .unwrap_or(None);

        let anti_hit = if q.anti_expected.is_empty() {
            None
        } else {
            anti_denom += 1;
            let hit = any_anti_expected_in_top_k(&q.anti_expected, &top_5_post);
            if hit {
                anti_num += 1;
            }
            Some(hit)
        };

        per_query.push(PerQueryMetrics {
            id: q.id.clone(),
            query: q.query.clone(),
            first_expected_rank: rank_post,
            anti_expected_hit_at_5: anti_hit,
            rank_pre_rerank: rank_pre,
            rank_post_rerank: rank_post,
        });
    }

    let n = queries.len().max(1) as f64;
    let fp_rate_at_5 = if anti_denom == 0 {
        0.0
    } else {
        anti_num as f64 / anti_denom as f64
    };

    let rerank_samples: Vec<f64> = results.iter().map(|r| r.rerank_latency_ms).collect();
    let e2e_samples: Vec<f64> = results.iter().map(|r| r.e2e_latency_ms).collect();
    let rerank_latency_ms =
        (!rerank_samples.is_empty()).then(|| LatencyStats::from_samples(&rerank_samples));
    let e2e_latency_ms =
        (!e2e_samples.is_empty()).then(|| LatencyStats::from_samples(&e2e_samples));

    Report {
        model_id: run_id.to_string(),
        reranker_id: Some(reranker_id.to_string()),
        recall_at_5_any: sum_any_5 / n,
        recall_at_5_all: sum_all_5 / n,
        recall_at_10_any: sum_any_10 / n,
        recall_at_10_all: sum_all_10 / n,
        mrr: sum_mrr / n,
        fp_rate_at_5,
        // The rerank path collapses to note slugs and carries no query tags, so
        // heading and per-group breakdowns don't apply here.
        correct_heading_rate: None,
        per_category: Vec::new(),
        per_language: Vec::new(),
        per_query,
        rerank_latency_ms,
        e2e_latency_ms,
    }
}

#[cfg(test)]
mod aggregate_tests {
    use super::*;
    use crate::eval::query::Query;

    fn q(id: &str, expected: &[&str], anti: &[&str]) -> Query {
        Query {
            id: id.to_string(),
            query: format!("q-{id}"),
            expected_notes: expected.iter().map(|s| s.to_string()).collect(),
            expected_heading_path: None,
            anti_expected: anti.iter().map(|s| s.to_string()).collect(),
            category: None,
            language: None,
        }
    }

    fn r(id: &str, top_k: &[&str]) -> QueryResult {
        QueryResult {
            query_id: id.to_string(),
            top_k: top_k.iter().map(|s| s.to_string()).collect(),
            top_k_headings: vec![None; top_k.len()],
        }
    }

    /// Result helper with parallel heading paths, for correct-heading tests.
    fn r_h(id: &str, hits: &[(&str, Option<&str>)]) -> QueryResult {
        QueryResult {
            query_id: id.to_string(),
            top_k: hits.iter().map(|(s, _)| s.to_string()).collect(),
            top_k_headings: hits.iter().map(|(_, h)| h.map(|x| x.to_string())).collect(),
        }
    }

    #[test]
    fn aggregate_computes_recall_mrr_fp() {
        let queries = vec![q("a", &["n1"], &[]), q("b", &["n2"], &["bad"])];
        let results = vec![
            r("a", &["n1", "x", "y", "z", "w", "u", "v", "p", "q", "r"]), // rank 1
            r("b", &["bad", "n2", "y", "z", "w", "u", "v", "p", "q", "r"]), // rank 2, anti hit
        ];
        let rep = aggregate("test", &queries, &results);
        assert_eq!(rep.recall_at_5_any, 1.0);
        assert_eq!(rep.recall_at_10_any, 1.0);
        assert!((rep.mrr - (1.0 + 0.5) / 2.0).abs() < 1e-9);
        assert!(
            (rep.fp_rate_at_5 - 1.0).abs() < 1e-9,
            "fp denom is 1 (only b has anti), numerator is 1"
        );
    }

    #[test]
    fn aggregate_handles_query_with_no_anti_expected_in_denom() {
        let queries = vec![q("a", &["n1"], &[])];
        let results = vec![r("a", &["n1"])];
        let rep = aggregate("test", &queries, &results);
        assert_eq!(rep.fp_rate_at_5, 0.0);
    }

    fn q_tagged(id: &str, note: &str, heading: Option<&str>, category: &str) -> Query {
        Query {
            id: id.to_string(),
            query: format!("q-{id}"),
            expected_notes: vec![note.to_string()],
            expected_heading_path: heading.map(str::to_string),
            anti_expected: Vec::new(),
            category: Some(category.to_string()),
            language: None,
        }
    }

    #[test]
    fn aggregate_reports_heading_rate_and_per_category() {
        let queries = vec![
            q_tagged("a", "n1", Some("Backups > Restoring"), "heading"),
            q_tagged("b", "n2", Some("Setup"), "heading"),
            q_tagged("c", "n3", None, "exact-name"),
        ];
        let results = vec![
            r_h("a", &[("n1", Some("Backups > Restoring"))]), // note + heading hit
            r_h("b", &[("n2", Some("Wrong heading"))]),       // note hit, heading miss
            r_h("c", &[("n3", None)]),                        // note hit, not heading-scoped
        ];
        let rep = aggregate("test", &queries, &results);

        // 2 heading-scoped queries, 1 correct → 0.5; the exact-name query is
        // excluded from the heading denominator.
        assert_eq!(rep.correct_heading_rate, Some(0.5));

        // First-seen category order: "heading" (n=2) then "exact-name" (n=1).
        assert_eq!(rep.per_category.len(), 2);
        assert_eq!(rep.per_category[0].label, "heading");
        assert_eq!(rep.per_category[0].n, 2);
        assert_eq!(rep.per_category[0].correct_heading_rate, Some(0.5));
        assert_eq!(rep.per_category[0].recall_at_5_any, 1.0);
        // A category with no heading-scoped query reports no heading rate.
        assert_eq!(rep.per_category[1].label, "exact-name");
        assert_eq!(rep.per_category[1].correct_heading_rate, None);

        // No language tags → no per-language rows.
        assert!(rep.per_language.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn recall_any_true_when_one_expected_in_top_k() {
        assert!(recall_at_k_any(&s(&["a", "b"]), &s(&["x", "a", "y"])));
    }

    #[test]
    fn recall_any_false_when_none_in_top_k() {
        assert!(!recall_at_k_any(&s(&["a"]), &s(&["x", "y"])));
    }

    #[test]
    fn recall_all_is_fraction_present() {
        assert_eq!(
            recall_at_k_all(&s(&["a", "b", "c"]), &s(&["a", "b", "z"])),
            2.0 / 3.0
        );
    }

    #[test]
    fn recall_all_is_zero_when_no_expected() {
        assert_eq!(recall_at_k_all(&s(&[]), &s(&["a"])), 0.0);
    }

    #[test]
    fn heading_hit_requires_both_matching_note_and_heading() {
        let expected = s(&["runbook"]);
        let slugs = s(&["runbook", "other"]);
        let headings = vec![Some("Backups".to_string()), None];
        assert_eq!(
            heading_hit(&expected, Some("Backups"), &slugs, &headings),
            Some(true)
        );
        // Expected note present but under a different heading.
        assert_eq!(
            heading_hit(&expected, Some("Setup"), &slugs, &headings),
            Some(false)
        );
        // Right heading text, but on a non-expected note.
        assert_eq!(
            heading_hit(
                &expected,
                Some("Backups"),
                &s(&["other"]),
                &[Some("Backups".to_string())]
            ),
            Some(false)
        );
        // Not a heading-scoped query → excluded from scoring.
        assert_eq!(heading_hit(&expected, None, &slugs, &headings), None);
    }

    #[test]
    fn first_rank_is_one_indexed() {
        assert_eq!(
            first_expected_rank(&s(&["b"]), &s(&["a", "b", "c"])),
            Some(2)
        );
    }

    #[test]
    fn first_rank_is_none_when_absent() {
        assert_eq!(first_expected_rank(&s(&["z"]), &s(&["a", "b"])), None);
    }

    #[test]
    fn anti_expected_hit_detected() {
        assert!(any_anti_expected_in_top_k(&s(&["bad"]), &s(&["a", "bad"])));
        assert!(!any_anti_expected_in_top_k(&s(&["bad"]), &s(&["a", "b"])));
    }
}

#[cfg(test)]
mod rerank_tests {
    use super::*;
    use crate::eval::query::Query;

    fn q(id: &str, query: &str, expected: &[&str], anti: &[&str]) -> Query {
        Query {
            id: id.to_string(),
            query: query.to_string(),
            expected_notes: expected.iter().map(|s| s.to_string()).collect(),
            expected_heading_path: None,
            anti_expected: anti.iter().map(|s| s.to_string()).collect(),
            category: None,
            language: None,
        }
    }

    fn top(slugs: &[&str]) -> Vec<String> {
        slugs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn aggregate_with_rerank_populates_pre_and_post_ranks() {
        let queries = vec![q("Q1", "any", &["x"], &[])];
        let pre = vec![RerankQueryResult {
            query_id: "Q1".to_string(),
            top_k_pre: top(&["a", "b", "x"]), // expected at rank 3 pre
            top_k_post: top(&["x", "a", "b"]), // expected at rank 1 post
            rerank_latency_ms: 12.0,
            e2e_latency_ms: 25.0,
        }];
        let report = aggregate_rerank("Nomic+JinaV2", "JinaV2", &queries, &pre);
        assert_eq!(report.per_query.len(), 1);
        let pq = &report.per_query[0];
        assert_eq!(pq.rank_pre_rerank, Some(3));
        assert_eq!(pq.rank_post_rerank, Some(1));
        assert_eq!(pq.first_expected_rank, Some(1)); // matches post for compatibility
        assert!(report.recall_at_5_any >= 0.999); // 1.0
    }

    #[test]
    fn aggregate_rerank_computes_latency_percentiles() {
        let queries = vec![
            q("Q1", "a", &["x"], &[]),
            q("Q2", "b", &["x"], &[]),
            q("Q3", "c", &["x"], &[]),
            q("Q4", "d", &["x"], &[]),
            q("Q5", "e", &["x"], &[]),
        ];
        let pre = vec![
            mk_qr("Q1", 10.0, 20.0),
            mk_qr("Q2", 12.0, 22.0),
            mk_qr("Q3", 14.0, 24.0),
            mk_qr("Q4", 16.0, 26.0),
            mk_qr("Q5", 100.0, 200.0),
        ];
        let report = aggregate_rerank("Nomic+JinaV2", "JinaV2", &queries, &pre);
        let stats = report.rerank_latency_ms.expect("present");
        assert!((stats.median - 14.0).abs() < 1e-6);
        // p90 of 5 values (sorted [10,12,14,16,100]) → index ceil(0.90*5)-1 = 4 → 100
        assert!((stats.p90 - 100.0).abs() < 1e-6);
        assert!((stats.max - 100.0).abs() < 1e-6);
    }

    fn mk_qr(id: &str, rerank_ms: f64, e2e_ms: f64) -> RerankQueryResult {
        RerankQueryResult {
            query_id: id.to_string(),
            top_k_pre: top(&["x"]),
            top_k_post: top(&["x"]),
            rerank_latency_ms: rerank_ms,
            e2e_latency_ms: e2e_ms,
        }
    }
}
