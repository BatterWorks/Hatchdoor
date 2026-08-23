use std::collections::BTreeMap;
use std::env;
use std::time::{Duration, Instant};

use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use rusqlite::{Connection, OpenFlags};
use tokenizers_fe::Tokenizer;

const DEFAULT_CACHE: &str = "data/cache/hatchdoor-cache.sqlite3";
const SAMPLE_NOTES: usize = 16;
const PRODUCTION_EMBEDDER_ID: &str =
    "EmbeddingGemma300MQ4-768-max2048-fastembed-v5-ctx1-gemma-retrieval-v1";
const PRODUCTION_REPRESENTATION: &str =
    "EmbeddingGemma300MQ4 retrieval-format v1 (768-dim, max 2048)";

#[derive(Clone)]
struct CachedChunk {
    note_slug: String,
    ordinal: i64,
    title: String,
    heading_path: Option<String>,
    content: String,
    raw_tokens: usize,
}

impl CachedChunk {
    /// The exact EmbeddingGemma retrieval document format used by production.
    fn embed_input(&self) -> String {
        let title = if self.title.trim().is_empty() {
            "none"
        } else {
            &self.title
        };
        let text = match self.heading_path.as_deref() {
            Some(path) if !path.is_empty() => format!("Section: {path}\n\n{}", self.content),
            _ => self.content.clone(),
        };
        format!("title: {title} | text: {text}")
    }
}

struct BenchResult {
    elapsed: Duration,
    vectors: Vec<Vec<f32>>,
}

fn main() -> Result<(), String> {
    let cache_path = env::args()
        .nth(1)
        .unwrap_or_else(|| DEFAULT_CACHE.to_string());
    validate_cache_path_embedder_identity(&cache_path)?;
    let rows = read_chunks(&cache_path)?;
    if rows.is_empty() {
        return Err(format!("cache contains no chunks: {cache_path}"));
    }

    println!("Index microbenchmark (read-only; cache will not be modified)");
    println!("cache={cache_path} chunks={}", rows.len());
    println!("representation={PRODUCTION_REPRESENTATION}");

    let mut model_2048 = load_model(2048)?;
    let raw_tokenizer = untruncated_tokenizer(&model_2048.tokenizer)?;
    let chunks = measure_raw_tokens(rows, &raw_tokenizer)?;
    print_truncation_report(&chunks);

    let sample = select_note_sample(&chunks, SAMPLE_NOTES);
    let sample_chunk_count: usize = sample.iter().map(Vec::len).sum();
    let sample_raw_tokens: usize = sample.iter().flatten().map(|chunk| chunk.raw_tokens).sum();
    println!(
        "sample_notes={} sample_chunks={} sample_raw_tokens={}",
        sample.len(),
        sample_chunk_count,
        sample_raw_tokens
    );

    warm_up(&mut model_2048)?;
    let current = bench_per_note(&mut model_2048, &sample)?;
    let no_padding_2048 = bench_individual(&mut model_2048, &sample)?;
    drop(model_2048);

    let mut model_1024 = load_model(1024)?;
    warm_up(&mut model_1024)?;
    let reduced = bench_per_note(&mut model_1024, &sample)?;
    let no_padding_1024 = bench_individual(&mut model_1024, &sample)?;

    println!();
    println!("Paired inference timings on the same sample:");
    print_timing(
        "2048 / production per-note batches",
        &current,
        sample_chunk_count,
    );
    print_timing(
        "2048 / production one chunk per call",
        &no_padding_2048,
        sample_chunk_count,
    );
    print_timing(
        "1024 / reduced per-note batches",
        &reduced,
        sample_chunk_count,
    );
    print_timing(
        "1024 / reduced one chunk per call",
        &no_padding_1024,
        sample_chunk_count,
    );

    println!();
    print_ratio(
        "1024/reduced vs 2048/production",
        reduced.elapsed,
        current.elapsed,
    );
    print_ratio(
        "2048/individual vs 2048/production",
        no_padding_2048.elapsed,
        current.elapsed,
    );
    print_ratio(
        "1024/individual vs 2048/production",
        no_padding_1024.elapsed,
        current.elapsed,
    );
    println!(
        "mean_cosine_same_chunk_2048_vs_1024={:.6}",
        mean_matching_cosine(&no_padding_2048.vectors, &no_padding_1024.vectors)
    );

    Ok(())
}

fn load_model(max_length: usize) -> Result<TextEmbedding, String> {
    println!("loading_model=EmbeddingGemma300MQ4 max_length={max_length}");
    TextEmbedding::try_new(
        InitOptions::new(EmbeddingModel::EmbeddingGemma300MQ4)
            .with_max_length(max_length)
            .with_show_download_progress(false),
    )
    .map_err(|error| {
        format!("failed loading EmbeddingGemma model at max_length={max_length}: {error}")
    })
}

fn validate_cache_path_embedder_identity(path: &str) -> Result<(), String> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| format!("failed opening cache read-only: {error}"))?;
    let stamped = connection
        .query_row(
            "SELECT value FROM metadata WHERE key = 'embedder_id'",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok();
    validate_cache_embedder_identity(stamped.as_deref())
}

fn validate_cache_embedder_identity(stamped: Option<&str>) -> Result<(), String> {
    match stamped {
        Some(identity) if identity == PRODUCTION_EMBEDDER_ID => Ok(()),
        Some(identity) => Err(format!(
            "cache was built with embedder {identity}, but index_microbench requires {PRODUCTION_EMBEDDER_ID}; rebuild the cache with the active representation"
        )),
        None => Err(
            "cache has no embedder_id stamp; rebuild it with the active representation".to_string(),
        ),
    }
}

type ChunkRow = (String, i64, String, Option<String>, String);

fn read_chunks(path: &str) -> Result<Vec<ChunkRow>, String> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| format!("failed opening cache read-only: {error}"))?;
    let mut statement = connection
        .prepare(
            "SELECT c.note_slug, c.ordinal, n.title, c.heading_path, c.content \
             FROM chunks c JOIN notes n ON n.slug = c.note_slug \
             ORDER BY c.note_slug, c.ordinal",
        )
        .map_err(|error| format!("failed preparing chunk query: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .map_err(|error| format!("failed querying chunks: {error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed reading chunks: {error}"))
}

fn untruncated_tokenizer(tokenizer: &Tokenizer) -> Result<Tokenizer, String> {
    let mut tokenizer = tokenizer.clone();
    tokenizer
        .with_truncation(None)
        .map_err(|error| format!("failed disabling tokenizer truncation: {error}"))?;
    tokenizer.with_padding(None);
    Ok(tokenizer)
}

fn measure_raw_tokens(
    rows: Vec<ChunkRow>,
    tokenizer: &Tokenizer,
) -> Result<Vec<CachedChunk>, String> {
    rows.into_iter()
        .map(|(note_slug, ordinal, title, heading_path, content)| {
            let mut chunk = CachedChunk {
                note_slug,
                ordinal,
                title,
                heading_path,
                content,
                raw_tokens: 0,
            };
            chunk.raw_tokens = tokenizer
                .encode(chunk.embed_input(), true)
                .map_err(|error| {
                    format!(
                        "failed tokenizing {}#{}: {error}",
                        chunk.note_slug, chunk.ordinal
                    )
                })?
                .get_ids()
                .len();
            Ok(chunk)
        })
        .collect()
}

fn print_truncation_report(chunks: &[CachedChunk]) {
    let raw_total: usize = chunks.iter().map(|chunk| chunk.raw_tokens).sum();
    let over_1024 = chunks
        .iter()
        .filter(|chunk| chunk.raw_tokens > 1024)
        .count();
    let discarded_1024: usize = chunks
        .iter()
        .map(|chunk| chunk.raw_tokens.saturating_sub(1024))
        .sum();
    let over_2048 = chunks
        .iter()
        .filter(|chunk| chunk.raw_tokens > 2048)
        .count();
    let discarded_2048: usize = chunks
        .iter()
        .map(|chunk| chunk.raw_tokens.saturating_sub(2048))
        .sum();
    let max_raw = chunks
        .iter()
        .map(|chunk| chunk.raw_tokens)
        .max()
        .unwrap_or(0);
    println!(
        "raw_tokens={} max_chunk_tokens={} over_1024={} discarded_at_1024={} ({:.2}%) over_2048={} discarded_at_2048={} ({:.2}%)",
        raw_total,
        max_raw,
        over_1024,
        discarded_1024,
        percent(discarded_1024, raw_total),
        over_2048,
        discarded_2048,
        percent(discarded_2048, raw_total),
    );
}

fn select_note_sample(chunks: &[CachedChunk], target_notes: usize) -> Vec<Vec<CachedChunk>> {
    let mut notes: BTreeMap<&str, Vec<CachedChunk>> = BTreeMap::new();
    for chunk in chunks {
        notes
            .entry(chunk.note_slug.as_str())
            .or_default()
            .push(chunk.clone());
    }
    let mut notes: Vec<Vec<CachedChunk>> = notes.into_values().collect();
    notes.sort_by_key(|note| {
        let longest = note.iter().map(|chunk| chunk.raw_tokens).max().unwrap_or(0);
        longest * note.len()
    });
    let count = target_notes.min(notes.len());
    if count <= 1 {
        return notes.into_iter().take(count).collect();
    }
    (0..count)
        .map(|index| {
            let source_index = index * (notes.len() - 1) / (count - 1);
            notes[source_index].clone()
        })
        .collect()
}

fn warm_up(model: &mut TextEmbedding) -> Result<(), String> {
    model
        .embed(vec!["title: warm up | text: warm up".to_string()], None)
        .map(|_| ())
        .map_err(|error| format!("warm-up embed failed: {error}"))
}

fn bench_per_note(
    model: &mut TextEmbedding,
    notes: &[Vec<CachedChunk>],
) -> Result<BenchResult, String> {
    let started = Instant::now();
    let mut vectors = Vec::new();
    for note in notes {
        let inputs = note
            .iter()
            .map(CachedChunk::embed_input)
            .collect::<Vec<_>>();
        vectors.extend(
            model
                .embed(inputs, None)
                .map_err(|error| format!("per-note benchmark embed failed: {error}"))?,
        );
    }
    Ok(BenchResult {
        elapsed: started.elapsed(),
        vectors,
    })
}

fn bench_individual(
    model: &mut TextEmbedding,
    notes: &[Vec<CachedChunk>],
) -> Result<BenchResult, String> {
    let started = Instant::now();
    let mut vectors = Vec::new();
    for chunk in notes.iter().flatten() {
        vectors.extend(
            model
                .embed(vec![chunk.embed_input()], None)
                .map_err(|error| {
                    format!(
                        "individual benchmark embed failed for {}#{}: {error}",
                        chunk.note_slug, chunk.ordinal
                    )
                })?,
        );
    }
    Ok(BenchResult {
        elapsed: started.elapsed(),
        vectors,
    })
}

fn print_timing(label: &str, result: &BenchResult, chunks: usize) {
    println!(
        "{label}: {:.3}s ({:.3} chunks/s)",
        result.elapsed.as_secs_f64(),
        chunks as f64 / result.elapsed.as_secs_f64()
    );
}

fn print_ratio(label: &str, candidate: Duration, baseline: Duration) {
    let ratio = candidate.as_secs_f64() / baseline.as_secs_f64();
    println!("{label}: {ratio:.3}x ({:+.1}%)", (ratio - 1.0) * 100.0);
}

fn mean_matching_cosine(left: &[Vec<f32>], right: &[Vec<f32>]) -> f64 {
    let sum: f64 = left
        .iter()
        .zip(right)
        .map(|(left, right)| {
            left.iter()
                .zip(right)
                .map(|(a, b)| (*a as f64) * (*b as f64))
                .sum::<f64>()
        })
        .sum();
    sum / left.len().max(1) as f64
}

fn percent(part: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        part as f64 * 100.0 / total as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_identity_must_match_the_active_gemma_representation() {
        let mismatch = validate_cache_embedder_identity(Some(
            "NomicEmbedTextV15-768-max1024-fastembed-v5-ctx1",
        ))
        .expect_err("superseded Nomic cache must be refused");
        assert!(mismatch.contains("EmbeddingGemma300MQ4"));

        let missing =
            validate_cache_embedder_identity(None).expect_err("unstamped cache must be refused");
        assert!(missing.contains("no embedder_id stamp"));
    }
}
