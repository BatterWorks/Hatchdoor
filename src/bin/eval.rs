use hatchdoor::embed::{Embedder, FastembedEmbedder, MatryoshkaEmbedder};
use hatchdoor::rerank::{FastembedReranker, Reranker};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

/// Load a model by id, optionally reduced to `dim` dimensions via Matryoshka
/// truncation. `dim` must be applied identically at build and query time, so it
/// is threaded through every subcommand.
fn load_embedder(id: &str, dim: Option<usize>) -> Result<Arc<dyn Embedder>, String> {
    let base: Arc<dyn Embedder> = match id {
        "BGESmallENV15" => Arc::new(FastembedEmbedder::bge_small()?),
        "NomicEmbedTextV15" => Arc::new(FastembedEmbedder::nomic_v1_5()?),
        "MxbaiEmbedLargeV1" => Arc::new(FastembedEmbedder::mxbai_large()?),
        other => return Err(format!("unknown model id: {other}")),
    };
    match dim {
        Some(d) => Ok(Arc::new(MatryoshkaEmbedder::new(base, d)?)),
        None => Ok(base),
    }
}

fn load_reranker(id: &str) -> Result<Arc<dyn Reranker>, String> {
    match id {
        "JINARerankerV1TurboEn" => Ok(Arc::new(FastembedReranker::jina_v1_turbo()?)),
        "JINARerankerV2BaseMultilingual" => {
            Ok(Arc::new(FastembedReranker::jina_v2_multilingual()?))
        }
        other => Err(format!("unknown reranker id: {other}")),
    }
}

fn print_usage() {
    eprintln!(
        "usage:
  eval build --model <id> --cache <path> [--max-tokens <n>] [--overlap <n>] [--no-context]
  eval run --model <id> --cache <path> --queries <path>
  eval rerank --model <id> --cache <path> --reranker <id> --queries <path> [--initial-k <n>]
  eval hybrid --model <id> --cache <path> --queries <path> [--initial-k <n>] [--rrf-k <n>]
  eval compare --model <id> --cache <path> --queries <path> [--initial-k <n>] [--rrf-k <n>]

--dim <n> (any subcommand): reduce vectors to n dims via Matryoshka truncation.
          Must match between build and query runs on the same cache.

models:    BGESmallENV15 | NomicEmbedTextV15 | MxbaiEmbedLargeV1
rerankers: JINARerankerV1TurboEn | JINARerankerV2BaseMultilingual"
    );
}

#[derive(Debug)]
enum Cmd {
    Build {
        model: String,
        cache: PathBuf,
        opts: hatchdoor::cache::BuildOptions,
    },
    Run {
        model: String,
        cache: PathBuf,
        queries: PathBuf,
    },
    Rerank {
        model: String,
        cache: PathBuf,
        reranker: String,
        queries: PathBuf,
        initial_k: usize,
    },
    Hybrid {
        model: String,
        cache: PathBuf,
        queries: PathBuf,
        initial_k: usize,
        rrf_k: usize,
    },
    Compare {
        model: String,
        cache: PathBuf,
        queries: PathBuf,
        initial_k: usize,
        rrf_k: usize,
    },
}

fn parse_args(argv: Vec<String>) -> Result<(Cmd, Option<usize>), String> {
    let mut it = argv.into_iter().skip(1);
    let sub = it.next().ok_or_else(|| "missing subcommand".to_string())?;
    let mut model: Option<String> = None;
    let mut cache: Option<PathBuf> = None;
    let mut queries: Option<PathBuf> = None;
    let mut reranker: Option<String> = None;
    let mut initial_k: Option<usize> = None;
    let mut rrf_k: Option<usize> = None;
    let mut max_tokens: Option<usize> = None;
    let mut overlap: Option<usize> = None;
    let mut no_context = false;
    let mut dim: Option<usize> = None;
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--model" => model = Some(it.next().ok_or("missing value for --model")?),
            "--cache" => cache = Some(PathBuf::from(it.next().ok_or("missing value for --cache")?)),
            "--max-tokens" => {
                let raw = it.next().ok_or("missing value for --max-tokens")?;
                max_tokens = Some(
                    raw.parse::<usize>()
                        .map_err(|e| format!("invalid --max-tokens {raw}: {e}"))?,
                );
            }
            "--overlap" => {
                let raw = it.next().ok_or("missing value for --overlap")?;
                overlap = Some(
                    raw.parse::<usize>()
                        .map_err(|e| format!("invalid --overlap {raw}: {e}"))?,
                );
            }
            "--no-context" => no_context = true,
            "--dim" => {
                let raw = it.next().ok_or("missing value for --dim")?;
                dim = Some(
                    raw.parse::<usize>()
                        .map_err(|e| format!("invalid --dim {raw}: {e}"))?,
                );
            }
            "--queries" => {
                queries = Some(PathBuf::from(
                    it.next().ok_or("missing value for --queries")?,
                ))
            }
            "--reranker" => reranker = Some(it.next().ok_or("missing value for --reranker")?),
            "--initial-k" => {
                let raw = it.next().ok_or("missing value for --initial-k")?;
                initial_k = Some(
                    raw.parse::<usize>()
                        .map_err(|e| format!("invalid --initial-k {raw}: {e}"))?,
                );
            }
            "--rrf-k" => {
                let raw = it.next().ok_or("missing value for --rrf-k")?;
                rrf_k = Some(
                    raw.parse::<usize>()
                        .map_err(|e| format!("invalid --rrf-k {raw}: {e}"))?,
                );
            }
            other => return Err(format!("unknown flag: {other}")),
        }
    }
    let model = model.ok_or("missing --model")?;
    let cache = cache.ok_or("missing --cache")?;
    let cmd = match sub.as_str() {
        "build" => {
            let mut opts = hatchdoor::cache::BuildOptions::default();
            if let Some(m) = max_tokens {
                opts.chunk.max_tokens = m;
            }
            if let Some(o) = overlap {
                opts.chunk.overlap_tokens = o;
            }
            opts.context = !no_context;
            if opts.chunk.overlap_tokens >= opts.chunk.max_tokens {
                return Err(format!(
                    "--overlap ({}) must be smaller than --max-tokens ({})",
                    opts.chunk.overlap_tokens, opts.chunk.max_tokens
                ));
            }
            Ok(Cmd::Build { model, cache, opts })
        }
        "run" => {
            let queries = queries.ok_or("missing --queries")?;
            Ok(Cmd::Run {
                model,
                cache,
                queries,
            })
        }
        "rerank" => {
            let queries = queries.ok_or("missing --queries")?;
            let reranker = reranker.ok_or("missing --reranker")?;
            let initial_k = initial_k.unwrap_or(20);
            Ok(Cmd::Rerank {
                model,
                cache,
                reranker,
                queries,
                initial_k,
            })
        }
        "hybrid" => {
            let queries = queries.ok_or("missing --queries")?;
            let initial_k = initial_k.unwrap_or(20);
            let rrf_k = rrf_k.unwrap_or(60);
            Ok(Cmd::Hybrid {
                model,
                cache,
                queries,
                initial_k,
                rrf_k,
            })
        }
        "compare" => {
            let queries = queries.ok_or("missing --queries")?;
            let initial_k = initial_k.unwrap_or(20);
            let rrf_k = rrf_k.unwrap_or(60);
            Ok(Cmd::Compare {
                model,
                cache,
                queries,
                initial_k,
                rrf_k,
            })
        }
        other => Err(format!("unknown subcommand: {other}")),
    }?;
    Ok((cmd, dim))
}

fn main() -> ExitCode {
    dotenvy::dotenv().ok();
    let (cmd, dim) = match parse_args(std::env::args().collect()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            print_usage();
            return ExitCode::from(2);
        }
    };
    match cmd {
        Cmd::Build { model, cache, opts } => {
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
                )
                .init();

            let vault_path = std::env::var("VAULT_PATH").unwrap_or_else(|_| "./vault".to_string());
            let vault_path = std::path::PathBuf::from(vault_path);

            let embedder = match load_embedder(&model, dim) {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::from(1);
                }
            };

            if cache.exists() {
                eprintln!(
                    "error: cache file already exists at {}. Delete it before rebuilding.",
                    cache.display()
                );
                return ExitCode::from(1);
            }

            let sqlite = match hatchdoor::cache::SqliteCache::open(&cache, embedder.embedding_dim())
            {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("error opening cache: {e}");
                    return ExitCode::from(1);
                }
            };

            let index = match hatchdoor::vault::VaultIndex::build(&vault_path) {
                Ok(i) => i,
                Err(e) => {
                    eprintln!("error building vault index: {e}");
                    return ExitCode::from(1);
                }
            };

            let started = std::time::Instant::now();
            if let Err(e) = sqlite.replace_from_index_with_options_stamped(
                &index,
                embedder.as_ref(),
                &model,
                &opts,
            ) {
                eprintln!("error populating cache: {e}");
                return ExitCode::from(1);
            }
            let elapsed = started.elapsed();
            println!(
                "build config: max_tokens={} overlap={} context={}",
                opts.chunk.max_tokens, opts.chunk.overlap_tokens, opts.context
            );
            println!(
                "build complete: model={model} cache={} elapsed={:.1}s",
                cache.display(),
                elapsed.as_secs_f64()
            );
            ExitCode::SUCCESS
        }
        Cmd::Run {
            model,
            cache,
            queries,
        } => {
            let embedder = match load_embedder(&model, dim) {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::from(1);
                }
            };

            if !cache.exists() {
                eprintln!(
                    "error: cache {} does not exist. Run `eval build` first.",
                    cache.display()
                );
                return ExitCode::from(1);
            }

            let sqlite = match hatchdoor::cache::SqliteCache::open(&cache, embedder.embedding_dim())
            {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("error opening cache: {e}");
                    return ExitCode::from(1);
                }
            };

            let stamped = sqlite.get_metadata("embedder_id").unwrap_or(None);
            match stamped.as_deref() {
                Some(id) if id == model => {}
                Some(id) => {
                    eprintln!(
                        "error: cache was built with embedder {id} but --model is {model}. \
                         Rebuild the cache or pass --model {id}."
                    );
                    return ExitCode::from(1);
                }
                None => {
                    eprintln!("error: cache has no embedder_id stamp; rebuild it.");
                    return ExitCode::from(1);
                }
            }

            let build_dur: Option<f64> = sqlite
                .get_metadata("build_duration_secs")
                .ok()
                .flatten()
                .and_then(|s| s.parse::<f64>().ok());

            let qs = match hatchdoor::eval::query::load_jsonl(&queries) {
                Ok(qs) => qs,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::from(1);
                }
            };

            let mut results = Vec::with_capacity(qs.len());
            for q in &qs {
                match sqlite.semantic_search(embedder.as_ref(), &q.query, 10) {
                    Ok(hits) => {
                        let top_k: Vec<String> = hits.iter().map(|h| h.note_slug.clone()).collect();
                        let top_k_headings: Vec<Option<String>> =
                            hits.iter().map(|h| h.heading_path.clone()).collect();
                        results.push(hatchdoor::eval::metrics::QueryResult {
                            query_id: q.id.clone(),
                            top_k,
                            top_k_headings,
                        });
                    }
                    Err(e) => {
                        eprintln!("warning: query {} failed: {e}", q.id);
                        results.push(hatchdoor::eval::metrics::QueryResult {
                            query_id: q.id.clone(),
                            top_k: Vec::new(),
                            top_k_headings: Vec::new(),
                        });
                    }
                }
            }

            let report = hatchdoor::eval::metrics::aggregate(&model, &qs, &results);

            println!("\nmodel: {}", report.model_id);
            println!("queries: {}", qs.len());
            println!(
                "Recall@5  (any/all): {:.3} / {:.3}",
                report.recall_at_5_any, report.recall_at_5_all
            );
            println!(
                "Recall@10 (any/all): {:.3} / {:.3}",
                report.recall_at_10_any, report.recall_at_10_all
            );
            println!("MRR:                 {:.3}", report.mrr);
            println!("FP-rate@5:           {:.3}", report.fp_rate_at_5);
            match report.correct_heading_rate {
                Some(rate) => println!("Correct-heading:     {rate:.3}"),
                None => println!("Correct-heading:     n/a (no heading-scoped queries)"),
            }
            for group in &report.per_category {
                println!(
                    "  [category {:>14}] n={:<3} R@5={:.3} R@10={:.3} MRR={:.3} heading={}",
                    group.label,
                    group.n,
                    group.recall_at_5_any,
                    group.recall_at_10_any,
                    group.mrr,
                    group
                        .correct_heading_rate
                        .map(|r| format!("{r:.3}"))
                        .unwrap_or_else(|| "n/a".to_string()),
                );
            }
            for group in &report.per_tier {
                println!(
                    "  [tier     {:>14}] n={:<3} R@5={:.3} R@10={:.3} MRR={:.3} heading={}",
                    group.label,
                    group.n,
                    group.recall_at_5_any,
                    group.recall_at_10_any,
                    group.mrr,
                    group
                        .correct_heading_rate
                        .map(|r| format!("{r:.3}"))
                        .unwrap_or_else(|| "n/a".to_string()),
                );
            }
            for group in &report.per_language {
                println!(
                    "  [language {:>14}] n={:<3} R@5={:.3} R@10={:.3} MRR={:.3}",
                    group.label, group.n, group.recall_at_5_any, group.recall_at_10_any, group.mrr,
                );
            }

            let report_path = std::path::PathBuf::from("eval/results.md");
            if let Err(e) =
                hatchdoor::eval::report::append_section(&report_path, &report, build_dur)
            {
                eprintln!("warning: failed to write report: {e}");
            } else {
                println!("\nappended to {}", report_path.display());
            }
            ExitCode::SUCCESS
        }
        Cmd::Rerank {
            model,
            cache,
            reranker: reranker_id,
            queries,
            initial_k,
        } => {
            if !cache.exists() {
                eprintln!(
                    "error: cache {} does not exist. Run `eval build` first.",
                    cache.display()
                );
                return ExitCode::from(1);
            }
            let embedder = match load_embedder(&model, dim) {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("error loading embedder: {e}");
                    return ExitCode::from(1);
                }
            };
            let reranker = match load_reranker(&reranker_id) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error loading reranker: {e}");
                    return ExitCode::from(1);
                }
            };
            let sqlite = match hatchdoor::cache::SqliteCache::open(&cache, embedder.embedding_dim())
            {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("error opening cache: {e}");
                    return ExitCode::from(1);
                }
            };
            let qs = match hatchdoor::eval::query::load_jsonl(&queries) {
                Ok(q) => q,
                Err(e) => {
                    eprintln!("error loading queries: {e}");
                    return ExitCode::from(1);
                }
            };

            let results = match hatchdoor::eval::rerank_runner::run_rerank_eval(
                &sqlite,
                embedder.as_ref(),
                reranker.as_ref(),
                &qs,
                initial_k,
            ) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error running rerank eval: {e}");
                    return ExitCode::from(1);
                }
            };

            let run_id = format!("{} + {}", model, reranker.id());
            let report =
                hatchdoor::eval::metrics::aggregate_rerank(&run_id, reranker.id(), &qs, &results);

            println!(
                "rerank complete: model={model} reranker={} initial_k={initial_k}",
                reranker.id()
            );
            println!("  Recall@5  (any): {:.3}", report.recall_at_5_any);
            println!("  Recall@5  (all): {:.3}", report.recall_at_5_all);
            println!("  Recall@10 (any): {:.3}", report.recall_at_10_any);
            println!("  Recall@10 (all): {:.3}", report.recall_at_10_all);
            println!("  MRR           : {:.3}", report.mrr);
            println!("  FP-rate@5     : {:.3}", report.fp_rate_at_5);
            if let Some(s) = report.rerank_latency_ms {
                println!(
                    "  rerank lat ms : median={:.1} p90={:.1} max={:.1}",
                    s.median, s.p90, s.max
                );
            }
            if let Some(s) = report.e2e_latency_ms {
                println!(
                    "  e2e    lat ms : median={:.1} p90={:.1} max={:.1}",
                    s.median, s.p90, s.max
                );
            }

            let results_md = std::path::PathBuf::from("eval/results.md");
            if let Err(e) =
                hatchdoor::eval::report::append_rerank_section(&results_md, &report, initial_k)
            {
                eprintln!(
                    "warning: failed to append section to {}: {e}",
                    results_md.display()
                );
            }
            ExitCode::SUCCESS
        }
        Cmd::Compare {
            model,
            cache,
            queries,
            initial_k,
            rrf_k,
        } => {
            if !cache.exists() {
                eprintln!(
                    "error: cache {} does not exist. Run `eval build` first.",
                    cache.display()
                );
                return ExitCode::from(1);
            }
            let embedder = match load_embedder(&model, dim) {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("error loading embedder: {e}");
                    return ExitCode::from(1);
                }
            };
            let sqlite = match hatchdoor::cache::SqliteCache::open(&cache, embedder.embedding_dim())
            {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("error opening cache: {e}");
                    return ExitCode::from(1);
                }
            };
            let qs = match hatchdoor::eval::query::load_jsonl(&queries) {
                Ok(q) => q,
                Err(e) => {
                    eprintln!("error loading queries: {e}");
                    return ExitCode::from(1);
                }
            };

            let (compare_results, summary) = match hatchdoor::eval::compare_runner::run_compare_eval(
                &sqlite,
                embedder.as_ref(),
                &qs,
                initial_k,
                rrf_k,
            ) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error running compare eval: {e}");
                    return ExitCode::from(1);
                }
            };

            // Print per-query table to stdout
            println!(
                "| {:<4} | {:<60} | {:^10} | {:^12} | {:^40} | {:^10} | {:^12} |",
                "ID",
                "Query",
                "Rank pure",
                "Rank hybrid",
                "Δ (pure−hybrid, +ve=hybrid better)",
                "Anti pure",
                "Anti hybrid"
            );
            println!("|---|---|---|---|---|---|---|");
            for r in &compare_results {
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
                let q_trunc = if r.query_text.len() > 60 {
                    format!("{}…", &r.query_text[..57])
                } else {
                    r.query_text.clone()
                };
                println!(
                    "| {} | {} | {} | {} | {} | {} | {} |",
                    r.query_id, q_trunc, rp, rh, delta, ap, ah
                );
            }
            println!();
            println!(
                "Hybrid wins: {}  |  Ties: {}  |  Pure wins: {}",
                summary.hybrid_wins, summary.ties, summary.pure_wins
            );
            println!(
                "Anti improvements: {}  |  Anti regressions: {}",
                summary.anti_improvements, summary.anti_regressions
            );
            println!(
                "Verdict: Hybrid wins on {} queries, loses on {}, ties on {}. Anti improvements: {}, anti regressions: {}.",
                summary.hybrid_wins,
                summary.pure_wins,
                summary.ties,
                summary.anti_improvements,
                summary.anti_regressions
            );

            let results_md = std::path::PathBuf::from("eval/results.md");
            if let Err(e) = hatchdoor::eval::report::append_compare_section(
                &results_md,
                &model,
                initial_k,
                rrf_k,
                &compare_results,
                &summary,
            ) {
                eprintln!(
                    "warning: failed to append section to {}: {e}",
                    results_md.display()
                );
            } else {
                println!("\nappended to {}", results_md.display());
            }
            ExitCode::SUCCESS
        }
        Cmd::Hybrid {
            model,
            cache,
            queries,
            initial_k,
            rrf_k,
        } => {
            if !cache.exists() {
                eprintln!(
                    "error: cache {} does not exist. Run `eval build` first.",
                    cache.display()
                );
                return ExitCode::from(1);
            }
            let embedder = match load_embedder(&model, dim) {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("error loading embedder: {e}");
                    return ExitCode::from(1);
                }
            };
            let sqlite = match hatchdoor::cache::SqliteCache::open(&cache, embedder.embedding_dim())
            {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("error opening cache: {e}");
                    return ExitCode::from(1);
                }
            };
            let qs = match hatchdoor::eval::query::load_jsonl(&queries) {
                Ok(q) => q,
                Err(e) => {
                    eprintln!("error loading queries: {e}");
                    return ExitCode::from(1);
                }
            };

            let hybrid = match hatchdoor::eval::hybrid_runner::run_hybrid_eval(
                &sqlite,
                embedder.as_ref(),
                &qs,
                initial_k,
                rrf_k,
                10,
            ) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error running hybrid eval: {e}");
                    return ExitCode::from(1);
                }
            };

            let results: Vec<hatchdoor::eval::metrics::QueryResult> =
                hybrid.iter().map(|h| h.query_result.clone()).collect();
            let run_id = format!("Hybrid ({} + FTS RRF)", model);
            let mut report = hatchdoor::eval::metrics::aggregate(&run_id, &qs, &results);
            let latencies: Vec<f64> = hybrid.iter().map(|h| h.latency_ms).collect();
            if !latencies.is_empty() {
                report.e2e_latency_ms = Some(hatchdoor::eval::metrics::LatencyStats::from_samples(
                    &latencies,
                ));
            }

            println!("hybrid complete: model={model} initial_k={initial_k} rrf_k={rrf_k}");
            println!("  Recall@5  (any): {:.3}", report.recall_at_5_any);
            println!("  Recall@5  (all): {:.3}", report.recall_at_5_all);
            println!("  Recall@10 (any): {:.3}", report.recall_at_10_any);
            println!("  Recall@10 (all): {:.3}", report.recall_at_10_all);
            println!("  MRR           : {:.3}", report.mrr);
            println!("  FP-rate@5     : {:.3}", report.fp_rate_at_5);
            if let Some(s) = report.e2e_latency_ms {
                println!(
                    "  e2e    lat ms : median={:.3} p90={:.3} max={:.3}",
                    s.median, s.p90, s.max
                );
            }

            let results_md = std::path::PathBuf::from("eval/results.md");
            if let Err(e) = hatchdoor::eval::report::append_hybrid_section(
                &results_md,
                &report,
                initial_k,
                rrf_k,
            ) {
                eprintln!(
                    "warning: failed to append section to {}: {e}",
                    results_md.display()
                );
            }
            ExitCode::SUCCESS
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_build_command() {
        let (cmd, _dim) = parse_args(argv(&[
            "eval",
            "build",
            "--model",
            "BGESmallENV15",
            "--cache",
            "/tmp/x.db",
        ]))
        .expect("parse");
        match cmd {
            Cmd::Build {
                model, cache, opts, ..
            } => {
                assert_eq!(model, "BGESmallENV15");
                assert_eq!(cache, PathBuf::from("/tmp/x.db"));
                // Defaults when no build flags are passed.
                assert_eq!(opts.chunk.max_tokens, 800);
                assert_eq!(opts.chunk.overlap_tokens, 50);
                assert!(opts.context);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn parses_run_command_with_queries() {
        let (cmd, _dim) = parse_args(argv(&[
            "eval",
            "run",
            "--model",
            "X",
            "--cache",
            "/c",
            "--queries",
            "/q",
        ]))
        .expect("parse");
        match cmd {
            Cmd::Run {
                model,
                cache,
                queries,
            } => {
                assert_eq!(model, "X");
                assert_eq!(cache, PathBuf::from("/c"));
                assert_eq!(queries, PathBuf::from("/q"));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn parses_compare_command() {
        let (cmd, _dim) = parse_args(argv(&[
            "eval",
            "compare",
            "--model",
            "NomicEmbedTextV15",
            "--cache",
            "/c.db",
            "--queries",
            "/q.jsonl",
            "--initial-k",
            "20",
            "--rrf-k",
            "60",
        ]))
        .expect("parse");
        match cmd {
            Cmd::Compare {
                model,
                cache,
                queries,
                initial_k,
                rrf_k,
            } => {
                assert_eq!(model, "NomicEmbedTextV15");
                assert_eq!(cache, PathBuf::from("/c.db"));
                assert_eq!(queries, PathBuf::from("/q.jsonl"));
                assert_eq!(initial_k, 20);
                assert_eq!(rrf_k, 60);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn parses_compare_command_defaults() {
        let (cmd, _dim) = parse_args(argv(&[
            "eval",
            "compare",
            "--model",
            "NomicEmbedTextV15",
            "--cache",
            "/c.db",
            "--queries",
            "/q.jsonl",
        ]))
        .expect("parse");
        match cmd {
            Cmd::Compare {
                initial_k, rrf_k, ..
            } => {
                assert_eq!(initial_k, 20);
                assert_eq!(rrf_k, 60);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn rejects_unknown_subcommand() {
        let err = parse_args(argv(&["eval", "wat", "--model", "x", "--cache", "/y"])).unwrap_err();
        assert!(err.contains("unknown subcommand"));
    }

    #[test]
    fn parses_rerank_command() {
        let (cmd, _dim) = parse_args(argv(&[
            "eval",
            "rerank",
            "--model",
            "NomicEmbedTextV15",
            "--cache",
            "/c.db",
            "--reranker",
            "JINARerankerV1TurboEn",
            "--queries",
            "/q.jsonl",
            "--initial-k",
            "30",
        ]))
        .expect("parse");
        match cmd {
            Cmd::Rerank {
                model,
                cache,
                reranker,
                queries,
                initial_k,
            } => {
                assert_eq!(model, "NomicEmbedTextV15");
                assert_eq!(cache, PathBuf::from("/c.db"));
                assert_eq!(reranker, "JINARerankerV1TurboEn");
                assert_eq!(queries, PathBuf::from("/q.jsonl"));
                assert_eq!(initial_k, 30);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn parses_rerank_command_default_initial_k() {
        let (cmd, _dim) = parse_args(argv(&[
            "eval",
            "rerank",
            "--model",
            "NomicEmbedTextV15",
            "--cache",
            "/c.db",
            "--reranker",
            "JINARerankerV2BaseMultilingual",
            "--queries",
            "/q.jsonl",
        ]))
        .expect("parse");
        match cmd {
            Cmd::Rerank { initial_k, .. } => assert_eq!(initial_k, 20),
            _ => panic!("wrong variant"),
        }
    }
}
