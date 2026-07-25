use std::collections::BTreeMap;
use std::env;
use std::time::{Duration, Instant};

use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use rusqlite::{Connection, OpenFlags};
use tokenizers_fe::Tokenizer;

const DOC_PREFIX: &str = "search_document: ";
const DEFAULT_CACHE: &str = "data/cache/hatchdoor-cache.sqlite3";
const SAMPLE_NOTES: usize = 16;

#[derive(Clone)]
struct CachedChunk {
    note_slug: String,
    ordinal: i64,
    content: String,
    raw_tokens: usize,
}

struct BenchResult {
    elapsed: Duration,
    vectors: Vec<Vec<f32>>,
}

fn main() -> Result<(), String> {
    let cache_path = env::args()
        .nth(1)
        .unwrap_or_else(|| DEFAULT_CACHE.to_string());
    let rows = read_chunks(&cache_path)?;
    if rows.is_empty() {
        return Err(format!("cache contains no chunks: {cache_path}"));
    }

    println!("Index microbenchmark (read-only; cache will not be modified)");
    println!("cache={cache_path} chunks={}", rows.len());

    let mut model_512 = load_model(512)?;
    let raw_tokenizer = untruncated_tokenizer(&model_512.tokenizer)?;
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

    warm_up(&mut model_512)?;
    let current = bench_per_note(&mut model_512, &sample)?;
    let no_padding_512 = bench_individual(&mut model_512, &sample)?;
    drop(model_512);

    let mut model_1024 = load_model(1024)?;
    warm_up(&mut model_1024)?;
    let expanded = bench_per_note(&mut model_1024, &sample)?;
    let no_padding_1024 = bench_individual(&mut model_1024, &sample)?;

    println!();
    println!("Paired inference timings on the same sample:");
    print_timing(
        "512 / current per-note batches",
        &current,
        sample_chunk_count,
    );
    print_timing(
        "512 / one chunk per call",
        &no_padding_512,
        sample_chunk_count,
    );
    print_timing(
        "1024 / current per-note batches",
        &expanded,
        sample_chunk_count,
    );
    print_timing(
        "1024 / one chunk per call",
        &no_padding_1024,
        sample_chunk_count,
    );

    println!();
    print_ratio(
        "1024/current vs 512/current",
        expanded.elapsed,
        current.elapsed,
    );
    print_ratio(
        "512/individual vs 512/current",
        no_padding_512.elapsed,
        current.elapsed,
    );
    print_ratio(
        "1024/individual vs 512/current",
        no_padding_1024.elapsed,
        current.elapsed,
    );
    println!(
        "mean_cosine_same_chunk_512_vs_1024={:.6}",
        mean_matching_cosine(&no_padding_512.vectors, &no_padding_1024.vectors)
    );

    Ok(())
}

fn load_model(max_length: usize) -> Result<TextEmbedding, String> {
    println!("loading_model=NomicEmbedTextV15 max_length={max_length}");
    TextEmbedding::try_new(
        InitOptions::new(EmbeddingModel::NomicEmbedTextV15)
            .with_max_length(max_length)
            .with_show_download_progress(false),
    )
    .map_err(|error| format!("failed loading Nomic model at max_length={max_length}: {error}"))
}

fn read_chunks(path: &str) -> Result<Vec<(String, i64, String)>, String> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| format!("failed opening cache read-only: {error}"))?;
    let mut statement = connection
        .prepare("SELECT note_slug, ordinal, content FROM chunks ORDER BY note_slug, ordinal")
        .map_err(|error| format!("failed preparing chunk query: {error}"))?;
    let rows = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
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
    rows: Vec<(String, i64, String)>,
    tokenizer: &Tokenizer,
) -> Result<Vec<CachedChunk>, String> {
    rows.into_iter()
        .map(|(note_slug, ordinal, content)| {
            let input = format!("{DOC_PREFIX}{content}");
            let raw_tokens = tokenizer
                .encode(input, true)
                .map_err(|error| format!("failed tokenizing {note_slug}#{ordinal}: {error}"))?
                .get_ids()
                .len();
            Ok(CachedChunk {
                note_slug,
                ordinal,
                content,
                raw_tokens,
            })
        })
        .collect()
}

fn print_truncation_report(chunks: &[CachedChunk]) {
    let raw_total: usize = chunks.iter().map(|chunk| chunk.raw_tokens).sum();
    let over_512 = chunks.iter().filter(|chunk| chunk.raw_tokens > 512).count();
    let discarded_512: usize = chunks
        .iter()
        .map(|chunk| chunk.raw_tokens.saturating_sub(512))
        .sum();
    let over_1024 = chunks
        .iter()
        .filter(|chunk| chunk.raw_tokens > 1024)
        .count();
    let discarded_1024: usize = chunks
        .iter()
        .map(|chunk| chunk.raw_tokens.saturating_sub(1024))
        .sum();
    let max_raw = chunks
        .iter()
        .map(|chunk| chunk.raw_tokens)
        .max()
        .unwrap_or(0);
    println!(
        "raw_tokens={} max_chunk_tokens={} over_512={} discarded_at_512={} ({:.2}%) over_1024={} discarded_at_1024={} ({:.2}%)",
        raw_total,
        max_raw,
        over_512,
        discarded_512,
        percent(discarded_512, raw_total),
        over_1024,
        discarded_1024,
        percent(discarded_1024, raw_total),
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
        .embed(vec![format!("{DOC_PREFIX}warm up")], None)
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
            .map(|chunk| format!("{DOC_PREFIX}{}", chunk.content))
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
                .embed(vec![format!("{DOC_PREFIX}{}", chunk.content)], None)
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
