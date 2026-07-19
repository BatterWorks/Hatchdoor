use std::collections::{HashMap, HashSet};
use std::fs;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use rusqlite::{OptionalExtension, Transaction, params};

use crate::cache::chunk_ops::{
    ChunkRow, delete_orphan_vectors, existing_chunk_hashes, replace_chunks_for_note,
};
use crate::chunk::{ChunkOptions, NoteChunking, chunk_note};
use crate::embed::Embedder;
use crate::startup::IndexingProgressSnapshot;
use crate::vault::{NoteEntry, VaultIndex, normalize_title};

use super::SqliteCache;
use super::parse::{
    FileSnapshot, content_hash, current_unix_timestamp, extract_headings, extract_tags,
    file_snapshot,
};

pub enum UpsertOutcome {
    /// The note row was written; `content` is the file text already read (and
    /// hashed) during the upsert, threaded on so chunking reuses it instead of
    /// reading the file a second time (which could see a mid-reindex edit and
    /// chunk content that disagrees with the stored content_hash).
    Wrote {
        slug: String,
        content: String,
    },
    Unchanged,
}

const FIRST_PROGRESS_LOG_AFTER: Duration = Duration::from_secs(10);
const PROGRESS_LOG_INTERVAL: Duration = Duration::from_secs(60);

impl SqliteCache {
    pub fn replace_from_index_with_embedder(
        &self,
        index: &VaultIndex,
        embedder: &dyn Embedder,
    ) -> Result<(), String> {
        self.replace_from_index_with_embedder_and_progress(index, embedder, None)
    }

    pub fn replace_from_index_with_embedder_and_progress(
        &self,
        index: &VaultIndex,
        embedder: &dyn Embedder,
        on_progress: Option<Arc<dyn Fn(IndexingProgressSnapshot) + Send + Sync>>,
    ) -> Result<(), String> {
        // If the embedding model changed since the last build, rebuild from
        // scratch so no vectors from the old model are reused (mixed-model vector
        // spaces make cosine/L2 distances meaningless).
        self.reset_if_embedder_changed(embedder)?;
        let entries = index.ordered_entries();
        let current_paths = entries
            .iter()
            .map(|entry| entry.relative_path.clone())
            .collect::<HashSet<_>>();
        let now = current_unix_timestamp();
        let mut conn = self.connection()?;
        let tx = conn
            .transaction()
            .map_err(|e| format!("failed to start SQLite cache refresh: {e}"))?;

        let cached_paths = cached_relative_paths(&tx)?;
        let is_incremental_refresh = !cached_paths.is_empty();
        for cached_path in cached_paths {
            if !current_paths.contains(&cached_path) {
                delete_note_by_relative_path(&tx, &cached_path)?;
            }
        }

        let started_at = Instant::now();
        let process_cpu_started = process_cpu_time();
        let total_notes = entries.len();
        tracing::info!(
            "Preparing search index for {}…",
            format_note_count(total_notes)
        );
        let mut chunks_embedded: usize = 0;
        let mut chunks_reused: usize = 0;
        let mut per_note_failures: usize = 0;
        let mut notes_changed: usize = 0;
        let mut notes_unchanged: usize = 0;
        let mut metrics = IndexingMetrics::default();
        let mut prepared_notes = Vec::new();

        // Chunk and measure changed notes up front. Chunking was less than 0.5%
        // of the measured full-vault runtime, and retaining these results gives
        // the heartbeat an exact embedding-work denominator without chunking
        // notes twice.
        for entry in &entries {
            let note_sync_started = Instant::now();
            let upsert_outcome = upsert_note_if_changed(&tx, entry, now)?;
            metrics.note_sync += note_sync_started.elapsed();
            match upsert_outcome {
                UpsertOutcome::Wrote { slug, content } => {
                    match prepare_note_for_embedding(&tx, slug, content, embedder) {
                        Ok(prepared) => prepared_notes.push(prepared),
                        Err(error) => {
                            per_note_failures += 1;
                            tracing::warn!(slug = %entry.slug, error = %error, "Per-note embedding preparation failed; marking note for re-embed on next reindex");
                            invalidate_note_content_hash(&tx, &entry.slug)?;
                        }
                    }
                }
                UpsertOutcome::Unchanged => notes_unchanged += 1,
            }
        }

        let total_chunks_to_embed: usize = prepared_notes
            .iter()
            .map(|note| note.texts_to_embed.len())
            .sum();
        let total_tokens_to_embed: usize = prepared_notes
            .iter()
            .flat_map(|note| note.embedding_input_token_lengths.iter())
            .sum();
        tracing::debug!(
            changed_notes = prepared_notes.len(),
            total_chunks_to_embed,
            total_tokens_to_embed,
            "Prepared indexing workload"
        );

        let embedding_started_at = Instant::now();
        let (progress, stop_heartbeat, heartbeat) = start_indexing_heartbeat(
            total_notes,
            total_chunks_to_embed,
            total_tokens_to_embed,
            embedding_started_at,
        );
        progress
            .notes_processed
            .store(notes_unchanged + per_note_failures, Ordering::Relaxed);
        progress
            .failures
            .store(per_note_failures, Ordering::Relaxed);
        let progress_reporter = ProgressReporter {
            progress: progress.as_ref(),
            on_progress: on_progress.as_ref(),
            notes_total: total_notes,
            chunks_total: total_chunks_to_embed,
            tokens_total: total_tokens_to_embed,
            started_at: embedding_started_at,
        };
        progress_reporter.notify();

        let indexing_result = (|| -> Result<(), String> {
            for prepared in prepared_notes {
                let slug = prepared.slug.clone();
                match embed_prepared_note(&tx, prepared, embedder, &progress_reporter) {
                    Ok(stats) => {
                        notes_changed += 1;
                        chunks_embedded += stats.embedded;
                        chunks_reused += stats.reused;
                        metrics.record_chunk_stats(&stats);
                    }
                    Err(error) => {
                        per_note_failures += 1;
                        progress
                            .failures
                            .store(per_note_failures, Ordering::Relaxed);
                        tracing::warn!(slug = %slug, error = %error, "Per-note embedding failed; marking note for re-embed on next reindex");
                        invalidate_note_content_hash(&tx, &slug)?;
                    }
                }
                progress.notes_processed.fetch_add(1, Ordering::Relaxed);
                progress_reporter.notify();
            }
            Ok(())
        })();

        let _ = stop_heartbeat.send(());
        if heartbeat.join().is_err() {
            tracing::warn!("Indexing progress heartbeat stopped unexpectedly");
        }
        indexing_result?;

        tracing::info!("Updating links between notes…");
        let links_started = Instant::now();
        rebuild_links(&tx, index, &entries)?;
        metrics.link_rebuild = links_started.elapsed();
        let removed = delete_orphan_vectors(&tx)?;
        if removed > 0 {
            tracing::debug!(removed, "Swept orphan chunk vectors");
        }

        let commit_started = Instant::now();
        tx.commit()
            .map_err(|e| format!("failed to commit SQLite cache refresh: {e}"))?;
        metrics.commit = commit_started.elapsed();

        // Release the writer lock before set_metadata re-acquires it, otherwise
        // this self-deadlocks (the guard `conn` still holds the same Mutex).
        drop(conn);

        // Record which model produced these vectors so a future build with a
        // different model triggers reset_if_embedder_changed above.
        self.set_metadata("embedder_id", &embedder.identity())?;

        let elapsed = started_at.elapsed();
        let failure_summary = if per_note_failures == 0 {
            String::new()
        } else {
            format!(", {} failed", format_count(per_note_failures))
        };
        let changed_action = if is_incremental_refresh {
            "updated"
        } else {
            "indexed"
        };
        tracing::info!(
            chunks_embedded,
            chunks_reused,
            "Search index ready: {} checked, {} {}, {} unchanged{} in {}",
            format_note_count(total_notes),
            format_count(notes_changed),
            changed_action,
            format_count(notes_unchanged),
            failure_summary,
            format_elapsed(elapsed),
        );
        let process_cpu_elapsed = process_cpu_started
            .zip(process_cpu_time())
            .and_then(|(start, end)| end.checked_sub(start));
        log_indexing_performance(
            &metrics,
            total_notes,
            notes_changed,
            elapsed,
            process_cpu_elapsed,
        );
        Ok(())
    }

    pub fn replace_from_index_with_embedder_stamped(
        &self,
        index: &VaultIndex,
        embedder: &dyn Embedder,
        embedder_id: &str,
    ) -> Result<(), String> {
        let started = std::time::Instant::now();
        self.replace_from_index_with_embedder(index, embedder)?;
        let secs = started.elapsed().as_secs_f64();
        self.set_metadata("embedder_id", embedder_id)?;
        self.set_metadata("build_duration_secs", &format!("{secs:.3}"))?;
        Ok(())
    }
}

#[derive(Default)]
struct IndexingMetrics {
    note_sync: Duration,
    chunking: Duration,
    chunk_pipeline: Duration,
    vector_reuse: Duration,
    embedding: Duration,
    sqlite_chunk_write: Duration,
    link_rebuild: Duration,
    commit: Duration,
    chunks_total: usize,
    embedder_calls: usize,
    embedding_input_bytes: usize,
    embedding_input_tokens: usize,
    embedding_padded_tokens: usize,
    embedding_input_token_lengths: Vec<usize>,
    embedding_call_input_counts: Vec<usize>,
    embedding_call_token_counts: Vec<usize>,
    embedding_call_padded_token_counts: Vec<usize>,
    embedding_call_durations: Vec<Duration>,
    unique_chunk_hashes: HashSet<String>,
    duplicate_chunks: usize,
    duplicate_input_bytes: usize,
    duplicate_input_tokens: usize,
}

impl IndexingMetrics {
    fn record_chunk_stats(&mut self, stats: &ChunkStats) {
        self.chunking += stats.chunking;
        self.chunk_pipeline += stats.pipeline;
        self.vector_reuse += stats.vector_reuse;
        self.embedding += stats.embedding;
        self.sqlite_chunk_write += stats.sqlite_write;
        self.chunks_total += stats.embedded + stats.reused;
        self.embedder_calls += stats.embedder_calls;
        self.embedding_input_bytes += stats.embedding_input_bytes;
        self.embedding_input_tokens += stats.embedding_input_tokens;
        self.embedding_padded_tokens += stats.embedding_padded_tokens;
        self.embedding_input_token_lengths
            .extend(stats.embedding_input_token_lengths.iter().copied());
        if stats.embedder_calls > 0 {
            self.embedding_call_input_counts
                .extend(std::iter::repeat_n(1, stats.embedder_calls));
            self.embedding_call_token_counts
                .extend(stats.embedding_input_token_lengths.iter().copied());
            self.embedding_call_padded_token_counts
                .extend(stats.embedding_input_token_lengths.iter().copied());
            self.embedding_call_durations
                .extend(stats.embedding_call_durations.iter().copied());
        }
        for chunk in &stats.chunk_measurements {
            self.record_chunk_measurement(chunk);
        }
    }

    fn record_chunk_measurement(&mut self, chunk: &ChunkMeasurement) {
        if !self.unique_chunk_hashes.insert(chunk.content_hash.clone()) {
            self.duplicate_chunks += 1;
            self.duplicate_input_bytes += chunk.input_bytes;
            self.duplicate_input_tokens += chunk.input_tokens;
        }
    }
}

fn log_indexing_performance(
    metrics: &IndexingMetrics,
    notes_total: usize,
    notes_changed: usize,
    elapsed: Duration,
    process_cpu_elapsed: Option<Duration>,
) {
    let elapsed_seconds = elapsed.as_secs_f64();
    let embedding_share_percent = if elapsed_seconds > 0.0 {
        metrics.embedding.as_secs_f64() / elapsed_seconds * 100.0
    } else {
        0.0
    };
    let chunks_per_second = if elapsed_seconds > 0.0 {
        metrics.chunks_total as f64 / elapsed_seconds
    } else {
        0.0
    };
    let process_cpu_ms = process_cpu_elapsed.map(duration_ms).unwrap_or(-1.0);
    let process_cpu_utilization_percent = process_cpu_elapsed
        .filter(|_| elapsed_seconds > 0.0)
        .map(|cpu| cpu.as_secs_f64() / elapsed_seconds * 100.0)
        .unwrap_or(-1.0);
    let duplicate_token_share_percent = if metrics.embedding_input_tokens > 0 {
        metrics.duplicate_input_tokens as f64 / metrics.embedding_input_tokens as f64 * 100.0
    } else {
        0.0
    };
    let padding_tokens = metrics
        .embedding_padded_tokens
        .saturating_sub(metrics.embedding_input_tokens);
    let padding_token_share_percent = if metrics.embedding_padded_tokens > 0 {
        padding_tokens as f64 / metrics.embedding_padded_tokens as f64 * 100.0
    } else {
        0.0
    };

    tracing::debug!(
        notes_total,
        notes_changed,
        chunks_total = metrics.chunks_total,
        embedder_calls = metrics.embedder_calls,
        embedding_input_bytes = metrics.embedding_input_bytes,
        embedding_input_tokens = metrics.embedding_input_tokens,
        embedding_padded_tokens = metrics.embedding_padded_tokens,
        padding_tokens,
        padding_token_share_percent,
        input_tokens_p50 = percentile_usize(&metrics.embedding_input_token_lengths, 50),
        input_tokens_p95 = percentile_usize(&metrics.embedding_input_token_lengths, 95),
        input_tokens_p99 = percentile_usize(&metrics.embedding_input_token_lengths, 99),
        input_tokens_max = metrics
            .embedding_input_token_lengths
            .iter()
            .copied()
            .max()
            .unwrap_or(0),
        inputs_at_512_token_limit = metrics
            .embedding_input_token_lengths
            .iter()
            .filter(|tokens| **tokens == 512)
            .count(),
        call_inputs_p50 = percentile_usize(&metrics.embedding_call_input_counts, 50),
        call_inputs_p95 = percentile_usize(&metrics.embedding_call_input_counts, 95),
        call_inputs_max = metrics
            .embedding_call_input_counts
            .iter()
            .copied()
            .max()
            .unwrap_or(0),
        call_tokens_p50 = percentile_usize(&metrics.embedding_call_token_counts, 50),
        call_tokens_p95 = percentile_usize(&metrics.embedding_call_token_counts, 95),
        call_tokens_max = metrics
            .embedding_call_token_counts
            .iter()
            .copied()
            .max()
            .unwrap_or(0),
        call_padded_tokens_p50 = percentile_usize(&metrics.embedding_call_padded_token_counts, 50),
        call_padded_tokens_p95 = percentile_usize(&metrics.embedding_call_padded_token_counts, 95),
        call_padded_tokens_max = metrics
            .embedding_call_padded_token_counts
            .iter()
            .copied()
            .max()
            .unwrap_or(0),
        call_duration_ms_p50 = percentile_duration_ms(&metrics.embedding_call_durations, 50),
        call_duration_ms_p95 = percentile_duration_ms(&metrics.embedding_call_durations, 95),
        call_duration_ms_max = metrics
            .embedding_call_durations
            .iter()
            .copied()
            .max()
            .map(duration_ms)
            .unwrap_or(0.0),
        unique_chunk_hashes = metrics.unique_chunk_hashes.len(),
        duplicate_chunks = metrics.duplicate_chunks,
        duplicate_input_bytes = metrics.duplicate_input_bytes,
        duplicate_input_tokens = metrics.duplicate_input_tokens,
        duplicate_token_share_percent,
        note_sync_ms = duration_ms(metrics.note_sync),
        chunking_ms = duration_ms(metrics.chunking),
        chunk_pipeline_ms = duration_ms(metrics.chunk_pipeline),
        vector_reuse_ms = duration_ms(metrics.vector_reuse),
        embedding_ms = duration_ms(metrics.embedding),
        sqlite_chunk_write_ms = duration_ms(metrics.sqlite_chunk_write),
        link_rebuild_ms = duration_ms(metrics.link_rebuild),
        commit_ms = duration_ms(metrics.commit),
        total_ms = duration_ms(elapsed),
        embedding_share_percent,
        chunks_per_second,
        process_cpu_ms,
        process_cpu_utilization_percent,
        "Indexing performance summary"
    );
}

fn percentile_usize(values: &[usize], percentile: usize) -> usize {
    if values.is_empty() {
        return 0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let index = percentile_index(sorted.len(), percentile);
    sorted[index]
}

fn percentile_duration_ms(values: &[Duration], percentile: usize) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let index = percentile_index(sorted.len(), percentile);
    duration_ms(sorted[index])
}

fn percentile_index(len: usize, percentile: usize) -> usize {
    let rank = len.saturating_mul(percentile.min(100)).div_ceil(100);
    rank.saturating_sub(1).min(len - 1)
}

fn process_cpu_time() -> Option<Duration> {
    let mut usage = MaybeUninit::<libc::rusage>::uninit();
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
        return None;
    }
    let usage = unsafe { usage.assume_init() };
    timeval_duration(usage.ru_utime).checked_add(timeval_duration(usage.ru_stime))
}

fn timeval_duration(value: libc::timeval) -> Duration {
    Duration::from_secs(value.tv_sec.max(0) as u64)
        + Duration::from_micros(value.tv_usec.max(0) as u64)
}

fn duration_ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

#[derive(Default)]
struct IndexingProgress {
    notes_processed: AtomicUsize,
    chunks_processed: AtomicUsize,
    tokens_processed: AtomicUsize,
    failures: AtomicUsize,
}

struct ProgressReporter<'a> {
    progress: &'a IndexingProgress,
    on_progress: Option<&'a Arc<dyn Fn(IndexingProgressSnapshot) + Send + Sync>>,
    notes_total: usize,
    chunks_total: usize,
    tokens_total: usize,
    started_at: Instant,
}

impl ProgressReporter<'_> {
    fn notify(&self) {
        let Some(on_progress) = self.on_progress else {
            return;
        };
        on_progress(IndexingProgressSnapshot {
            notes_completed: self.progress.notes_processed.load(Ordering::Relaxed),
            notes_total: self.notes_total,
            chunks_completed: self.progress.chunks_processed.load(Ordering::Relaxed),
            chunks_total: self.chunks_total,
            tokens_completed: self.progress.tokens_processed.load(Ordering::Relaxed),
            tokens_total: self.tokens_total,
            elapsed_seconds: self.started_at.elapsed().as_secs(),
        });
    }
}

fn start_indexing_heartbeat(
    total_notes: usize,
    total_chunks: usize,
    total_tokens: usize,
    started_at: Instant,
) -> (
    Arc<IndexingProgress>,
    mpsc::Sender<()>,
    thread::JoinHandle<()>,
) {
    let progress = Arc::new(IndexingProgress::default());
    let heartbeat_progress = progress.clone();
    let (stop_tx, stop_rx) = mpsc::channel();
    let heartbeat = thread::spawn(move || {
        let mut has_logged = false;
        loop {
            match stop_rx.recv_timeout(progress_log_delay(has_logged)) {
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    log_indexing_progress(
                        heartbeat_progress.notes_processed.load(Ordering::Relaxed),
                        total_notes,
                        heartbeat_progress.chunks_processed.load(Ordering::Relaxed),
                        total_chunks,
                        heartbeat_progress.tokens_processed.load(Ordering::Relaxed),
                        total_tokens,
                        started_at.elapsed(),
                        heartbeat_progress.failures.load(Ordering::Relaxed),
                    );
                    has_logged = true;
                }
                Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });
    (progress, stop_tx, heartbeat)
}

fn progress_log_delay(has_logged: bool) -> Duration {
    if has_logged {
        PROGRESS_LOG_INTERVAL
    } else {
        FIRST_PROGRESS_LOG_AFTER
    }
}

fn estimated_remaining(
    elapsed: Duration,
    tokens_processed: usize,
    total_tokens: usize,
) -> Option<Duration> {
    if tokens_processed == 0 || tokens_processed >= total_tokens {
        return None;
    }
    let tokens_remaining = total_tokens - tokens_processed;
    Some(elapsed.mul_f64(tokens_remaining as f64 / tokens_processed as f64))
}

#[allow(clippy::too_many_arguments)]
fn log_indexing_progress(
    notes_processed: usize,
    total_notes: usize,
    chunks_processed: usize,
    total_chunks: usize,
    tokens_processed: usize,
    total_tokens: usize,
    elapsed: Duration,
    failures: usize,
) {
    tracing::info!(
        "{}",
        indexing_progress_message(
            notes_processed,
            total_notes,
            chunks_processed,
            total_chunks,
            tokens_processed,
            total_tokens,
            elapsed,
            failures,
        )
    );
}

#[allow(clippy::too_many_arguments)]
fn indexing_progress_message(
    notes_processed: usize,
    total_notes: usize,
    chunks_processed: usize,
    total_chunks: usize,
    tokens_processed: usize,
    total_tokens: usize,
    elapsed: Duration,
    failures: usize,
) -> String {
    let percent = tokens_processed.saturating_mul(100) / total_tokens.max(1);
    let eta = estimated_remaining(elapsed, tokens_processed, total_tokens)
        .map(format_eta)
        .unwrap_or_else(|| "estimating time remaining…".to_string());
    let failure_summary = if failures == 0 {
        String::new()
    } else {
        format!(" — {} failed", format_count(failures))
    };

    format!(
        "Indexing: {} of {} notes — {} of {} chunks — {}% of embedding work — {}{}",
        format_count(notes_processed),
        format_count(total_notes),
        format_count(chunks_processed),
        format_count(total_chunks),
        percent,
        eta,
        failure_summary,
    )
}

fn format_count(value: usize) -> String {
    let digits = value.to_string();
    let mut formatted = String::with_capacity(digits.len() + digits.len() / 3);
    for (position, character) in digits.chars().enumerate() {
        if position > 0 && (digits.len() - position) % 3 == 0 {
            formatted.push(',');
        }
        formatted.push(character);
    }
    formatted
}

fn format_note_count(value: usize) -> String {
    format!(
        "{} {}",
        format_count(value),
        if value == 1 { "note" } else { "notes" }
    )
}

fn format_eta(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds < 10 {
        "less than 10 seconds remaining".to_string()
    } else if seconds < 60 {
        format!("about {seconds} seconds remaining")
    } else if seconds < 3_600 {
        let minutes = seconds.div_ceil(60);
        format!("about {minutes} {} remaining", pluralize(minutes, "minute"))
    } else {
        let hours = seconds / 3_600;
        let minutes = (seconds % 3_600) / 60;
        if minutes == 0 {
            format!("about {hours} {} remaining", pluralize(hours, "hour"))
        } else {
            format!("about {hours}h {minutes}m remaining")
        }
    }
}

fn format_elapsed(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds == 0 {
        "less than 1s".to_string()
    } else if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3_600 {
        format!("{}m {}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h {}m", seconds / 3_600, (seconds % 3_600) / 60)
    }
}

fn pluralize(value: u64, singular: &str) -> String {
    if value == 1 {
        singular.to_string()
    } else {
        format!("{singular}s")
    }
}

#[derive(Debug)]
struct CachedNoteState {
    slug: String,
    content_hash: String,
    snapshot: FileSnapshot,
}

fn cached_relative_paths(tx: &Transaction<'_>) -> Result<Vec<String>, String> {
    let mut stmt = tx
        .prepare("SELECT relative_path FROM notes")
        .map_err(|error| format!("failed to prepare cached path query: {error}"))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| format!("failed to query cached note paths: {error}"))?;

    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| format!("failed reading cached note paths: {error}"))
}

fn cached_note_state(
    tx: &Transaction<'_>,
    relative_path: &str,
) -> Result<Option<CachedNoteState>, String> {
    tx.query_row(
        r#"
        SELECT slug, content_hash, mtime_ns, size_bytes
        FROM notes
        WHERE relative_path = ?1
        "#,
        params![relative_path],
        |row| {
            Ok(CachedNoteState {
                slug: row.get(0)?,
                content_hash: row.get(1)?,
                snapshot: FileSnapshot {
                    mtime_ns: row.get(2)?,
                    size_bytes: row.get(3)?,
                },
            })
        },
    )
    .optional()
    .map_err(|error| format!("failed reading cached state for '{relative_path}': {error}"))
}

/// Force a note to be re-processed on the next reindex by clearing its stored
/// content hash. Used when chunking/embedding failed for the note after its
/// `notes` row was already written, so the cache does not silently keep a note
/// whose chunks/vectors disagree with its content. The empty string can never
/// equal a real content hash, so change-detection will always re-fire.
fn invalidate_note_content_hash(tx: &Transaction<'_>, slug: &str) -> Result<(), String> {
    tx.execute(
        "UPDATE notes SET content_hash = '' WHERE slug = ?1",
        params![slug],
    )
    .map_err(|error| format!("failed invalidating content hash for '{slug}': {error}"))?;
    Ok(())
}

fn delete_note_by_relative_path(tx: &Transaction<'_>, relative_path: &str) -> Result<(), String> {
    let rowid = tx
        .query_row(
            "SELECT id FROM notes WHERE relative_path = ?1",
            params![relative_path],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| {
            format!("failed finding cached note '{relative_path}' for delete: {error}")
        })?;

    if let Some(rowid) = rowid {
        tx.execute("DELETE FROM note_fts WHERE rowid = ?1", params![rowid])
            .map_err(|error| format!("failed deleting FTS row for '{relative_path}': {error}"))?;
    }

    tx.execute(
        "DELETE FROM notes WHERE relative_path = ?1",
        params![relative_path],
    )
    .map_err(|error| format!("failed deleting cached note '{relative_path}': {error}"))?;
    Ok(())
}

fn upsert_note_if_changed(
    tx: &Transaction<'_>,
    entry: &NoteEntry,
    indexed_at: i64,
) -> Result<UpsertOutcome, String> {
    let snapshot = file_snapshot(&entry.path)?;
    let cached = cached_note_state(tx, &entry.relative_path)?;

    let content = fs::read_to_string(&entry.path)
        .map_err(|error| format!("failed reading note '{}': {error}", entry.path.display()))?;
    let hash = content_hash(&content);

    let cached_matches_file_and_content = cached.as_ref().is_some_and(|cached| {
        cached.slug == entry.slug && cached.snapshot == snapshot && cached.content_hash == hash
    });
    if cached_matches_file_and_content {
        return Ok(UpsertOutcome::Unchanged);
    }

    let cached_matches_content = cached
        .as_ref()
        .is_some_and(|cached| cached.slug == entry.slug && cached.content_hash == hash);
    if cached_matches_content {
        update_note_file_metadata(tx, entry, &content, snapshot, indexed_at)?;
        return Ok(UpsertOutcome::Unchanged);
    }

    if let Some(cached) = cached.as_ref()
        && cached.slug != entry.slug
    {
        delete_note_by_relative_path(tx, &entry.relative_path)?;
    }

    upsert_note_content(tx, entry, &content, &hash, snapshot, indexed_at)?;
    Ok(UpsertOutcome::Wrote {
        slug: entry.slug.clone(),
        content,
    })
}

fn update_note_file_metadata(
    tx: &Transaction<'_>,
    entry: &NoteEntry,
    content: &str,
    snapshot: FileSnapshot,
    indexed_at: i64,
) -> Result<(), String> {
    let normalized_title = normalize_title(&entry.title);
    let normalized_relative_path = normalize_title(&entry.relative_path);
    let absolute_path = entry.path.to_string_lossy().to_string();
    tx.execute(
        r#"
        UPDATE notes
        SET title = ?2,
            normalized_title = ?3,
            slug = ?4,
            normalized_relative_path = ?5,
            absolute_path = ?6,
            mtime_ns = ?7,
            size_bytes = ?8,
            indexed_at = ?9
        WHERE relative_path = ?1
        "#,
        params![
            &entry.relative_path,
            &entry.title,
            &normalized_title,
            &entry.slug,
            &normalized_relative_path,
            &absolute_path,
            snapshot.mtime_ns,
            snapshot.size_bytes,
            indexed_at,
        ],
    )
    .map_err(|error| {
        format!(
            "failed updating cached metadata for '{}': {error}",
            entry.slug
        )
    })?;

    let note_id = tx
        .query_row(
            "SELECT id FROM notes WHERE relative_path = ?1",
            params![&entry.relative_path],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("failed reading note id for '{}': {error}", entry.slug))?;
    tx.execute("DELETE FROM note_fts WHERE rowid = ?1", params![note_id])
        .map_err(|error| format!("failed deleting old FTS row for '{}': {error}", entry.slug))?;
    tx.execute(
        r#"
        INSERT INTO note_fts(rowid, title, relative_path, content, slug)
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
        params![
            note_id,
            &entry.title,
            &entry.relative_path,
            content,
            &entry.slug
        ],
    )
    .map_err(|error| {
        format!(
            "failed refreshing FTS metadata for '{}': {error}",
            entry.slug
        )
    })?;
    Ok(())
}

fn upsert_note_content(
    tx: &Transaction<'_>,
    entry: &NoteEntry,
    content: &str,
    hash: &str,
    snapshot: FileSnapshot,
    indexed_at: i64,
) -> Result<(), String> {
    let absolute_path = entry.path.to_string_lossy().to_string();
    let normalized_title = normalize_title(&entry.title);
    let normalized_relative_path = normalize_title(&entry.relative_path);

    tx.execute(
        r#"
        INSERT INTO notes(
            slug,
            title,
            normalized_title,
            relative_path,
            normalized_relative_path,
            absolute_path,
            content,
            content_hash,
            mtime_ns,
            size_bytes,
            indexed_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        ON CONFLICT(relative_path) DO UPDATE SET
            slug = excluded.slug,
            title = excluded.title,
            normalized_title = excluded.normalized_title,
            normalized_relative_path = excluded.normalized_relative_path,
            absolute_path = excluded.absolute_path,
            content = excluded.content,
            content_hash = excluded.content_hash,
            mtime_ns = excluded.mtime_ns,
            size_bytes = excluded.size_bytes,
            indexed_at = excluded.indexed_at
        "#,
        params![
            &entry.slug,
            &entry.title,
            &normalized_title,
            &entry.relative_path,
            &normalized_relative_path,
            &absolute_path,
            content,
            hash,
            snapshot.mtime_ns,
            snapshot.size_bytes,
            indexed_at,
        ],
    )
    .map_err(|error| format!("failed upserting note '{}': {error}", entry.slug))?;

    let note_id = tx
        .query_row(
            "SELECT id FROM notes WHERE relative_path = ?1",
            params![&entry.relative_path],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("failed reading note id for '{}': {error}", entry.slug))?;

    tx.execute("DELETE FROM note_fts WHERE rowid = ?1", params![note_id])
        .map_err(|error| format!("failed deleting old FTS row for '{}': {error}", entry.slug))?;
    tx.execute(
        r#"
        INSERT INTO note_fts(rowid, title, relative_path, content, slug)
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
        params![
            note_id,
            &entry.title,
            &entry.relative_path,
            content,
            &entry.slug
        ],
    )
    .map_err(|error| format!("failed indexing note '{}' for search: {error}", entry.slug))?;

    rebuild_note_details(tx, entry, content)?;
    Ok(())
}

fn rebuild_note_details(
    tx: &Transaction<'_>,
    entry: &NoteEntry,
    content: &str,
) -> Result<(), String> {
    tx.execute(
        "DELETE FROM headings WHERE note_slug = ?1",
        params![&entry.slug],
    )
    .map_err(|error| format!("failed deleting old headings for '{}': {error}", entry.slug))?;
    tx.execute(
        "DELETE FROM tags WHERE note_slug = ?1",
        params![&entry.slug],
    )
    .map_err(|error| format!("failed deleting old tags for '{}': {error}", entry.slug))?;

    for heading in extract_headings(content) {
        tx.execute(
            r#"
            INSERT OR IGNORE INTO headings(note_slug, level, text, anchor, position)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
            params![
                &entry.slug,
                heading.level as i64,
                &heading.text,
                &heading.anchor,
                heading.position as i64
            ],
        )
        .map_err(|error| format!("failed caching heading for '{}': {error}", entry.slug))?;
    }

    for tag in extract_tags(content) {
        tx.execute(
            r#"
            INSERT OR IGNORE INTO tags(note_slug, tag)
            VALUES (?1, ?2)
            "#,
            params![&entry.slug, &tag],
        )
        .map_err(|error| format!("failed caching tag for '{}': {error}", entry.slug))?;
    }

    Ok(())
}

fn rebuild_links(
    tx: &Transaction<'_>,
    index: &VaultIndex,
    entries: &[NoteEntry],
) -> Result<(), String> {
    tx.execute("DELETE FROM note_links", [])
        .map_err(|error| format!("failed clearing cached note links: {error}"))?;

    for entry in entries {
        if let Some(links) = index.note_links(&entry.slug) {
            for link in links.outgoing {
                tx.execute(
                    r#"
                    INSERT OR IGNORE INTO note_links(source_slug, target_slug)
                    VALUES (?1, ?2)
                    "#,
                    params![&entry.slug, &link.slug],
                )
                .map_err(|error| format!("failed caching link for '{}': {error}", entry.slug))?;
            }
        }
    }

    Ok(())
}

pub struct ChunkStats {
    #[allow(dead_code)]
    pub embedded: usize,
    #[allow(dead_code)]
    pub reused: usize,
    pub embedder_calls: usize,
    pub embedding_input_bytes: usize,
    pub embedding_input_tokens: usize,
    pub embedding_padded_tokens: usize,
    pub embedding_input_token_lengths: Vec<usize>,
    embedding_call_durations: Vec<Duration>,
    chunk_measurements: Vec<ChunkMeasurement>,
    pub pipeline: Duration,
    pub chunking: Duration,
    pub vector_reuse: Duration,
    pub embedding: Duration,
    pub sqlite_write: Duration,
}

#[derive(Clone)]
struct ChunkMeasurement {
    content_hash: String,
    input_bytes: usize,
    input_tokens: usize,
}

struct PreparedNote {
    slug: String,
    chunking: NoteChunking,
    preserved: HashMap<String, Vec<f32>>,
    texts_to_embed: Vec<String>,
    indices_needing_embed: Vec<usize>,
    embedding_input_bytes: usize,
    embedding_input_token_lengths: Vec<usize>,
    chunk_measurements: Vec<ChunkMeasurement>,
    chunking_elapsed: Duration,
    vector_reuse_elapsed: Duration,
}

fn prepare_note_for_embedding(
    tx: &Transaction<'_>,
    slug: String,
    content: String,
    embedder: &dyn Embedder,
) -> Result<PreparedNote, String> {
    let chunking_started = Instant::now();
    let tokenizer = embedder.tokenizer();
    let chunking = chunk_note(&content, tokenizer.clone(), ChunkOptions::default());
    let chunking_elapsed = chunking_started.elapsed();

    let reuse_started = Instant::now();
    let existing = existing_chunk_hashes(tx, &slug)?;
    let preserved = preserve_existing_vectors(tx, &slug, &chunking.chunks, &existing)?;
    let vector_reuse_elapsed = reuse_started.elapsed();

    let doc_prefix = embedder.doc_prefix();
    let chunk_measurements = chunking
        .chunks
        .iter()
        .map(|chunk| {
            let input = format!("{doc_prefix}{}", chunk.content);
            let input_tokens = tokenizer
                .encode(input.as_str(), true)
                .map_err(|error| format!("failed measuring tokens for '{slug}': {error}"))?
                .get_ids()
                .len();
            Ok(ChunkMeasurement {
                content_hash: chunk.content_hash.clone(),
                input_bytes: input.len(),
                input_tokens,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut texts_to_embed: Vec<String> = Vec::new();
    let mut indices_needing_embed: Vec<usize> = Vec::new();
    for (idx, chunk) in chunking.chunks.iter().enumerate() {
        if !preserved.contains_key(&chunk.content_hash) {
            texts_to_embed.push(format!("{doc_prefix}{}", chunk.content));
            indices_needing_embed.push(idx);
        }
    }

    if !texts_to_embed.is_empty() {
        tracing::debug!(
            slug,
            new = texts_to_embed.len(),
            reused = chunking.chunks.len() - texts_to_embed.len(),
            "Embedding chunks for note"
        );
    }

    let embedding_input_bytes = texts_to_embed.iter().map(String::len).sum();
    let embedding_input_token_lengths: Vec<usize> = indices_needing_embed
        .iter()
        .map(|index| chunk_measurements[*index].input_tokens)
        .collect();
    let embedded_chunk_measurements: Vec<ChunkMeasurement> = indices_needing_embed
        .iter()
        .map(|index| chunk_measurements[*index].clone())
        .collect();

    Ok(PreparedNote {
        slug,
        chunking,
        preserved,
        texts_to_embed,
        indices_needing_embed,
        embedding_input_bytes,
        embedding_input_token_lengths,
        chunk_measurements: embedded_chunk_measurements,
        chunking_elapsed,
        vector_reuse_elapsed,
    })
}

fn embed_prepared_note(
    tx: &Transaction<'_>,
    prepared: PreparedNote,
    embedder: &dyn Embedder,
    progress_reporter: &ProgressReporter<'_>,
) -> Result<ChunkStats, String> {
    let progress = progress_reporter.progress;
    let pipeline_started = Instant::now();
    let PreparedNote {
        slug,
        chunking,
        preserved,
        texts_to_embed,
        indices_needing_embed,
        embedding_input_bytes,
        embedding_input_token_lengths,
        chunk_measurements,
        chunking_elapsed,
        vector_reuse_elapsed,
    } = prepared;
    if chunking.chunks.is_empty() {
        let sqlite_started = Instant::now();
        replace_chunks_for_note(tx, &slug, &[], None, None)?;
        return Ok(ChunkStats {
            embedded: 0,
            reused: 0,
            embedder_calls: 0,
            embedding_input_bytes: 0,
            embedding_input_tokens: 0,
            embedding_padded_tokens: 0,
            embedding_input_token_lengths: Vec::new(),
            embedding_call_durations: Vec::new(),
            chunk_measurements: Vec::new(),
            pipeline: pipeline_started.elapsed() + chunking_elapsed + vector_reuse_elapsed,
            chunking: chunking_elapsed,
            vector_reuse: vector_reuse_elapsed,
            embedding: Duration::ZERO,
            sqlite_write: sqlite_started.elapsed(),
        });
    }

    let embedding_input_tokens: usize = embedding_input_token_lengths.iter().sum();
    // Each chunk is embedded in its own call. This avoids BatchLongest padding
    // short chunks to the longest sibling chunk in the same note.
    let embedding_padded_tokens = embedding_input_tokens;
    let embedding_started = Instant::now();
    let mut new_vectors = Vec::with_capacity(texts_to_embed.len());
    let mut embedding_call_durations = Vec::with_capacity(texts_to_embed.len());
    for (text, input_tokens) in texts_to_embed
        .iter()
        .zip(embedding_input_token_lengths.iter().copied())
    {
        let call_started = Instant::now();
        let mut vectors = embedder.embed(std::slice::from_ref(text))?;
        embedding_call_durations.push(call_started.elapsed());
        if vectors.len() != 1 {
            return Err(format!(
                "embedder returned {} vectors for one input",
                vectors.len()
            ));
        }
        new_vectors.push(vectors.remove(0));
        progress.chunks_processed.fetch_add(1, Ordering::Relaxed);
        progress
            .tokens_processed
            .fetch_add(input_tokens, Ordering::Relaxed);
        progress_reporter.notify();
    }
    let embedding_elapsed = embedding_started.elapsed();
    if !texts_to_embed.is_empty() {
        let tokens_per_second = if embedding_elapsed.is_zero() {
            0.0
        } else {
            embedding_input_tokens as f64 / embedding_elapsed.as_secs_f64()
        };
        tracing::debug!(
            slug,
            inputs = texts_to_embed.len(),
            input_bytes = embedding_input_bytes,
            input_tokens = embedding_input_tokens,
            padded_tokens = embedding_padded_tokens,
            padding_tokens = embedding_padded_tokens.saturating_sub(embedding_input_tokens),
            min_input_tokens = embedding_input_token_lengths
                .iter()
                .copied()
                .min()
                .unwrap_or(0),
            max_input_tokens = embedding_input_token_lengths
                .iter()
                .copied()
                .max()
                .unwrap_or(0),
            elapsed_ms = duration_ms(embedding_elapsed),
            tokens_per_second,
            calls = texts_to_embed.len(),
            "Embedding note performance"
        );
    }

    let mut vectors: Vec<Vec<f32>> = Vec::with_capacity(chunking.chunks.len());
    let mut need_new: std::collections::HashSet<usize> =
        indices_needing_embed.iter().copied().collect();
    let mut new_iter = new_vectors.into_iter();
    for (idx, chunk) in chunking.chunks.iter().enumerate() {
        if need_new.remove(&idx) {
            vectors.push(new_iter.next().ok_or("embedder returned too few vectors")?);
        } else {
            vectors.push(
                preserved
                    .get(&chunk.content_hash)
                    .cloned()
                    .ok_or("preserved vector missing for unchanged chunk")?,
            );
        }
    }

    let tags_json = serde_json::to_string(&chunking.tags).ok();
    let aliases_json = serde_json::to_string(&chunking.aliases).ok();
    let rows: Vec<ChunkRow<'_>> = chunking
        .chunks
        .iter()
        .zip(vectors.iter())
        .map(|(chunk, vector)| ChunkRow { chunk, vector })
        .collect();

    let sqlite_started = Instant::now();
    replace_chunks_for_note(
        tx,
        &slug,
        &rows,
        tags_json.as_deref(),
        aliases_json.as_deref(),
    )?;
    Ok(ChunkStats {
        embedded: indices_needing_embed.len(),
        reused: chunking.chunks.len() - indices_needing_embed.len(),
        embedder_calls: texts_to_embed.len(),
        embedding_input_bytes,
        embedding_input_tokens,
        embedding_padded_tokens,
        embedding_input_token_lengths,
        embedding_call_durations,
        chunk_measurements,
        pipeline: pipeline_started.elapsed() + chunking_elapsed + vector_reuse_elapsed,
        chunking: chunking_elapsed,
        vector_reuse: vector_reuse_elapsed,
        embedding: embedding_elapsed,
        sqlite_write: sqlite_started.elapsed(),
    })
}

fn preserve_existing_vectors(
    tx: &Transaction<'_>,
    _slug: &str,
    chunks: &[crate::chunk::Chunk],
    existing: &std::collections::HashMap<String, i64>,
) -> Result<std::collections::HashMap<String, Vec<f32>>, String> {
    let mut out = std::collections::HashMap::new();
    let mut stmt = tx
        .prepare("SELECT embedding FROM chunk_vectors WHERE chunk_id = ?1")
        .map_err(|e| format!("prepare vector lookup: {e}"))?;
    for chunk in chunks {
        if let Some(chunk_id) = existing.get(&chunk.content_hash) {
            let bytes: Vec<u8> = stmt
                .query_row(rusqlite::params![chunk_id], |row| row.get(0))
                .map_err(|e| format!("read preserved vector: {e}"))?;
            let floats: Vec<f32> = bytemuck::cast_slice(&bytes).to_vec();
            out.insert(chunk.content_hash.clone(), floats);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::SqliteCache;
    use crate::embed::{Embedder, StubEmbedder};
    use crate::vault::VaultIndex;
    use std::sync::Arc;
    use tempfile::tempdir;

    #[test]
    fn performance_percentiles_use_nearest_rank() {
        let values = [10, 20, 30, 40, 50];
        assert_eq!(percentile_usize(&values, 50), 30);
        assert_eq!(percentile_usize(&values, 95), 50);
        assert_eq!(percentile_usize(&[], 95), 0);
    }

    #[test]
    fn duplicate_measurements_count_only_repeated_embedding_inputs() {
        let first = ChunkMeasurement {
            content_hash: "same".to_string(),
            input_bytes: 100,
            input_tokens: 25,
        };
        let repeated = ChunkMeasurement {
            content_hash: "same".to_string(),
            input_bytes: 100,
            input_tokens: 25,
        };
        let unique = ChunkMeasurement {
            content_hash: "different".to_string(),
            input_bytes: 80,
            input_tokens: 20,
        };
        let mut metrics = IndexingMetrics::default();

        metrics.record_chunk_measurement(&first);
        metrics.record_chunk_measurement(&repeated);
        metrics.record_chunk_measurement(&unique);

        assert_eq!(metrics.unique_chunk_hashes.len(), 2);
        assert_eq!(metrics.duplicate_chunks, 1);
        assert_eq!(metrics.duplicate_input_bytes, 100);
        assert_eq!(metrics.duplicate_input_tokens, 25);
    }

    #[test]
    fn progress_observer_receives_exact_workload_and_completion() {
        let dir = tempdir().expect("temp dir");
        std::fs::write(
            dir.path().join("Long note.md"),
            format!("# Long note\n\n{}", "measured indexing work ".repeat(900)),
        )
        .expect("write note");
        let index = VaultIndex::build(dir.path()).expect("index");
        let cache = SqliteCache::in_memory(384).expect("cache");
        let snapshots = Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed = snapshots.clone();
        let observer = Arc::new(move |snapshot| {
            observed.lock().expect("snapshots lock").push(snapshot);
        });

        cache
            .replace_from_index_with_embedder_and_progress(
                &index,
                &StubEmbedder::new(384),
                Some(observer),
            )
            .expect("populate cache");

        let snapshots = snapshots.lock().expect("snapshots lock");
        let first = snapshots.first().expect("initial snapshot");
        let last = snapshots.last().expect("final snapshot");
        assert_eq!(first.notes_total, 1);
        assert!(first.chunks_total > 1);
        assert!(first.tokens_total > 0);
        assert_eq!(first.tokens_completed, 0);
        assert_eq!(last.notes_completed, last.notes_total);
        assert_eq!(last.chunks_completed, last.chunks_total);
        assert_eq!(last.tokens_completed, last.tokens_total);
    }

    #[test]
    fn process_cpu_measurement_is_available_and_monotonic() {
        let before = process_cpu_time().expect("process CPU time");
        let after = process_cpu_time().expect("process CPU time");
        assert!(after >= before);
    }

    #[test]
    fn replace_from_index_stamps_embedder_id_and_build_duration() {
        use crate::embed::StubEmbedder;
        use crate::vault::VaultIndex;

        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a.md"), "# A\nhello").unwrap();
        let index = VaultIndex::build(dir.path()).expect("index");
        let cache = SqliteCache::in_memory(384).expect("cache");
        let embedder = StubEmbedder::new(384);

        cache
            .replace_from_index_with_embedder_stamped(&index, &embedder, "TestStub")
            .expect("populate");

        assert_eq!(
            cache.get_metadata("embedder_id").expect("get").as_deref(),
            Some("TestStub")
        );
        let dur = cache
            .get_metadata("build_duration_secs")
            .expect("get")
            .expect("present");
        assert!(
            dur.parse::<f64>().is_ok(),
            "duration should parse as f64, got {dur}"
        );
    }

    #[test]
    fn refresh_updates_content_even_when_cached_file_snapshot_matches() {
        let dir = tempdir().expect("temp dir");
        let note_path = dir.path().join("Home.md");
        fs::write(&note_path, "# Home\nalpha token").expect("write original note");

        let cache = SqliteCache::in_memory(384).expect("sqlite cache");
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
        let index = VaultIndex::build(dir.path()).expect("build original index");
        cache
            .replace_from_index_with_embedder(&index, embedder.as_ref())
            .expect("initial populate");

        fs::write(&note_path, "# Home\nbravo token").expect("write changed note");
        let snapshot = file_snapshot(&note_path).expect("file snapshot");
        {
            let conn = cache.connection().expect("connection");
            conn.execute(
                "UPDATE notes SET mtime_ns = ?1, size_bytes = ?2 WHERE slug = 'home'",
                params![snapshot.mtime_ns, snapshot.size_bytes],
            )
            .expect("force cached snapshot to match file");
        }

        let refreshed_index = VaultIndex::build(dir.path()).expect("build refreshed index");
        cache
            .replace_from_index_with_embedder(&refreshed_index, embedder.as_ref())
            .expect("refresh populate");

        let note = cache
            .read_note_by_slug("home")
            .expect("read note")
            .expect("note exists");
        assert_eq!(note.content, "# Home\nbravo token");

        let hits = cache.search("bravo", true, 10).expect("content search");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].slug, "home");
    }
}

#[cfg(test)]
mod chunk_integration_tests {
    use std::sync::Arc;
    use std::time::Duration;

    use tempfile::TempDir;

    use super::{
        estimated_remaining, format_count, format_elapsed, format_eta, format_note_count,
        indexing_progress_message, progress_log_delay,
    };
    use crate::cache::SqliteCache;
    use crate::embed::{Embedder, StubEmbedder};
    use crate::vault::VaultIndex;

    fn make_vault(files: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new().expect("tempdir");
        for (name, body) in files {
            std::fs::write(dir.path().join(name), body).expect("write");
        }
        dir
    }

    /// Embedder whose `embed` always fails, to simulate a transient embedding
    /// error (OOM, model timeout, read race) for a note during reindex.
    struct FailingEmbedder {
        inner: StubEmbedder,
    }
    impl Embedder for FailingEmbedder {
        fn embed(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
            Err("simulated embed failure".to_string())
        }
        fn embedding_dim(&self) -> usize {
            self.inner.embedding_dim()
        }
        fn tokenizer(&self) -> std::sync::Arc<tokenizers::Tokenizer> {
            self.inner.tokenizer()
        }
    }

    fn note_chunk_count(cache: &SqliteCache, slug: &str) -> i64 {
        cache
            .connection()
            .expect("conn")
            .query_row(
                "SELECT COUNT(*) FROM chunks WHERE note_slug = ?1",
                [slug],
                |r| r.get(0),
            )
            .expect("count")
    }

    #[test]
    fn per_note_embed_failure_self_heals_on_next_reindex() {
        let dir = make_vault(&[("a.md", "# A\n\nbody A")]);
        let cache = SqliteCache::in_memory(384).expect("open");
        let index = VaultIndex::build(dir.path()).expect("build");

        // First reindex: embedding fails. The failure is swallowed (the build
        // still completes) and the note is left with NO chunks.
        let failing = Arc::new(FailingEmbedder {
            inner: StubEmbedder::new(384),
        });
        cache
            .replace_from_index_with_embedder(&index, failing.as_ref())
            .expect("first populate completes despite per-note failure");
        assert_eq!(
            note_chunk_count(&cache, "a"),
            0,
            "embed failed, so the note has no chunks yet"
        );

        // Second reindex with a working embedder must RE-CHUNK the note rather
        // than treating it as Unchanged (change-detection keys off content_hash,
        // which the failed first pass must have invalidated).
        let working: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
        cache
            .replace_from_index_with_embedder(&index, working.as_ref())
            .expect("second populate");
        assert!(
            note_chunk_count(&cache, "a") > 0,
            "note must be re-chunked once the embedder recovers, not stuck Unchanged"
        );
    }

    /// Wraps a StubEmbedder with a caller-chosen identity and a call counter, so
    /// tests can simulate swapping the embedding model.
    struct IdentifiedEmbedder {
        inner: StubEmbedder,
        id: String,
        embed_calls: std::sync::atomic::AtomicUsize,
    }
    impl IdentifiedEmbedder {
        fn new(id: &str) -> Self {
            Self {
                inner: StubEmbedder::new(384),
                id: id.to_string(),
                embed_calls: std::sync::atomic::AtomicUsize::new(0),
            }
        }
    }
    impl Embedder for IdentifiedEmbedder {
        fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
            self.embed_calls
                .fetch_add(texts.len(), std::sync::atomic::Ordering::SeqCst);
            self.inner.embed(texts)
        }
        fn embedding_dim(&self) -> usize {
            self.inner.embedding_dim()
        }
        fn identity(&self) -> String {
            self.id.clone()
        }
        fn tokenizer(&self) -> std::sync::Arc<tokenizers::Tokenizer> {
            self.inner.tokenizer()
        }
    }

    #[test]
    fn swapping_the_embedder_model_rebuilds_the_vector_index() {
        // Two models with the same dimension but different identities. Reusing
        // the first model's vectors for unchanged notes under the second model
        // would mix two incompatible embedding spaces in one vec0 index.
        let dir = make_vault(&[("a.md", "# A\n\nbody A")]);
        let cache = SqliteCache::in_memory(384).expect("open");
        let index = VaultIndex::build(dir.path()).expect("build");

        let model_a = IdentifiedEmbedder::new("model-a");
        cache
            .replace_from_index_with_embedder(&index, &model_a)
            .expect("first build");
        assert_eq!(
            cache.get_metadata("embedder_id").expect("get").as_deref(),
            Some("model-a")
        );

        // Same vault content, different model. The note is byte-identical, so
        // content-hash change-detection would treat it as Unchanged and reuse
        // model-a's vectors — unless the identity change forces a rebuild.
        let model_b = IdentifiedEmbedder::new("model-b");
        cache
            .replace_from_index_with_embedder(&index, &model_b)
            .expect("second build");

        assert!(
            model_b
                .embed_calls
                .load(std::sync::atomic::Ordering::SeqCst)
                > 0,
            "a model swap must re-embed the vault, not reuse the old model's vectors"
        );
        assert_eq!(
            cache.get_metadata("embedder_id").expect("get").as_deref(),
            Some("model-b"),
            "the new model's identity must be stamped"
        );
    }

    #[test]
    fn replace_from_index_chunks_and_embeds_every_note() {
        let dir = make_vault(&[("a.md", "# A\n\nbody A"), ("b.md", "# B\n\nbody B")]);
        let cache = SqliteCache::in_memory(384).expect("open");
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
        let index = VaultIndex::build(dir.path()).expect("build");

        cache
            .replace_from_index_with_embedder(&index, embedder.as_ref())
            .expect("replace");

        let conn = cache.connection().expect("conn");
        let note_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM notes", [], |r| r.get(0))
            .expect("count");
        assert_eq!(note_count, 2);
        let chunk_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0))
            .expect("count");
        assert!(chunk_count >= 2);
        let vector_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM chunk_vectors", [], |r| r.get(0))
            .expect("count");
        assert_eq!(vector_count, chunk_count);
    }

    #[test]
    fn unchanged_note_triggers_zero_new_embedding_calls() {
        struct CountingEmbedder {
            inner: StubEmbedder,
            calls: std::sync::atomic::AtomicUsize,
        }
        impl Embedder for CountingEmbedder {
            fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
                self.calls
                    .fetch_add(texts.len(), std::sync::atomic::Ordering::SeqCst);
                self.inner.embed(texts)
            }
            fn embedding_dim(&self) -> usize {
                self.inner.embedding_dim()
            }
            fn tokenizer(&self) -> std::sync::Arc<tokenizers::Tokenizer> {
                self.inner.tokenizer()
            }
        }

        let dir = make_vault(&[("a.md", "# A\n\nbody A")]);
        let cache = SqliteCache::in_memory(384).expect("open");
        let embedder = Arc::new(CountingEmbedder {
            inner: StubEmbedder::new(384),
            calls: 0.into(),
        });

        let index = VaultIndex::build(dir.path()).expect("build");
        cache
            .replace_from_index_with_embedder(&index, embedder.as_ref())
            .expect("first");
        let first_calls = embedder.calls.load(std::sync::atomic::Ordering::SeqCst);
        assert!(first_calls >= 1);

        cache
            .replace_from_index_with_embedder(&index, embedder.as_ref())
            .expect("second");
        let second_calls = embedder.calls.load(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            second_calls, first_calls,
            "unchanged note must not re-embed"
        );
    }

    #[test]
    fn new_chunks_are_embedded_one_per_call() {
        struct BatchRecordingEmbedder {
            inner: StubEmbedder,
            calls: std::sync::atomic::AtomicUsize,
            largest_batch: std::sync::atomic::AtomicUsize,
        }
        impl Embedder for BatchRecordingEmbedder {
            fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
                self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                self.largest_batch
                    .fetch_max(texts.len(), std::sync::atomic::Ordering::SeqCst);
                self.inner.embed(texts)
            }
            fn embedding_dim(&self) -> usize {
                self.inner.embedding_dim()
            }
            fn tokenizer(&self) -> std::sync::Arc<tokenizers::Tokenizer> {
                self.inner.tokenizer()
            }
        }

        let body = (0..1_700)
            .map(|index| format!("word{index}"))
            .collect::<Vec<_>>()
            .join(" ");
        let dir = make_vault(&[("long.md", &body)]);
        let cache = SqliteCache::in_memory(384).expect("open");
        let embedder = BatchRecordingEmbedder {
            inner: StubEmbedder::new(384),
            calls: 0.into(),
            largest_batch: 0.into(),
        };
        let index = VaultIndex::build(dir.path()).expect("build");

        cache
            .replace_from_index_with_embedder(&index, &embedder)
            .expect("populate");

        assert!(
            embedder.calls.load(std::sync::atomic::Ordering::SeqCst) > 1,
            "fixture must produce multiple chunks"
        );
        assert_eq!(
            embedder
                .largest_batch
                .load(std::sync::atomic::Ordering::SeqCst),
            1,
            "indexing must avoid cross-chunk padding"
        );
    }

    #[test]
    fn deleting_a_note_removes_its_chunks_and_vectors() {
        let dir = make_vault(&[("a.md", "# A\n\nbody A"), ("b.md", "# B\n\nbody B")]);
        let cache = SqliteCache::in_memory(384).expect("open");
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));

        let index1 = VaultIndex::build(dir.path()).expect("build1");
        cache
            .replace_from_index_with_embedder(&index1, embedder.as_ref())
            .expect("first");

        std::fs::remove_file(dir.path().join("b.md")).expect("remove");
        let index2 = VaultIndex::build(dir.path()).expect("build2");
        cache
            .replace_from_index_with_embedder(&index2, embedder.as_ref())
            .expect("second");

        let conn = cache.connection().expect("conn");
        let chunks_for_b: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chunks WHERE note_slug = 'b'",
                [],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(chunks_for_b, 0);
        let total_vectors: i64 = conn
            .query_row("SELECT COUNT(*) FROM chunk_vectors", [], |r| r.get(0))
            .expect("count");
        let total_chunks: i64 = conn
            .query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0))
            .expect("count");
        assert_eq!(
            total_vectors, total_chunks,
            "no orphan vectors after delete"
        );
    }

    #[test]
    fn progress_logging_starts_after_ten_seconds_then_repeats_each_minute() {
        assert_eq!(progress_log_delay(false), Duration::from_secs(10));
        assert_eq!(progress_log_delay(true), Duration::from_secs(60));
    }

    #[test]
    fn remaining_time_is_extrapolated_from_processed_tokens() {
        assert_eq!(
            estimated_remaining(Duration::from_secs(30), 25_000, 100_000),
            Some(Duration::from_secs(90))
        );
        assert_eq!(
            estimated_remaining(Duration::from_secs(30), 0, 100_000),
            None
        );
        assert_eq!(
            estimated_remaining(Duration::from_secs(30), 100_000, 100_000),
            None
        );
    }

    #[test]
    fn progress_message_keeps_note_counts_but_percent_and_eta_follow_tokens() {
        assert_eq!(
            indexing_progress_message(
                45,
                309,
                80,
                573,
                50_000,
                247_202,
                Duration::from_secs(60),
                0,
            ),
            "Indexing: 45 of 309 notes — 80 of 573 chunks — 20% of embedding work — about 4 minutes remaining"
        );
    }

    #[test]
    fn progress_values_are_formatted_for_people() {
        assert_eq!(format_count(12_481), "12,481");
        assert_eq!(format_note_count(1), "1 note");
        assert_eq!(format_note_count(12_481), "12,481 notes");
        assert_eq!(
            format_eta(Duration::from_secs(8)),
            "less than 10 seconds remaining"
        );
        assert_eq!(
            format_eta(Duration::from_secs(125)),
            "about 3 minutes remaining"
        );
        assert_eq!(format_elapsed(Duration::from_secs(166)), "2m 46s");
    }
}
