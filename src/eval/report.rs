use std::io::Write;
use std::path::Path;

use crate::eval::compare_runner::{CompareQueryResult, CompareSummary};
use crate::eval::metrics::{GroupReport, Report};

/// Write a markdown table for one grouping dimension (category or language),
/// or nothing when no query carried that tag.
fn write_group_table(f: &mut impl Write, title: &str, groups: &[GroupReport]) {
    if groups.is_empty() {
        return;
    }
    writeln!(f, "### {title}").ok();
    writeln!(f).ok();
    writeln!(
        f,
        "| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |"
    )
    .ok();
    writeln!(f, "|---|---|---|---|---|---|").ok();
    for g in groups {
        let heading = g
            .correct_heading_rate
            .map(|r| format!("{r:.3}"))
            .unwrap_or_else(|| "n/a".to_string());
        writeln!(
            f,
            "| {} | {} | {:.3} | {:.3} | {:.3} | {heading} |",
            g.label, g.n, g.recall_at_5_any, g.recall_at_10_any, g.mrr
        )
        .ok();
    }
    writeln!(f).ok();
}

/// Build-phase telemetry read back from cache metadata, reported alongside the
/// retrieval metrics so each results section records how the index was produced.
#[derive(Debug, Default, Clone)]
pub struct BuildInfo {
    pub duration_secs: Option<f64>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub peak_rss_mb: Option<f64>,
}

pub fn append_section(path: &Path, report: &Report, build: &BuildInfo) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create parent: {e}"))?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("open {}: {e}", path.display()))?;

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let dur = build
        .duration_secs
        .map(|s| format!("{s:.1} s"))
        .unwrap_or_else(|| "(unknown)".to_string());

    writeln!(f).ok();
    writeln!(f, "## {}", report.model_id).map_err(|e| format!("write: {e}"))?;
    writeln!(f).ok();
    writeln!(f, "- Run timestamp: {now}").ok();
    writeln!(f, "- Build duration: {dur}").ok();
    if let (Some(start), Some(end)) = (&build.started_at, &build.finished_at) {
        writeln!(f, "- Build window: {start} → {end}").ok();
    }
    if let Some(rss) = build.peak_rss_mb {
        writeln!(f, "- Build peak RSS: {rss:.1} MB").ok();
    }
    writeln!(f).ok();
    writeln!(f, "| Metric | Value |").ok();
    writeln!(f, "|---|---|").ok();
    writeln!(f, "| Recall@5 (any) | {:.3} |", report.recall_at_5_any).ok();
    writeln!(f, "| Recall@5 (all) | {:.3} |", report.recall_at_5_all).ok();
    writeln!(f, "| Recall@10 (any) | {:.3} |", report.recall_at_10_any).ok();
    writeln!(f, "| Recall@10 (all) | {:.3} |", report.recall_at_10_all).ok();
    writeln!(f, "| MRR | {:.3} |", report.mrr).ok();
    writeln!(f, "| FP-rate@5 | {:.3} |", report.fp_rate_at_5).ok();
    match report.correct_heading_rate {
        Some(rate) => writeln!(f, "| Correct-heading | {rate:.3} |").ok(),
        None => writeln!(f, "| Correct-heading | n/a |").ok(),
    };
    writeln!(f).ok();
    write_group_table(&mut f, "Per-category", &report.per_category);
    write_group_table(&mut f, "Per-tier", &report.per_tier);
    write_group_table(&mut f, "Per-language", &report.per_language);
    writeln!(f, "### Per-query breakdown").ok();
    writeln!(f).ok();
    writeln!(
        f,
        "| ID | Query | Rank of first expected | Anti in top-5? |"
    )
    .ok();
    writeln!(f, "|---|---|---|---|").ok();
    for pq in &report.per_query {
        let rank = pq
            .first_expected_rank
            .map(|r| r.to_string())
            .unwrap_or_else(|| "—".to_string());
        let anti = match pq.anti_expected_hit_at_5 {
            Some(true) => "yes",
            Some(false) => "no",
            None => "—",
        };
        let query_truncated = if pq.query.len() > 80 {
            format!("{}…", &pq.query[..77])
        } else {
            pq.query.clone()
        };
        writeln!(
            f,
            "| {} | {} | {} | {} |",
            pq.id, query_truncated, rank, anti
        )
        .ok();
    }
    Ok(())
}

pub fn append_rerank_section(
    path: &Path,
    report: &Report,
    initial_k: usize,
    max_pair_tokens: usize,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create parent: {e}"))?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("open {}: {e}", path.display()))?;

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    writeln!(f).ok();
    writeln!(f, "## Rerank — {}", report.model_id).map_err(|e| format!("write: {e}"))?;
    writeln!(f).ok();
    writeln!(f, "- Run timestamp: {now}").ok();
    writeln!(f, "- Initial K: {initial_k}").ok();
    writeln!(f, "- Max query-document pair tokens: {max_pair_tokens}").ok();
    if let Some(stats) = report.rerank_latency_ms {
        writeln!(
            f,
            "- Median rerank latency: {:.1} ms (p90: {:.1}, max: {:.1})",
            stats.median, stats.p90, stats.max
        )
        .ok();
    }
    if let Some(stats) = report.e2e_latency_ms {
        writeln!(
            f,
            "- Median end-to-end latency: {:.1} ms (p90: {:.1}, max: {:.1})",
            stats.median, stats.p90, stats.max
        )
        .ok();
    }
    writeln!(f).ok();
    writeln!(f, "| Metric | Value |").ok();
    writeln!(f, "|---|---|").ok();
    writeln!(f, "| Recall@5 (any) | {:.3} |", report.recall_at_5_any).ok();
    writeln!(f, "| Recall@5 (all) | {:.3} |", report.recall_at_5_all).ok();
    writeln!(f, "| Recall@10 (any) | {:.3} |", report.recall_at_10_any).ok();
    writeln!(f, "| Recall@10 (all) | {:.3} |", report.recall_at_10_all).ok();
    writeln!(f, "| MRR | {:.3} |", report.mrr).ok();
    writeln!(f, "| FP-rate@5 | {:.3} |", report.fp_rate_at_5).ok();
    match report.correct_heading_rate {
        Some(rate) => writeln!(f, "| Correct-heading | {rate:.3} |").ok(),
        None => writeln!(f, "| Correct-heading | n/a |").ok(),
    };
    writeln!(f).ok();
    writeln!(f, "### Per-query breakdown").ok();
    writeln!(f).ok();
    writeln!(
        f,
        "| ID | Query | Rank pre | Rank post | Δ | Anti in top-5? |"
    )
    .ok();
    writeln!(f, "|---|---|---|---|---|---|").ok();
    for pq in &report.per_query {
        let pre = pq
            .rank_pre_rerank
            .map(|r| r.to_string())
            .unwrap_or_else(|| "—".to_string());
        let post = pq
            .rank_post_rerank
            .map(|r| r.to_string())
            .unwrap_or_else(|| "—".to_string());
        let delta = match (pq.rank_pre_rerank, pq.rank_post_rerank) {
            (Some(a), Some(b)) => {
                let d = a as i64 - b as i64;
                if d > 0 {
                    format!("+{d}")
                } else {
                    d.to_string()
                }
            }
            _ => "—".to_string(),
        };
        let anti = match pq.anti_expected_hit_at_5 {
            Some(true) => "yes",
            Some(false) => "no",
            None => "—",
        };
        let query_truncated = if pq.query.len() > 80 {
            format!("{}…", &pq.query[..77])
        } else {
            pq.query.clone()
        };
        writeln!(
            f,
            "| {} | {} | {} | {} | {} | {} |",
            pq.id, query_truncated, pre, post, delta, anti
        )
        .ok();
    }
    Ok(())
}

pub fn append_hybrid_section(
    path: &Path,
    report: &Report,
    initial_k: usize,
    rrf_k: usize,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create parent: {e}"))?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("open {}: {e}", path.display()))?;

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    writeln!(f).ok();
    writeln!(f, "## Hybrid — {}", report.model_id).map_err(|e| format!("write: {e}"))?;
    writeln!(f).ok();
    writeln!(f, "- Run timestamp: {now}").ok();
    writeln!(f, "- Initial K per retriever: {initial_k}").ok();
    writeln!(f, "- RRF k: {rrf_k}").ok();
    if let Some(stats) = report.e2e_latency_ms {
        writeln!(
            f,
            "- Median end-to-end latency: {:.1} ms (p90: {:.1}, max: {:.1})",
            stats.median, stats.p90, stats.max
        )
        .ok();
    }
    writeln!(f).ok();
    writeln!(f, "| Metric | Value |").ok();
    writeln!(f, "|---|---|").ok();
    writeln!(f, "| Recall@5 (any) | {:.3} |", report.recall_at_5_any).ok();
    writeln!(f, "| Recall@5 (all) | {:.3} |", report.recall_at_5_all).ok();
    writeln!(f, "| Recall@10 (any) | {:.3} |", report.recall_at_10_any).ok();
    writeln!(f, "| Recall@10 (all) | {:.3} |", report.recall_at_10_all).ok();
    writeln!(f, "| MRR | {:.3} |", report.mrr).ok();
    writeln!(f, "| FP-rate@5 | {:.3} |", report.fp_rate_at_5).ok();
    writeln!(f).ok();
    writeln!(f, "### Per-query breakdown").ok();
    writeln!(f).ok();
    writeln!(
        f,
        "| ID | Query | Rank of first expected | Anti in top-5? |"
    )
    .ok();
    writeln!(f, "|---|---|---|---|").ok();
    for pq in &report.per_query {
        let rank = pq
            .first_expected_rank
            .map(|r| r.to_string())
            .unwrap_or_else(|| "—".to_string());
        let anti = match pq.anti_expected_hit_at_5 {
            Some(true) => "yes",
            Some(false) => "no",
            None => "—",
        };
        let query_truncated = if pq.query.len() > 80 {
            format!("{}…", &pq.query[..77])
        } else {
            pq.query.clone()
        };
        writeln!(
            f,
            "| {} | {} | {} | {} |",
            pq.id, query_truncated, rank, anti
        )
        .ok();
    }
    Ok(())
}

pub fn append_compare_section(
    path: &Path,
    model: &str,
    initial_k: usize,
    rrf_k: usize,
    results: &[CompareQueryResult],
    summary: &CompareSummary,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create parent: {e}"))?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("open {}: {e}", path.display()))?;

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    writeln!(f).ok();
    writeln!(f, "## Compare — pure vs hybrid").map_err(|e| format!("write: {e}"))?;
    writeln!(f).ok();
    writeln!(f, "- Run timestamp: {now}").ok();
    writeln!(f, "- Model: {model}").ok();
    writeln!(f, "- Initial K: {initial_k}  RRF k: {rrf_k}").ok();
    writeln!(f).ok();

    writeln!(
        f,
        "| ID | Query | Rank pure | Rank hybrid | Δ (pure − hybrid, +ve = hybrid better) | Anti pure | Anti hybrid |"
    )
    .ok();
    writeln!(f, "|---|---|---|---|---|---|---|").ok();

    for r in results {
        let rp = r
            .rank_pure
            .map(|v| v.to_string())
            .unwrap_or_else(|| "—".to_string());
        let rh = r
            .rank_hybrid
            .map(|v| v.to_string())
            .unwrap_or_else(|| "—".to_string());
        let delta = match (r.rank_pure, r.rank_hybrid) {
            (Some(p), Some(h)) => {
                let d = p as i64 - h as i64;
                if d > 0 {
                    format!("+{d}")
                } else {
                    d.to_string()
                }
            }
            (None, Some(_)) => "+∞".to_string(),
            (Some(_), None) => "-∞".to_string(),
            (None, None) => "0".to_string(),
        };
        let ap = match r.anti_pure {
            Some(true) => "yes",
            Some(false) => "no",
            None => "—",
        };
        let ah = match r.anti_hybrid {
            Some(true) => "yes",
            Some(false) => "no",
            None => "—",
        };
        let q_trunc = if r.query_text.len() > 80 {
            format!("{}…", &r.query_text[..77])
        } else {
            r.query_text.clone()
        };
        writeln!(
            f,
            "| {} | {} | {} | {} | {} | {} | {} |",
            r.query_id, q_trunc, rp, rh, delta, ap, ah
        )
        .ok();
    }

    writeln!(f).ok();
    writeln!(f, "### Summary").ok();
    writeln!(f).ok();
    writeln!(f, "- Hybrid wins (lower rank): **{}**", summary.hybrid_wins).ok();
    writeln!(f, "- Ties: **{}**", summary.ties).ok();
    writeln!(f, "- Pure wins (lower rank): **{}**", summary.pure_wins).ok();
    writeln!(
        f,
        "- Anti improvements (hybrid drops anti pure had): **{}**",
        summary.anti_improvements
    )
    .ok();
    writeln!(
        f,
        "- Anti regressions (hybrid adds anti pure didn't have): **{}**",
        summary.anti_regressions
    )
    .ok();
    writeln!(f).ok();
    let verdict = format!(
        "**Verdict:** Hybrid wins on {} queries, loses on {}, ties on {}. Anti improvements: {}, anti regressions: {}.",
        summary.hybrid_wins,
        summary.pure_wins,
        summary.ties,
        summary.anti_improvements,
        summary.anti_regressions
    );
    writeln!(f, "{verdict}").ok();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::metrics::{PerQueryMetrics, Report};

    fn fake_report() -> Report {
        Report {
            model_id: "BGESmallENV15".to_string(),
            reranker_id: None,
            recall_at_5_any: 0.84,
            recall_at_5_all: 0.71,
            recall_at_10_any: 0.92,
            recall_at_10_all: 0.78,
            mrr: 0.61,
            fp_rate_at_5: 0.20,
            correct_heading_rate: Some(0.5),
            per_category: Vec::new(),
            per_tier: Vec::new(),
            per_language: Vec::new(),
            per_query: vec![PerQueryMetrics {
                id: "U1".to_string(),
                query: "Where does my Plex media live?".to_string(),
                first_expected_rank: Some(1),
                anti_expected_hit_at_5: None,
                rank_pre_rerank: None,
                rank_post_rerank: None,
            }],
            rerank_latency_ms: None,
            e2e_latency_ms: None,
        }
    }

    #[test]
    fn append_section_writes_expected_markdown() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("results.md");
        let build = BuildInfo {
            duration_secs: Some(612.5),
            started_at: Some("2026-07-25T20:00:00Z".to_string()),
            finished_at: Some("2026-07-25T20:10:12Z".to_string()),
            peak_rss_mb: Some(2048.0),
        };
        append_section(&path, &fake_report(), &build).expect("append");
        let text = std::fs::read_to_string(&path).expect("read");
        assert!(text.contains("## BGESmallENV15"), "header missing");
        assert!(text.contains("Recall@5 (any)"), "metrics table missing");
        assert!(text.contains("0.840"), "recall_at_5_any value missing");
        assert!(text.contains("612.5"), "build duration missing");
        assert!(
            text.contains("2026-07-25T20:10:12Z"),
            "build window missing"
        );
        assert!(text.contains("2048.0 MB"), "peak RSS missing");
        assert!(text.contains("U1"), "per-query row missing");
        assert!(
            text.contains("Where does my Plex media live?"),
            "query text missing"
        );
    }

    #[test]
    fn append_section_appends_to_existing_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("results.md");
        append_section(&path, &fake_report(), &BuildInfo::default()).expect("first");
        append_section(&path, &fake_report(), &BuildInfo::default()).expect("second");
        let text = std::fs::read_to_string(&path).expect("read");
        let occurrences = text.matches("## BGESmallENV15").count();
        assert_eq!(occurrences, 2);
    }

    fn fake_rerank_report() -> Report {
        Report {
            model_id: "NomicEmbedTextV15 + JINARerankerV2BaseMultilingual".to_string(),
            reranker_id: Some("JINARerankerV2BaseMultilingual".to_string()),
            recall_at_5_any: 1.0,
            recall_at_5_all: 0.98,
            recall_at_10_any: 1.0,
            recall_at_10_all: 0.99,
            mrr: 0.95,
            fp_rate_at_5: 0.0,
            correct_heading_rate: None,
            per_category: Vec::new(),
            per_tier: Vec::new(),
            per_language: Vec::new(),
            per_query: vec![
                PerQueryMetrics {
                    id: "U5".to_string(),
                    query: "I am travelling by plane with the baby".to_string(),
                    first_expected_rank: Some(1),
                    anti_expected_hit_at_5: Some(false),
                    rank_pre_rerank: Some(1),
                    rank_post_rerank: Some(1),
                },
                PerQueryMetrics {
                    id: "D2".to_string(),
                    query: "What's the MergerFS pool layout on BatterProx?".to_string(),
                    first_expected_rank: Some(1),
                    anti_expected_hit_at_5: None,
                    rank_pre_rerank: Some(4),
                    rank_post_rerank: Some(1),
                },
            ],
            rerank_latency_ms: Some(crate::eval::metrics::LatencyStats {
                median: 180.0,
                p90: 240.0,
                max: 300.0,
            }),
            e2e_latency_ms: Some(crate::eval::metrics::LatencyStats {
                median: 200.0,
                p90: 270.0,
                max: 340.0,
            }),
        }
    }

    #[test]
    fn append_rerank_section_writes_expected_markdown() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("results.md");
        append_rerank_section(&path, &fake_rerank_report(), 20, 512).expect("append");
        let text = std::fs::read_to_string(&path).expect("read");
        assert!(text.contains("## Rerank — NomicEmbedTextV15 + JINARerankerV2BaseMultilingual"));
        assert!(text.contains("Initial K: 20"));
        assert!(text.contains("Median rerank latency: 180.0 ms"));
        assert!(text.contains("p90: 240.0"));
        assert!(text.contains("Median end-to-end latency: 200.0 ms"));
        assert!(text.contains("| ID | Query | Rank pre | Rank post | Δ | Anti in top-5? |"));
        // D2 moved from rank 4 → 1, delta = 3
        assert!(text.contains("| D2 |"));
        assert!(text.contains("| 4 | 1 | +3 |"));
        // U5 unchanged at rank 1, delta = 0
        assert!(text.contains("| U5 |"));
        assert!(text.contains("| 1 | 1 | 0 |"));
    }
}
