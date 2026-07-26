use hatchdoor::embed::{
    Embedder, FastembedEmbedder, MatryoshkaEmbedder, NomicV2Embedder, Qwen3Embedder,
};
use hatchdoor::rerank::{FastembedReranker, Reranker};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

/// Load a model by id, optionally reduced to `dim` dimensions via Matryoshka
/// truncation. `dim` must be applied identically at build and query time, so it
/// is threaded through every subcommand.
fn load_embedder(id: &str, dim: Option<usize>) -> Result<Arc<dyn Embedder>, String> {
    let base: Arc<dyn Embedder> = match id {
        // Control (current production).
        "NomicEmbedTextV15" => Arc::new(FastembedEmbedder::nomic_v1_5()?),
        // English-only floor (native ONNX).
        "GTEBaseENV15" => Arc::new(FastembedEmbedder::gte_base_en()?),
        // Midsize multilingual, Candle backend.
        "NomicEmbedTextV2Moe" => Arc::new(NomicV2Embedder::load()?),
        // Large multilingual ceiling, Candle backend (native 1024-dim; sweep at --dim 512).
        "Qwen3Embedding0_6B" => Arc::new(Qwen3Embedder::load()?),
        // Multilingual 4-bit ONNX challenger. Gemma terms: benchmark-only for now.
        "EmbeddingGemma300MQ4" => Arc::new(FastembedEmbedder::embedding_gemma_300m_q4()?),
        // Second midsize multilingual, user-defined ONNX (not a native enum model).
        "SnowflakeArcticEmbedMV2" => Arc::new(FastembedEmbedder::arctic_m_v2()?),
        other => return Err(format!("unknown model id: {other}")),
    };
    match dim {
        Some(d) => Ok(Arc::new(MatryoshkaEmbedder::new(base, d)?)),
        None => Ok(base),
    }
}

fn load_reranker(id: &str, max_pair_tokens: usize) -> Result<Arc<dyn Reranker>, String> {
    match id {
        "JINARerankerV1TurboEn" => Ok(Arc::new(
            FastembedReranker::jina_v1_turbo_with_max_pair_tokens(max_pair_tokens)?,
        )),
        "JINARerankerV2BaseMultilingual" => Ok(Arc::new(
            FastembedReranker::jina_v2_multilingual_with_max_pair_tokens(max_pair_tokens)?,
        )),
        "BGERerankerV2M3" => Ok(Arc::new(
            FastembedReranker::bge_reranker_v2_m3_with_max_pair_tokens(max_pair_tokens)?,
        )),
        "GTEMultilingualRerankerBase" => Ok(Arc::new(
            FastembedReranker::gte_multilingual_base_with_max_pair_tokens(max_pair_tokens)?,
        )),
        other => Err(format!("unknown reranker id: {other}")),
    }
}

/// Peak resident set size of this process so far, in MiB. On Linux `ru_maxrss`
/// is reported in kibibytes. Returns 0.0 if the syscall fails.
fn peak_rss_mb() -> f64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
        return 0.0;
    }
    let usage = unsafe { usage.assume_init() };
    usage.ru_maxrss as f64 / 1024.0
}

fn print_usage() {
    eprintln!(
        "usage:
  eval build --model <id> --cache <path> [--max-tokens <n>] [--overlap <n>] [--batch-size <n>] [--no-context]
  eval run --model <id> --cache <path> --queries <path>
  eval rerank --model <id> --cache <path> --reranker <id> --queries <path> [--initial-k <n>] [--max-pair-tokens <n>]
  eval hybrid --model <id> --cache <path> --queries <path> [--initial-k <n>] [--rrf-k <n>]
  eval compare --model <id> --cache <path> --queries <path> [--initial-k <n>] [--rrf-k <n>]
  eval prefetch   (loads every sweep model + smoke-tests one embedding each)

--dim <n> (any subcommand): reduce vectors to n dims via Matryoshka truncation.
          Must match between build and query runs on the same cache.

models:    NomicEmbedTextV15 (control) | GTEBaseENV15 | NomicEmbedTextV2Moe
	           | Qwen3Embedding0_6B | EmbeddingGemma300MQ4 | SnowflakeArcticEmbedMV2
rerankers: JINARerankerV1TurboEn | JINARerankerV2BaseMultilingual
	   | BGERerankerV2M3 | GTEMultilingualRerankerBase

--dim per model in the sweep: Nomic v2 & Arctic at 768/256; Qwen3 (native 1024) at 512."
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
        max_pair_tokens: usize,
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
    /// Load every sweep model and embed a probe string, validating each produces
    /// a finite, correctly-sized vector. Warms the HF cache and fail-fast checks
    /// the wiring before the full sweep.
    Prefetch,
}

/// The locked benchmark model set (see the eval-model-benchmark-set design).
const SWEEP_MODELS: &[&str] = &[
    "NomicEmbedTextV15",
    "GTEBaseENV15",
    "NomicEmbedTextV2Moe",
    "Qwen3Embedding0_6B",
    "EmbeddingGemma300MQ4",
    "SnowflakeArcticEmbedMV2",
];

fn parse_args(argv: Vec<String>) -> Result<(Cmd, Option<usize>), String> {
    let mut it = argv.into_iter().skip(1);
    let sub = it.next().ok_or_else(|| "missing subcommand".to_string())?;
    let mut model: Option<String> = None;
    let mut cache: Option<PathBuf> = None;
    let mut queries: Option<PathBuf> = None;
    let mut reranker: Option<String> = None;
    let mut initial_k: Option<usize> = None;
    let mut max_pair_tokens: Option<usize> = None;
    let mut rrf_k: Option<usize> = None;
    let mut max_tokens: Option<usize> = None;
    let mut overlap: Option<usize> = None;
    let mut batch_size: Option<usize> = None;
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
            "--batch-size" => {
                let raw = it.next().ok_or("missing value for --batch-size")?;
                let parsed = raw
                    .parse::<usize>()
                    .map_err(|e| format!("invalid --batch-size {raw}: {e}"))?;
                if parsed == 0 {
                    return Err("--batch-size must be at least 1".to_string());
                }
                batch_size = Some(parsed);
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
            "--max-pair-tokens" => {
                let raw = it.next().ok_or("missing value for --max-pair-tokens")?;
                let parsed = raw
                    .parse::<usize>()
                    .map_err(|e| format!("invalid --max-pair-tokens {raw}: {e}"))?;
                if parsed == 0 {
                    return Err("--max-pair-tokens must be at least 1".to_string());
                }
                max_pair_tokens = Some(parsed);
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
    // Prefetch needs neither --model nor --cache; it iterates the whole set.
    if sub == "prefetch" {
        return Ok((Cmd::Prefetch, dim));
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
            if let Some(n) = batch_size {
                opts.embedding_batch_size = n;
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
            let max_pair_tokens = max_pair_tokens.unwrap_or(512);
            Ok(Cmd::Rerank {
                model,
                cache,
                reranker,
                queries,
                initial_k,
                max_pair_tokens,
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

            let build_started_at = chrono::Utc::now();
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
            let build_finished_at = chrono::Utc::now();
            let peak_rss_mb = peak_rss_mb();

            // Persist wall-clock build window + peak memory alongside the
            // duration so the sweep can report them per cache. `ru_maxrss` is the
            // process high-water mark, so it captures the model load + embedding
            // peak — i.e. whether this model fits in the box's memory.
            let started_iso = build_started_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            let finished_iso = build_finished_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            // Also stamp the build config so `eval run` can label each results
            // section — the report header is otherwise just the model id, which
            // is identical across a model's chunk/context/dim cells.
            let dim_str = dim
                .map(|d| d.to_string())
                .unwrap_or_else(|| "native".to_string());
            for (k, v) in [
                ("build_started_at", started_iso.clone()),
                ("build_finished_at", finished_iso.clone()),
                ("build_peak_rss_mb", format!("{peak_rss_mb:.1}")),
                ("build_max_tokens", opts.chunk.max_tokens.to_string()),
                ("build_overlap", opts.chunk.overlap_tokens.to_string()),
                ("build_context", opts.context.to_string()),
                ("build_batch_size", opts.embedding_batch_size.to_string()),
                ("build_dim", dim_str),
            ] {
                if let Err(e) = sqlite.set_metadata(k, &v) {
                    eprintln!("warning: failed to stamp {k}: {e}");
                }
            }

            println!(
                "build config: max_tokens={} overlap={} batch_size={} context={}",
                opts.chunk.max_tokens,
                opts.chunk.overlap_tokens,
                opts.embedding_batch_size,
                opts.context
            );
            println!(
                "build complete: model={model} cache={} elapsed={:.1}s peak_rss={peak_rss_mb:.1}MB",
                cache.display(),
                elapsed.as_secs_f64()
            );
            println!("build window: {started_iso} → {finished_iso}");
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

            let meta_str = |key: &str| sqlite.get_metadata(key).ok().flatten();
            let meta_f64 = |key: &str| meta_str(key).and_then(|s| s.parse::<f64>().ok());
            let build_info = hatchdoor::eval::report::BuildInfo {
                duration_secs: meta_f64("build_duration_secs"),
                started_at: meta_str("build_started_at"),
                finished_at: meta_str("build_finished_at"),
                peak_rss_mb: meta_f64("build_peak_rss_mb"),
            };

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

            let mut report = hatchdoor::eval::metrics::aggregate(&model, &qs, &results);

            // Label the report section with the build config read back from the
            // cache, so a model's six chunk/context/dim cells are distinguishable
            // in results.md instead of sharing one `## <model>` header.
            if let (Some(mt), Some(ov)) = (meta_str("build_max_tokens"), meta_str("build_overlap"))
            {
                let ctx = match meta_str("build_context").as_deref() {
                    Some("false") => "off",
                    _ => "on",
                };
                let d = meta_str("build_dim").unwrap_or_else(|| "native".to_string());
                let batch = meta_str("build_batch_size").unwrap_or_else(|| "1".to_string());
                let report_model = if model == "EmbeddingGemma300MQ4" {
                    "EmbeddingGemma300MQ4 · retrieval-format v1"
                } else {
                    &model
                };
                report.model_id = format!(
                    "{report_model} — chunk {mt}/{ov} · ctx {ctx} · dim {d} · batch {batch}"
                );
            }

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
                hatchdoor::eval::report::append_section(&report_path, &report, &build_info)
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
            max_pair_tokens,
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
            let reranker = match load_reranker(&reranker_id, max_pair_tokens) {
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

            let run_id = format!(
                "{} + {} · pair max {}",
                model,
                reranker.id(),
                max_pair_tokens
            );
            let report =
                hatchdoor::eval::metrics::aggregate_rerank(&run_id, reranker.id(), &qs, &results);

            println!(
                "rerank complete: model={model} reranker={} initial_k={initial_k} max_pair_tokens={max_pair_tokens}",
                reranker.id(),
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
            if let Err(e) = hatchdoor::eval::report::append_rerank_section(
                &results_md,
                &report,
                initial_k,
                max_pair_tokens,
            ) {
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
        Cmd::Prefetch => {
            use std::io::Write;
            let probe = vec!["search test".to_string()];
            let mut failures = 0usize;
            for id in SWEEP_MODELS {
                print!("{id}: loading… ");
                std::io::stdout().flush().ok();
                match load_embedder(id, dim) {
                    Ok(embedder) => match embedder.embed(&probe) {
                        Ok(vecs) if !vecs.is_empty() => {
                            let v = &vecs[0];
                            let expected = embedder.embedding_dim();
                            let dim_ok = v.len() == expected;
                            let finite = v.iter().all(|x| x.is_finite());
                            let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
                            let ok = dim_ok && finite;
                            if !ok {
                                failures += 1;
                            }
                            println!(
                                "dim={} (expected {expected}) norm={norm:.4} finite={finite} => {}",
                                v.len(),
                                if ok { "OK" } else { "FAIL" }
                            );
                        }
                        Ok(_) => {
                            failures += 1;
                            println!("FAIL (empty output)");
                        }
                        Err(e) => {
                            failures += 1;
                            println!("FAIL (embed: {e})");
                        }
                    },
                    Err(e) => {
                        failures += 1;
                        println!("FAIL (load: {e})");
                    }
                }
            }
            if failures == 0 {
                println!(
                    "\nAll {} models loaded and produced valid embeddings.",
                    SWEEP_MODELS.len()
                );
                ExitCode::SUCCESS
            } else {
                eprintln!("\n{failures} model(s) failed.");
                ExitCode::from(1)
            }
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
                assert_eq!(opts.embedding_batch_size, 1);
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
                max_pair_tokens,
            } => {
                assert_eq!(model, "NomicEmbedTextV15");
                assert_eq!(cache, PathBuf::from("/c.db"));
                assert_eq!(reranker, "JINARerankerV1TurboEn");
                assert_eq!(queries, PathBuf::from("/q.jsonl"));
                assert_eq!(initial_k, 30);
                assert_eq!(max_pair_tokens, 512);
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
            Cmd::Rerank {
                initial_k,
                max_pair_tokens,
                ..
            } => {
                assert_eq!(initial_k, 20);
                assert_eq!(max_pair_tokens, 512);
            }
            _ => panic!("wrong variant"),
        }
    }
}
