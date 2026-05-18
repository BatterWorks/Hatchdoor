use crate::eval::query::Query;

/// Per-query result. `top_k` is the ordered list of note slugs returned.
#[derive(Debug, Clone)]
pub struct QueryResult {
    pub query_id: String,
    pub top_k: Vec<String>,
}

/// Aggregate metrics for one model over the full query set.
#[derive(Debug, Clone)]
pub struct Report {
    pub model_id: String,
    pub recall_at_5_any: f64,
    pub recall_at_5_all: f64,
    pub recall_at_10_any: f64,
    pub recall_at_10_all: f64,
    pub mrr: f64,
    pub fp_rate_at_5: f64,
    pub per_query: Vec<PerQueryMetrics>,
}

#[derive(Debug, Clone)]
pub struct PerQueryMetrics {
    pub id: String,
    pub query: String,
    /// 1-based rank of the first expected note in top-10. None if not found.
    pub first_expected_rank: Option<usize>,
    /// Whether any anti_expected note appeared in top-5.
    pub anti_expected_hit_at_5: Option<bool>,
}

pub fn recall_at_k_any(expected: &[String], top_k: &[String]) -> bool {
    expected.iter().any(|e| top_k.iter().any(|t| t == e))
}

pub fn recall_at_k_all(expected: &[String], top_k: &[String]) -> f64 {
    if expected.is_empty() {
        return 0.0;
    }
    let hits = expected.iter().filter(|e| top_k.iter().any(|t| t == *e)).count();
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

    for q in queries {
        let result = by_id.get(q.id.as_str());
        let top_10: Vec<String> = result.map(|r| r.top_k.iter().take(10).cloned().collect()).unwrap_or_default();
        let top_5: Vec<String> = top_10.iter().take(5).cloned().collect();

        if recall_at_k_any(&q.expected_notes, &top_5) {
            sum_any_5 += 1.0;
        }
        if recall_at_k_any(&q.expected_notes, &top_10) {
            sum_any_10 += 1.0;
        }
        sum_all_5 += recall_at_k_all(&q.expected_notes, &top_5);
        sum_all_10 += recall_at_k_all(&q.expected_notes, &top_10);

        let rank = first_expected_rank(&q.expected_notes, &top_10);
        if let Some(r) = rank {
            sum_mrr += 1.0 / r as f64;
        }

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
        });
    }

    let n = queries.len().max(1) as f64;
    let fp_rate_at_5 = if anti_denom == 0 {
        0.0
    } else {
        anti_num as f64 / anti_denom as f64
    };

    Report {
        model_id: model_id.to_string(),
        recall_at_5_any: sum_any_5 / n,
        recall_at_5_all: sum_all_5 / n,
        recall_at_10_any: sum_any_10 / n,
        recall_at_10_all: sum_all_10 / n,
        mrr: sum_mrr / n,
        fp_rate_at_5,
        per_query,
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
        }
    }

    fn r(id: &str, top_k: &[&str]) -> QueryResult {
        QueryResult {
            query_id: id.to_string(),
            top_k: top_k.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn aggregate_computes_recall_mrr_fp() {
        let queries = vec![
            q("a", &["n1"], &[]),
            q("b", &["n2"], &["bad"]),
        ];
        let results = vec![
            r("a", &["n1", "x", "y", "z", "w", "u", "v", "p", "q", "r"]), // rank 1
            r("b", &["bad", "n2", "y", "z", "w", "u", "v", "p", "q", "r"]), // rank 2, anti hit
        ];
        let rep = aggregate("test", &queries, &results);
        assert_eq!(rep.recall_at_5_any, 1.0);
        assert_eq!(rep.recall_at_10_any, 1.0);
        assert!((rep.mrr - (1.0 + 0.5) / 2.0).abs() < 1e-9);
        assert!((rep.fp_rate_at_5 - 1.0).abs() < 1e-9, "fp denom is 1 (only b has anti), numerator is 1");
    }

    #[test]
    fn aggregate_handles_query_with_no_anti_expected_in_denom() {
        let queries = vec![q("a", &["n1"], &[])];
        let results = vec![r("a", &["n1"])];
        let rep = aggregate("test", &queries, &results);
        assert_eq!(rep.fp_rate_at_5, 0.0);
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
        assert_eq!(recall_at_k_all(&s(&["a", "b", "c"]), &s(&["a", "b", "z"])), 2.0 / 3.0);
    }

    #[test]
    fn recall_all_is_zero_when_no_expected() {
        assert_eq!(recall_at_k_all(&s(&[]), &s(&["a"])), 0.0);
    }

    #[test]
    fn first_rank_is_one_indexed() {
        assert_eq!(first_expected_rank(&s(&["b"]), &s(&["a", "b", "c"])), Some(2));
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
