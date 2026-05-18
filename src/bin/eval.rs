use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use hatchdoor::embed::{Embedder, FastembedEmbedder};

fn load_embedder(id: &str) -> Result<Arc<dyn Embedder>, String> {
    match id {
        "BGESmallENV15" => Ok(Arc::new(FastembedEmbedder::bge_small()?)),
        "NomicEmbedTextV15" => Ok(Arc::new(FastembedEmbedder::nomic_v1_5()?)),
        "MxbaiEmbedLargeV1" => Ok(Arc::new(FastembedEmbedder::mxbai_large()?)),
        other => Err(format!("unknown model id: {other}")),
    }
}

fn print_usage() {
    eprintln!(
        "usage:
  eval build --model <id> --cache <path>
  eval run --model <id> --cache <path> --queries <path>

models: BGESmallENV15 | NomicEmbedTextV15 | MxbaiEmbedLargeV1"
    );
}

#[derive(Debug)]
enum Cmd {
    Build { model: String, cache: PathBuf },
    Run { model: String, cache: PathBuf, queries: PathBuf },
}

fn parse_args(argv: Vec<String>) -> Result<Cmd, String> {
    let mut it = argv.into_iter().skip(1);
    let sub = it.next().ok_or_else(|| "missing subcommand".to_string())?;
    let mut model: Option<String> = None;
    let mut cache: Option<PathBuf> = None;
    let mut queries: Option<PathBuf> = None;
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--model" => model = Some(it.next().ok_or("missing value for --model")?),
            "--cache" => cache = Some(PathBuf::from(it.next().ok_or("missing value for --cache")?)),
            "--queries" => queries = Some(PathBuf::from(it.next().ok_or("missing value for --queries")?)),
            other => return Err(format!("unknown flag: {other}")),
        }
    }
    let model = model.ok_or("missing --model")?;
    let cache = cache.ok_or("missing --cache")?;
    match sub.as_str() {
        "build" => Ok(Cmd::Build { model, cache }),
        "run" => {
            let queries = queries.ok_or("missing --queries")?;
            Ok(Cmd::Run { model, cache, queries })
        }
        other => Err(format!("unknown subcommand: {other}")),
    }
}

fn main() -> ExitCode {
    let cmd = match parse_args(std::env::args().collect()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            print_usage();
            return ExitCode::from(2);
        }
    };
    match cmd {
        Cmd::Build { model, cache } => {
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
                )
                .init();

            let vault_path = std::env::var("VAULT_PATH").unwrap_or_else(|_| "./vault".to_string());
            let vault_path = std::path::PathBuf::from(vault_path);

            let embedder = match load_embedder(&model) {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::from(1);
                }
            };

            if cache.exists() {
                eprintln!("error: cache file already exists at {}. Delete it before rebuilding.", cache.display());
                return ExitCode::from(1);
            }

            let sqlite = match hatchdoor::cache::SqliteCache::open(&cache, embedder.embedding_dim()) {
                Ok(s) => s,
                Err(e) => { eprintln!("error opening cache: {e}"); return ExitCode::from(1); }
            };

            let index = match hatchdoor::vault::VaultIndex::build(&vault_path) {
                Ok(i) => i,
                Err(e) => { eprintln!("error building vault index: {e}"); return ExitCode::from(1); }
            };

            let started = std::time::Instant::now();
            if let Err(e) = sqlite.replace_from_index_with_embedder_stamped(&index, embedder.as_ref(), &model) {
                eprintln!("error populating cache: {e}");
                return ExitCode::from(1);
            }
            let elapsed = started.elapsed();
            println!("build complete: model={model} cache={} elapsed={:.1}s",
                cache.display(), elapsed.as_secs_f64());
            ExitCode::SUCCESS
        }
        Cmd::Run { model, cache, queries } => {
            let embedder = match load_embedder(&model) {
                Ok(e) => e,
                Err(e) => { eprintln!("error: {e}"); return ExitCode::from(1); }
            };

            if !cache.exists() {
                eprintln!("error: cache {} does not exist. Run `eval build` first.", cache.display());
                return ExitCode::from(1);
            }

            let sqlite = match hatchdoor::cache::SqliteCache::open(&cache, embedder.embedding_dim()) {
                Ok(s) => s,
                Err(e) => { eprintln!("error opening cache: {e}"); return ExitCode::from(1); }
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
                Err(e) => { eprintln!("error: {e}"); return ExitCode::from(1); }
            };

            let mut results = Vec::with_capacity(qs.len());
            for q in &qs {
                match sqlite.semantic_search(embedder.as_ref(), &q.query, 10) {
                    Ok(hits) => {
                        let top_k: Vec<String> = hits.into_iter().map(|h| h.note_slug).collect();
                        results.push(hatchdoor::eval::metrics::QueryResult {
                            query_id: q.id.clone(),
                            top_k,
                        });
                    }
                    Err(e) => {
                        eprintln!("warning: query {} failed: {e}", q.id);
                        results.push(hatchdoor::eval::metrics::QueryResult {
                            query_id: q.id.clone(),
                            top_k: Vec::new(),
                        });
                    }
                }
            }

            let report = hatchdoor::eval::metrics::aggregate(&model, &qs, &results);

            println!("\nmodel: {}", report.model_id);
            println!("queries: {}", qs.len());
            println!("Recall@5  (any/all): {:.3} / {:.3}", report.recall_at_5_any, report.recall_at_5_all);
            println!("Recall@10 (any/all): {:.3} / {:.3}", report.recall_at_10_any, report.recall_at_10_all);
            println!("MRR:                 {:.3}", report.mrr);
            println!("FP-rate@5:           {:.3}", report.fp_rate_at_5);

            let report_path = std::path::PathBuf::from("eval/results.md");
            if let Err(e) = hatchdoor::eval::report::append_section(&report_path, &report, build_dur) {
                eprintln!("warning: failed to write report: {e}");
            } else {
                println!("\nappended to {}", report_path.display());
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
        let cmd = parse_args(argv(&["eval", "build", "--model", "BGESmallENV15", "--cache", "/tmp/x.db"])).expect("parse");
        match cmd {
            Cmd::Build { model, cache } => {
                assert_eq!(model, "BGESmallENV15");
                assert_eq!(cache, PathBuf::from("/tmp/x.db"));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn parses_run_command_with_queries() {
        let cmd = parse_args(argv(&["eval", "run", "--model", "X", "--cache", "/c", "--queries", "/q"])).expect("parse");
        match cmd {
            Cmd::Run { model, cache, queries } => {
                assert_eq!(model, "X");
                assert_eq!(cache, PathBuf::from("/c"));
                assert_eq!(queries, PathBuf::from("/q"));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn rejects_unknown_subcommand() {
        let err = parse_args(argv(&["eval", "wat", "--model", "x", "--cache", "/y"])).unwrap_err();
        assert!(err.contains("unknown subcommand"));
    }
}
