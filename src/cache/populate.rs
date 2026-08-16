use std::collections::{BTreeMap, HashMap, HashSet};
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
use crate::vault::{MARKER_FILE_NAME, NoteEntry, VaultIndex, normalize_title};

use super::SqliteCache;
use super::parse::{
    FileSnapshot, content_hash, current_unix_timestamp, extract_headings, extract_tags,
    file_snapshot, parse_frontmatter_metadata,
};

/// Build-time variables the benchmark can sweep. Production uses `Default`
/// (800/50 chunks, contextual documents); the eval harness overrides them per
/// cache to compare configurations. Model choice and vector dimension are
/// carried by the embedder itself (see the Matryoshka decorator), not here.
#[derive(Debug, Clone)]
pub struct BuildOptions {
    pub chunk: ChunkOptions,
    /// When true, each chunk embeds with its title + heading-path header (and
    /// hashes over that). When false, the pre-context behaviour: raw body only,
    /// body-only reuse hash — kept so the benchmark can measure the header's
    /// contribution in isolation.
    pub context: bool,
    /// Maximum number of same-note chunks submitted to one embedder call.
    /// Production keeps the conservative one-input default; eval can raise this
    /// to measure whether ONNX batching improves build throughput.
    pub embedding_batch_size: usize,
    /// When false, build every structural row (notes, links, tags, headings,
    /// chunk text) but embed nothing. This is the structure-only pass that
    /// publishes a browsable Vault ahead of its vectors; it reuses the existing
    /// per-note `embed` path rather than adding a second build.
    pub embed: bool,
}

impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            chunk: ChunkOptions::default(),
            context: true,
            embedding_batch_size: 1,
            embed: true,
        }
    }
}

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
    /// The file could not be read as text: non-UTF-8 bytes, a permission
    /// change, or a delete racing the scan that listed it. Reported rather
    /// than returned as `Err` so one unreadable file cannot abort indexing
    /// for the whole Vault — the same tradeoff the per-note embedding
    /// failure path below already makes.
    Unreadable {
        reason: String,
    },
}

const FIRST_PROGRESS_LOG_AFTER: Duration = Duration::from_secs(10);
const PROGRESS_LOG_INTERVAL: Duration = Duration::from_secs(60);

impl SqliteCache {
    /// Convenience entry point for callers with no progress reporting, who
    /// retain the historical default of embedding every layer.
    pub fn replace_from_index_with_embedder(
        &self,
        index: &VaultIndex,
        embedder: &dyn Embedder,
    ) -> Result<(), String> {
        self.replace_from_index_with_progress(index, embedder, None, true)
    }

    /// Populate with optional progress reporting and an explicit
    /// `embed_layers` (`HATCHDOOR_EMBED_LAYERS`) toggle. This is the entry
    /// point the server runtime uses; other callers pass `true` for the
    /// historical default of embedding every layer.
    pub fn replace_from_index_with_progress(
        &self,
        index: &VaultIndex,
        embedder: &dyn Embedder,
        on_progress: Option<Arc<dyn Fn(IndexingProgressSnapshot) + Send + Sync>>,
        embed_layers: bool,
    ) -> Result<(), String> {
        self.replace_with_options(
            index,
            embedder,
            on_progress,
            embed_layers,
            &BuildOptions::default(),
        )
    }

    /// Populate with explicit [`BuildOptions`] (chunk size, context toggle). The
    /// benchmark entry point; production paths use the defaulting wrappers.
    pub fn replace_from_index_with_options(
        &self,
        index: &VaultIndex,
        embedder: &dyn Embedder,
        opts: &BuildOptions,
    ) -> Result<(), String> {
        self.replace_with_options(index, embedder, None, true, opts)
    }

    /// Core populate. `embed_layers` (`HATCHDOOR_EMBED_LAYERS`, default true)
    /// controls whether demoted-layer notes get their vectors built. When false,
    /// demoted notes still get chunk rows (so keyword search works) but no vectors
    /// and no embedding work — the cost win the flag exists for. Demoted layers
    /// degrade to keyword-only, not to nothing.
    pub(crate) fn replace_with_options(
        &self,
        index: &VaultIndex,
        embedder: &dyn Embedder,
        on_progress: Option<Arc<dyn Fn(IndexingProgressSnapshot) + Send + Sync>>,
        embed_layers: bool,
        opts: &BuildOptions,
    ) -> Result<(), String> {
        let _epoch = self
            .snapshot_model_epoch
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.replace_with_options_unlocked(index, embedder, on_progress, embed_layers, opts, None)
    }

    /// Callers hold `snapshot_model_epoch`. `build_stamp` carries the metadata a
    /// stamped build records; it is written inside the populate transaction so a
    /// metadata failure cannot leave readers on a split generation.
    fn replace_with_options_unlocked(
        &self,
        index: &VaultIndex,
        embedder: &dyn Embedder,
        on_progress: Option<Arc<dyn Fn(IndexingProgressSnapshot) + Send + Sync>>,
        embed_layers: bool,
        opts: &BuildOptions,
        build_stamp: Option<BuildStamp>,
    ) -> Result<(), String> {
        // If the embedding model changed since the last build, rebuild from
        // scratch so no vectors from the old model are reused (mixed-model vector
        // spaces make cosine/L2 distances meaningless).
        self.reset_if_embedder_changed(embedder)?;

        // The embed-layers flag participates in the reindex the same way the
        // marker set does: flipping it changes no note's content or mtime, so the
        // incremental upsert would short-circuit and leave demoted notes either
        // permanently unembedded (after true→false) or permanently vector-less
        // (after false→true). Detect the flip and force every note back through
        // the write path so demoted vectors are (re)built or dropped to match.
        let embed_layers_value = if embed_layers { "true" } else { "false" };
        let stored_embed_layers = self.get_metadata("embed_layers")?;
        let embed_layers_changed = stored_embed_layers.is_some()
            && stored_embed_layers.as_deref() != Some(embed_layers_value);
        if embed_layers_changed {
            tracing::info!(
                embed_layers,
                "HATCHDOOR_EMBED_LAYERS changed; rebuilding demoted-layer vectors"
            );
        }

        // Adding, removing, renaming a layer or editing a marker description
        // changes no note's content or mtime, so the incremental upsert would
        // short-circuit to Unchanged and leave every note on its old
        // classification. Detect a marker-set change up front and, when it
        // changed, force every note row through the write path so `layer` is
        // rewritten. Read the stored hash before opening the write transaction
        // (get_metadata takes its own connection).
        let marker_set_hash = content_hash(&index.layers.hash_input());
        let stored_marker_set_hash = self.get_metadata("marker_set_hash")?;
        let marker_set_changed =
            stored_marker_set_hash.as_deref() != Some(marker_set_hash.as_str());
        if marker_set_changed && stored_marker_set_hash.is_some() {
            tracing::info!("Layer marker set changed; reclassifying every note");
        }

        // Guard against silent promotion: if a `.hatchdoor-layer` marker present
        // at the last index has vanished (a sync tool dropped the dotfile), keep
        // its notes on their prior layer rather than leaking them onto the default
        // surface. Read the persisted marker set before opening the write
        // transaction (get_metadata takes its own connection).
        let fresh_markers = index.layers.named_markers();
        let persisted_markers: BTreeMap<String, String> = self
            .get_metadata("marker_set")?
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();
        let mut entries = index.ordered_entries();
        let vanished_markers =
            retain_vanished_classifications(&mut entries, &persisted_markers, &fresh_markers);
        for marker in &vanished_markers {
            tracing::warn!(
                expected_marker = %marker.marker_path,
                layer = %marker.layer,
                notes = marker.note_count,
                "Layer marker file is missing; refusing to silently promote its notes to the \
                 default surface and retaining their prior classification. Reinstate the marker \
                 to restore it, or clear the persisted marker set to acknowledge the promotion."
            );
        }
        // Persist the effective marker set: fresh markers, plus any vanished ones
        // whose classification we are retaining, so the guard keeps firing until
        // the marker is reinstated or the persisted entry is cleared.
        let mut effective_markers = fresh_markers.clone();
        for (dir, name) in &persisted_markers {
            effective_markers
                .entry(dir.clone())
                .or_insert_with(|| name.clone());
        }
        let effective_markers_json = serde_json::to_string(&effective_markers)
            .map_err(|e| format!("failed serializing marker set: {e}"))?;
        let layer_catalog: Vec<crate::search::LayerInfo> = index
            .layers
            .layer_names()
            .into_iter()
            .map(|name| {
                let description = index.layers.description(&name).map(str::to_string);
                crate::search::LayerInfo { name, description }
            })
            .collect();
        let layer_catalog_json = serde_json::to_string(&layer_catalog)
            .map_err(|e| format!("failed serializing layer catalog: {e}"))?;

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
        let mut unreadable_notes: usize = 0;
        let mut last_unreadable_reason: Option<String> = None;
        let mut notes_changed: usize = 0;
        let mut notes_unchanged: usize = 0;
        let mut metrics = IndexingMetrics::default();
        let mut prepared_notes = Vec::new();

        // Chunk and measure changed notes up front. Chunking was less than 0.5%
        // of the measured full-vault runtime, and retaining these results gives
        // the heartbeat an exact embedding-work denominator without chunking
        // notes twice.
        let force_note_refresh = marker_set_changed || embed_layers_changed;
        for entry in &entries {
            let note_sync_started = Instant::now();
            let upsert_outcome = upsert_note_if_changed(&tx, entry, now, force_note_refresh)?;
            metrics.note_sync += note_sync_started.elapsed();
            // A demoted note is embedded only when the flag allows it; a
            // default-surface note is always embedded. A structure-only build
            // (`opts.embed == false`) embeds neither.
            let embed_this_note = opts.embed && (embed_layers || entry.layer.is_none());
            match upsert_outcome {
                UpsertOutcome::Wrote { slug, content } => {
                    match prepare_note_for_embedding(
                        &tx,
                        slug,
                        &entry.title,
                        content,
                        entry.layer.clone(),
                        embed_this_note,
                        embedder,
                        opts,
                    ) {
                        Ok(prepared) => prepared_notes.push(prepared),
                        Err(error) => {
                            per_note_failures += 1;
                            tracing::warn!(slug = %entry.slug, error = %error, "Per-note embedding preparation failed; marking note for re-embed on next reindex");
                            invalidate_note_content_hash(&tx, &entry.slug)?;
                        }
                    }
                }
                UpsertOutcome::Unchanged => notes_unchanged += 1,
                UpsertOutcome::Unreadable { reason } => {
                    per_note_failures += 1;
                    unreadable_notes += 1;
                    last_unreadable_reason = Some(reason);
                    tracing::warn!(
                        path = %entry.relative_path,
                        "Skipping unreadable note; the rest of the Vault still indexes"
                    );
                }
            }
        }

        // Tolerating individual unreadable files must not extend to a Vault
        // that has become unreadable as a whole — an unmounted volume or a
        // revoked permission would otherwise publish an empty index over a
        // good one. Losing every note the scan just listed is that case, so it
        // stays fatal and the prior snapshot is retained as stale. A Vault
        // that legitimately holds no notes has nothing to lose and succeeds.
        if !entries.is_empty() && unreadable_notes == entries.len() {
            return Err(format!(
                "every note in the Vault became unreadable during indexing ({} of {}); \
                 the previous index is kept. Last error: {}",
                unreadable_notes,
                entries.len(),
                last_unreadable_reason.unwrap_or_else(|| "unknown".to_string())
            ));
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

        // A cache refresh is one generation. Its metadata participates in the
        // same transaction as notes and every derived row so a metadata-write
        // failure cannot make readers observe a split generation.
        let embedder_id = build_stamp
            .as_ref()
            .map(|stamp| stamp.embedder_id.clone())
            .unwrap_or_else(|| embedder.identity());
        set_metadata_in_transaction(&tx, "embedder_id", &embedder_id)?;
        set_metadata_in_transaction(&tx, "marker_set_hash", &marker_set_hash)?;
        set_metadata_in_transaction(&tx, "marker_set", &effective_markers_json)?;
        set_metadata_in_transaction(&tx, "embed_layers", embed_layers_value)?;
        set_metadata_in_transaction(&tx, "layer_catalog", &layer_catalog_json)?;
        if let Some(stamp) = build_stamp {
            set_metadata_in_transaction(
                &tx,
                "build_duration_secs",
                &format!("{:.3}", stamp.started_at.elapsed().as_secs_f64()),
            )?;
        }

        let commit_started = Instant::now();
        tx.commit()
            .map_err(|e| format!("failed to commit SQLite cache refresh: {e}"))?;
        metrics.commit = commit_started.elapsed();

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
        self.replace_from_index_with_options_stamped(
            index,
            embedder,
            embedder_id,
            &BuildOptions::default(),
        )
    }

    /// Stamped populate with explicit [`BuildOptions`]. Records the model id and
    /// build duration alongside the vectors so a benchmark cache is
    /// self-describing.
    pub fn replace_from_index_with_options_stamped(
        &self,
        index: &VaultIndex,
        embedder: &dyn Embedder,
        embedder_id: &str,
        opts: &BuildOptions,
    ) -> Result<(), String> {
        let _epoch = self
            .snapshot_model_epoch
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.replace_with_options_unlocked(
            index,
            embedder,
            None,
            true,
            opts,
            Some(BuildStamp {
                embedder_id: embedder_id.to_string(),
                started_at: Instant::now(),
            }),
        )
    }
}

struct BuildStamp {
    embedder_id: String,
    started_at: Instant,
}

fn set_metadata_in_transaction(tx: &Transaction<'_>, key: &str, value: &str) -> Result<(), String> {
    tx.execute(
        "INSERT INTO metadata(key, value) VALUES (?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )
    .map_err(|error| format!("set build metadata '{key}': {error}"))?;
    Ok(())
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
                .extend(stats.embedding_call_input_counts.iter().copied());
            self.embedding_call_token_counts
                .extend(stats.embedding_call_token_counts.iter().copied());
            self.embedding_call_padded_token_counts
                .extend(stats.embedding_call_padded_token_counts.iter().copied());
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
        while let Err(mpsc::RecvTimeoutError::Timeout) =
            stop_rx.recv_timeout(progress_log_delay(has_logged))
        {
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
        if position > 0 && (digits.len() - position).is_multiple_of(3) {
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
    // The note row has already switched to the new Markdown content. Remove the
    // old derived rows before committing that fact, otherwise search could pair
    // the changed note with chunks/vectors built from the previous body.
    replace_chunks_for_note(tx, slug, None, &[], None, None)?;
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

/// A marker that was in the persisted set but is absent from the freshly
/// collected one, plus the count of notes whose classification is being retained
/// (rather than silently promoted to the default surface).
#[derive(Debug, PartialEq, Eq)]
struct VanishedMarker {
    marker_path: String,
    layer: String,
    note_count: usize,
}

/// True when `relative_path` sits inside (or is) the marker directory `dir`.
fn path_is_under(relative_path: &str, dir: &str) -> bool {
    dir.is_empty() || relative_path == dir || relative_path.starts_with(&format!("{dir}/"))
}

/// Guard against silent promotion. A `.hatchdoor-layer` dotfile can be dropped
/// invisibly by sync tooling; if its notes were then reclassified onto the
/// default surface they would leak into every default search, tree and graph.
///
/// For each marker present in the persisted set but gone from the fresh one,
/// any note that the fresh index would place on the default surface (`layer =
/// None`) but that still sits under the vanished marker's directory keeps the
/// marker's layer. Notes the fresh index still classifies (nearest surviving
/// marker) are left alone — a reclassification between two live layers is not a
/// promotion. Returns one report per vanished marker so the caller can log it.
fn retain_vanished_classifications(
    entries: &mut [NoteEntry],
    persisted_markers: &BTreeMap<String, String>,
    fresh_markers: &BTreeMap<String, String>,
) -> Vec<VanishedMarker> {
    let vanished: BTreeMap<&String, &String> = persisted_markers
        .iter()
        .filter(|(dir, _)| !fresh_markers.contains_key(*dir))
        .collect();
    if vanished.is_empty() {
        return Vec::new();
    }

    for entry in entries.iter_mut() {
        if entry.layer.is_some() {
            continue;
        }
        // Longest-prefix wins, matching nearest-marker resolution.
        let mut best: Option<(&str, usize)> = None;
        for (dir, name) in &vanished {
            if path_is_under(&entry.relative_path, dir)
                && best.is_none_or(|(_, len)| dir.len() >= len)
            {
                best = Some((name.as_str(), dir.len()));
            }
        }
        if let Some((name, _)) = best {
            entry.layer = Some(name.to_string());
        }
    }

    vanished
        .into_iter()
        .map(|(dir, name)| {
            let note_count = entries
                .iter()
                .filter(|entry| {
                    entry.layer.as_deref() == Some(name.as_str())
                        && path_is_under(&entry.relative_path, dir)
                })
                .count();
            let marker_path = if dir.is_empty() {
                MARKER_FILE_NAME.to_string()
            } else {
                format!("{dir}/{MARKER_FILE_NAME}")
            };
            VanishedMarker {
                marker_path,
                layer: name.clone(),
                note_count,
            }
        })
        .collect()
}

fn upsert_note_if_changed(
    tx: &Transaction<'_>,
    entry: &NoteEntry,
    indexed_at: i64,
    force_layer_refresh: bool,
) -> Result<UpsertOutcome, String> {
    let snapshot = match file_snapshot(&entry.path) {
        Ok(snapshot) => snapshot,
        Err(reason) => return Ok(UpsertOutcome::Unreadable { reason }),
    };
    let cached = cached_note_state(tx, &entry.relative_path)?;

    let content = match fs::read_to_string(&entry.path) {
        Ok(content) => content,
        Err(error) => {
            return Ok(UpsertOutcome::Unreadable {
                reason: format!("failed reading note '{}': {error}", entry.path.display()),
            });
        }
    };
    let hash = content_hash(&content);

    // When the marker set changed, the note's `layer` may differ even though its
    // content, mtime and slug are identical. `layer` is only written on the full
    // write path (`upsert_note_content`), so both incremental short-circuits must
    // be skipped to let the new classification land.
    if !force_layer_refresh {
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
    let frontmatter = parse_frontmatter_metadata(content).unwrap_or_else(|error| {
        tracing::warn!(slug = %entry.slug, error = %error, "Ignoring malformed YAML frontmatter");
        Default::default()
    });
    let aliases_json = serde_json::to_string(&frontmatter.aliases)
        .map_err(|error| format!("failed serializing aliases for '{}': {error}", entry.slug))?;
    let frontmatter_json = serde_json::to_string(&frontmatter.properties).map_err(|error| {
        format!(
            "failed serializing frontmatter properties for '{}': {error}",
            entry.slug
        )
    })?;

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
            layer,
            aliases_json,
            frontmatter_json,
            mtime_ns,
            size_bytes,
            indexed_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
        ON CONFLICT(relative_path) DO UPDATE SET
            slug = excluded.slug,
            title = excluded.title,
            normalized_title = excluded.normalized_title,
            normalized_relative_path = excluded.normalized_relative_path,
            absolute_path = excluded.absolute_path,
            content = excluded.content,
            content_hash = excluded.content_hash,
            layer = excluded.layer,
            aliases_json = excluded.aliases_json,
            frontmatter_json = excluded.frontmatter_json,
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
            &entry.layer,
            &aliases_json,
            &frontmatter_json,
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
    embedding_call_input_counts: Vec<usize>,
    embedding_call_token_counts: Vec<usize>,
    embedding_call_padded_token_counts: Vec<usize>,
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
    /// The note's layer, routing its vectors to the default or demoted table.
    layer: Option<String>,
    /// Whether this note is embedded at all. False for a demoted note under
    /// `HATCHDOOR_EMBED_LAYERS=false`: chunk rows are written for keyword search
    /// but no vectors are produced or stored.
    embed: bool,
    chunking: NoteChunking,
    preserved: HashMap<String, Vec<f32>>,
    texts_to_embed: Vec<String>,
    indices_needing_embed: Vec<usize>,
    embedding_input_bytes: usize,
    embedding_input_token_lengths: Vec<usize>,
    embedding_batch_size: usize,
    chunk_measurements: Vec<ChunkMeasurement>,
    chunking_elapsed: Duration,
    vector_reuse_elapsed: Duration,
}

/// Reuse/change-detection hash for Hatchdoor's canonical contextual document.
/// This is retained for tests; production vector reuse hashes the complete
/// model-formatted embedding input in [`chunk_reuse_hash`].
#[cfg(test)]
fn embedding_reuse_hash(title: &str, heading_path: Option<&str>, body: &str) -> String {
    let doc = crate::embed::contextual_document(title, heading_path, body);
    blake3::hash(doc.as_bytes()).to_hex().to_string()
}

/// Vector-reuse key for a chunk under the active build options: header-inclusive
/// when contextual embedding is on, body-only when off. The two representations
/// must never collide in a shared cache, so they hash different inputs.
fn chunk_reuse_hash(
    context: bool,
    embedder: &dyn Embedder,
    title: &str,
    heading_path: Option<&str>,
    body: &str,
) -> String {
    let input = chunk_embed_input(context, embedder, title, heading_path, body);
    blake3::hash(input.as_bytes()).to_hex().to_string()
}

/// The document text embedded for a chunk under the active build options.
fn chunk_embed_input(
    context: bool,
    embedder: &dyn Embedder,
    title: &str,
    heading_path: Option<&str>,
    body: &str,
) -> String {
    if context {
        embedder.document_input(title, heading_path, body)
    } else {
        format!("{}{}", embedder.doc_prefix(), body)
    }
}

// Each parameter is a distinct, independently-sourced input (transaction, note
// identity fields, the embedder, and the build options); bundling them into a
// struct purely to satisfy the lint would add ceremony without clarity.
#[allow(clippy::too_many_arguments)]
fn prepare_note_for_embedding(
    tx: &Transaction<'_>,
    slug: String,
    title: &str,
    content: String,
    layer: Option<String>,
    embed: bool,
    embedder: &dyn Embedder,
    opts: &BuildOptions,
) -> Result<PreparedNote, String> {
    let chunking_started = Instant::now();
    let mut chunking = chunk_note(&content, embedder, opts.chunk);
    // Rehash each chunk over its *embedded input* (title + heading path + body
    // when contextual), not just its body. This is the vector-reuse key, so a
    // heading edit — which changes a downstream chunk's heading path without
    // touching its body — invalidates the cached vector instead of silently
    // reusing a stale one.
    for chunk in &mut chunking.chunks {
        chunk.content_hash = chunk_reuse_hash(
            opts.context,
            embedder,
            title,
            chunk.heading_path.as_deref(),
            &chunk.content,
        );
    }
    let chunking_elapsed = chunking_started.elapsed();

    // A note we are not embedding still gets chunk rows (keyword search) but no
    // vector work: skip reuse, measurement and the embed list entirely.
    if !embed {
        return Ok(PreparedNote {
            slug,
            layer,
            embed,
            chunking,
            preserved: HashMap::new(),
            texts_to_embed: Vec::new(),
            indices_needing_embed: Vec::new(),
            embedding_input_bytes: 0,
            embedding_input_token_lengths: Vec::new(),
            embedding_batch_size: opts.embedding_batch_size,
            chunk_measurements: Vec::new(),
            chunking_elapsed,
            vector_reuse_elapsed: Duration::ZERO,
        });
    }

    let reuse_started = Instant::now();
    let existing = existing_chunk_hashes(tx, &slug)?;
    let preserved =
        preserve_existing_vectors(tx, &slug, layer.as_deref(), &chunking.chunks, &existing)?;
    let vector_reuse_elapsed = reuse_started.elapsed();

    let chunk_measurements = chunking
        .chunks
        .iter()
        .map(|chunk| {
            let input = chunk_embed_input(
                opts.context,
                embedder,
                title,
                chunk.heading_path.as_deref(),
                &chunk.content,
            );
            let input_tokens = embedder
                .token_count(input.as_str(), true)
                .map_err(|error| format!("failed measuring tokens for '{slug}': {error}"))?;
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
            texts_to_embed.push(chunk_embed_input(
                opts.context,
                embedder,
                title,
                chunk.heading_path.as_deref(),
                &chunk.content,
            ));
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
        layer,
        embed,
        chunking,
        preserved,
        texts_to_embed,
        indices_needing_embed,
        embedding_input_bytes,
        embedding_input_token_lengths,
        embedding_batch_size: opts.embedding_batch_size,
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
        layer,
        embed,
        chunking,
        preserved,
        texts_to_embed,
        indices_needing_embed,
        embedding_input_bytes,
        embedding_input_token_lengths,
        embedding_batch_size,
        chunk_measurements,
        chunking_elapsed,
        vector_reuse_elapsed,
    } = prepared;
    if chunking.chunks.is_empty() {
        let sqlite_started = Instant::now();
        replace_chunks_for_note(tx, &slug, layer.as_deref(), &[], None, None)?;
        return Ok(ChunkStats {
            embedded: 0,
            reused: 0,
            embedder_calls: 0,
            embedding_input_bytes: 0,
            embedding_input_tokens: 0,
            embedding_padded_tokens: 0,
            embedding_input_token_lengths: Vec::new(),
            embedding_call_input_counts: Vec::new(),
            embedding_call_token_counts: Vec::new(),
            embedding_call_padded_token_counts: Vec::new(),
            embedding_call_durations: Vec::new(),
            chunk_measurements: Vec::new(),
            pipeline: pipeline_started.elapsed() + chunking_elapsed + vector_reuse_elapsed,
            chunking: chunking_elapsed,
            vector_reuse: vector_reuse_elapsed,
            embedding: Duration::ZERO,
            sqlite_write: sqlite_started.elapsed(),
        });
    }

    // Not embedded (a demoted note under HATCHDOOR_EMBED_LAYERS=false): write the
    // chunk rows so keyword/FTS search still finds it, but store no vectors.
    if !embed {
        let tags_json = serde_json::to_string(&chunking.tags).ok();
        let aliases_json = serde_json::to_string(&chunking.aliases).ok();
        let rows: Vec<ChunkRow<'_>> = chunking
            .chunks
            .iter()
            .map(|chunk| ChunkRow {
                chunk,
                vector: None,
            })
            .collect();
        let sqlite_started = Instant::now();
        replace_chunks_for_note(
            tx,
            &slug,
            layer.as_deref(),
            &rows,
            tags_json.as_deref(),
            aliases_json.as_deref(),
        )?;
        return Ok(ChunkStats {
            embedded: 0,
            reused: 0,
            embedder_calls: 0,
            embedding_input_bytes: 0,
            embedding_input_tokens: 0,
            embedding_padded_tokens: 0,
            embedding_input_token_lengths: Vec::new(),
            embedding_call_input_counts: Vec::new(),
            embedding_call_token_counts: Vec::new(),
            embedding_call_padded_token_counts: Vec::new(),
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
    // The backend pads a batch to its longest input. Keep the batches within a
    // note so vectors retain their original chunk order; the eval harness uses
    // this controlled variant before considering a larger cross-note scheduler.
    let batch_size = embedding_batch_size.max(1);
    let mut embedding_padded_tokens = 0;
    let embedding_started = Instant::now();
    let mut new_vectors = Vec::with_capacity(texts_to_embed.len());
    let calls = texts_to_embed.len().div_ceil(batch_size);
    let mut embedding_call_durations = Vec::with_capacity(calls);
    let mut embedding_call_input_counts = Vec::with_capacity(calls);
    let mut embedding_call_token_counts = Vec::with_capacity(calls);
    let mut embedding_call_padded_token_counts = Vec::with_capacity(calls);
    for (texts, token_lengths) in texts_to_embed
        .chunks(batch_size)
        .zip(embedding_input_token_lengths.chunks(batch_size))
    {
        let input_tokens: usize = token_lengths.iter().sum();
        let padded_tokens = token_lengths.iter().copied().max().unwrap_or(0) * texts.len();
        let call_started = Instant::now();
        let vectors = embedder.embed(texts)?;
        embedding_call_durations.push(call_started.elapsed());
        if vectors.len() != texts.len() {
            return Err(format!(
                "embedder returned {} vectors for {} inputs",
                vectors.len(),
                texts.len()
            ));
        }
        embedding_padded_tokens += padded_tokens;
        embedding_call_input_counts.push(texts.len());
        embedding_call_token_counts.push(input_tokens);
        embedding_call_padded_token_counts.push(padded_tokens);
        new_vectors.extend(vectors);
        progress
            .chunks_processed
            .fetch_add(texts.len(), Ordering::Relaxed);
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
            calls,
            batch_size,
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
        .map(|(chunk, vector)| ChunkRow {
            chunk,
            vector: Some(vector.as_slice()),
        })
        .collect();

    let sqlite_started = Instant::now();
    replace_chunks_for_note(
        tx,
        &slug,
        layer.as_deref(),
        &rows,
        tags_json.as_deref(),
        aliases_json.as_deref(),
    )?;
    Ok(ChunkStats {
        embedded: indices_needing_embed.len(),
        reused: chunking.chunks.len() - indices_needing_embed.len(),
        embedder_calls: calls,
        embedding_input_bytes,
        embedding_input_tokens,
        embedding_padded_tokens,
        embedding_input_token_lengths,
        embedding_call_input_counts,
        embedding_call_token_counts,
        embedding_call_padded_token_counts,
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
    layer: Option<&str>,
    chunks: &[crate::chunk::Chunk],
    existing: &std::collections::HashMap<String, i64>,
) -> Result<std::collections::HashMap<String, Vec<f32>>, String> {
    let mut out = std::collections::HashMap::new();
    // A note's vectors live in the table its layer routes to, so read from the
    // matching one. A missing row is not an error: it means the chunk was never
    // vectored (e.g. a demoted note built while HATCHDOOR_EMBED_LAYERS=false, now
    // being embedded), so it simply falls through to a fresh embed.
    let table = match layer {
        None => "chunk_vectors",
        Some(_) => "chunk_vectors_demoted",
    };
    let mut stmt = tx
        .prepare(&format!(
            "SELECT embedding FROM {table} WHERE chunk_id = ?1"
        ))
        .map_err(|e| format!("prepare vector lookup: {e}"))?;
    for chunk in chunks {
        if let Some(chunk_id) = existing.get(&chunk.content_hash) {
            let bytes: Option<Vec<u8>> = stmt
                .query_row(rusqlite::params![chunk_id], |row| row.get(0))
                .optional()
                .map_err(|e| format!("read preserved vector: {e}"))?;
            if let Some(bytes) = bytes {
                let floats: Vec<f32> = bytemuck::cast_slice(&bytes).to_vec();
                out.insert(chunk.content_hash.clone(), floats);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::SqliteCache;
    use crate::embed::{Embedder, StubEmbedder};
    use crate::search::LayerSelection;
    use crate::vault::VaultIndex;
    use std::sync::Arc;
    use tempfile::tempdir;

    /// Build a vault with a demoted `sources/` note and index it with an explicit
    /// `embed_layers` flag.
    fn demoted_vault_with_flag(
        embed_layers: bool,
    ) -> (tempfile::TempDir, SqliteCache, StubEmbedder) {
        let dir = tempdir().expect("temp dir");
        std::fs::create_dir_all(dir.path().join("sources")).expect("sources dir");
        std::fs::write(
            dir.path().join("sources/Clip.md"),
            "# Clip\n\nmelatonin regulates the circadian rhythm",
        )
        .expect("note");
        std::fs::write(dir.path().join("sources/.hatchdoor-layer"), "sources").expect("marker");

        let cache = SqliteCache::in_memory(384).expect("cache");
        let embedder = StubEmbedder::new(384);
        let index = VaultIndex::build(dir.path()).expect("index");
        cache
            .replace_with_options(
                &index,
                &embedder,
                None,
                embed_layers,
                &BuildOptions::default(),
            )
            .expect("populate");
        (dir, cache, embedder)
    }

    fn sources_selection() -> LayerSelection {
        let (selection, _) =
            LayerSelection::parse(&["sources".to_string()], &["sources".to_string()]);
        selection
    }

    #[test]
    fn populate_persists_layer_catalog_with_names_and_descriptions() {
        // The MCP surface (Group D) generates its `layers` enum and per-value
        // docs at request time, when only the SQLite cache is reachable — the
        // in-memory LayerMap is long gone. So populate must persist the vault's
        // layer names and descriptions for the tool-list builder to read back.
        let dir = tempdir().expect("temp dir");
        std::fs::create_dir_all(dir.path().join("sources")).expect("sources dir");
        std::fs::create_dir_all(dir.path().join("archive")).expect("archive dir");
        std::fs::write(
            dir.path().join("sources/.hatchdoor-layer"),
            "name: sources\ndescription: Raw captured clippings.\n",
        )
        .expect("sources marker");
        // A second layer with no description exercises the None case.
        std::fs::write(dir.path().join("archive/.hatchdoor-layer"), "archive").expect("marker");
        std::fs::write(dir.path().join("sources/Clip.md"), "# Clip\n\nbody").expect("note");

        let cache = SqliteCache::in_memory(384).expect("cache");
        let embedder = StubEmbedder::new(384);
        let index = VaultIndex::build(dir.path()).expect("index");
        cache
            .replace_from_index_with_embedder(&index, &embedder)
            .expect("populate");

        let catalog = cache.layer_catalog().expect("layer catalog");
        assert_eq!(
            catalog,
            vec![
                crate::search::LayerInfo {
                    name: "archive".to_string(),
                    description: None,
                },
                crate::search::LayerInfo {
                    name: "sources".to_string(),
                    description: Some("Raw captured clippings.".to_string()),
                },
            ],
            "catalog must carry every discovered layer, sorted, with its description"
        );
    }

    #[test]
    fn layer_catalog_is_empty_for_a_vault_with_no_markers() {
        let dir = tempdir().expect("temp dir");
        std::fs::write(dir.path().join("Home.md"), "# Home").expect("note");
        let cache = SqliteCache::in_memory(384).expect("cache");
        let embedder = StubEmbedder::new(384);
        let index = VaultIndex::build(dir.path()).expect("index");
        cache
            .replace_from_index_with_embedder(&index, &embedder)
            .expect("populate");
        assert!(
            cache.layer_catalog().expect("catalog").is_empty(),
            "a vault with no layer markers advertises no layers"
        );
    }

    #[test]
    fn embed_layers_false_skips_demoted_vectors_but_keeps_keyword_search() {
        let (_dir, cache, embedder) = demoted_vault_with_flag(false);

        // No demoted vectors were built: a layer semantic search finds nothing.
        let semantic = cache
            .semantic_search_layered(&embedder, "melatonin circadian", 10, &sources_selection())
            .expect("semantic");
        assert!(
            semantic.is_empty(),
            "HATCHDOOR_EMBED_LAYERS=false must leave demoted layers without vectors: {:?}",
            semantic.iter().map(|h| &h.note_slug).collect::<Vec<_>>()
        );
        // The demoted vector table is genuinely empty.
        let demoted_count: i64 = cache
            .read()
            .expect("read")
            .query_row("SELECT COUNT(*) FROM chunk_vectors_demoted", [], |r| {
                r.get(0)
            })
            .expect("count");
        assert_eq!(demoted_count, 0);

        // Keyword search still finds the demoted note (chunk rows were written).
        let keyword = cache
            .fts_search_chunks_layered("melatonin", 10, &sources_selection())
            .expect("keyword");
        assert!(
            keyword.iter().any(|h| h.note_slug == "clip"),
            "keyword search over the demoted layer must still find the note: {:?}",
            keyword.iter().map(|h| &h.note_slug).collect::<Vec<_>>()
        );
    }

    #[test]
    fn flipping_embed_layers_false_to_true_re_embeds_demoted_layers() {
        // The flag must participate in the reindex: flipping it back to true must
        // actually build the demoted vectors, not leave them permanently empty
        // because no note's content changed.
        let (dir, cache, embedder) = demoted_vault_with_flag(false);
        assert!(
            cache
                .semantic_search_layered(&embedder, "melatonin", 10, &sources_selection())
                .expect("semantic")
                .is_empty(),
            "precondition: no demoted vectors while the flag is false"
        );

        // Reindex the SAME vault (no content change) with the flag now true.
        let index = VaultIndex::build(dir.path()).expect("reindex");
        cache
            .replace_with_options(&index, &embedder, None, true, &BuildOptions::default())
            .expect("re-embed");

        let semantic = cache
            .semantic_search_layered(&embedder, "melatonin", 10, &sources_selection())
            .expect("semantic after flip");
        assert!(
            semantic.iter().any(|h| h.note_slug == "clip"),
            "flipping the flag back to true must re-embed the demoted layer: {:?}",
            semantic.iter().map(|h| &h.note_slug).collect::<Vec<_>>()
        );
    }

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
            .replace_from_index_with_progress(&index, &StubEmbedder::new(384), Some(observer), true)
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

    /// Reads the `layer` column for a note by slug directly from the notes table.
    #[cfg(test)]
    fn read_layer_for_slug(cache: &SqliteCache, slug: &str) -> Option<String> {
        let conn = cache.connection().expect("connection");
        conn.query_row(
            "SELECT layer FROM notes WHERE slug = ?1",
            params![slug],
            |row| row.get::<_, Option<String>>(0),
        )
        .expect("query layer")
    }

    #[test]
    fn populate_writes_note_layer_from_index() {
        let dir = tempdir().expect("temp dir");
        std::fs::create_dir_all(dir.path().join("sources")).expect("dirs");
        std::fs::create_dir_all(dir.path().join("wiki")).expect("dirs");
        std::fs::write(dir.path().join("sources/.hatchdoor-layer"), "sources").expect("marker");
        std::fs::write(dir.path().join("sources/Clip.md"), "# Clip\nraw source").expect("note");
        std::fs::write(dir.path().join("wiki/Page.md"), "# Page\ncompiled").expect("note");

        let index = VaultIndex::build(dir.path()).expect("index");
        let cache = SqliteCache::in_memory(384).expect("cache");
        cache
            .replace_from_index_with_embedder(&index, &StubEmbedder::new(384))
            .expect("populate");

        // The demoted note carries its layer name; the default-surface note is NULL.
        assert_eq!(
            read_layer_for_slug(&cache, "clip").as_deref(),
            Some("sources"),
            "demoted note must record layer = 'sources'"
        );
        assert_eq!(
            read_layer_for_slug(&cache, "page"),
            None,
            "default-surface note must record layer IS NULL"
        );
    }

    #[test]
    fn a_vanished_marker_does_not_silently_promote_its_notes() {
        // `.hatchdoor-layer` is a dotfile that sync tools drop invisibly. If the
        // marker disappears, promoting its (possibly thousands of) notes onto the
        // default surface is the modal silent failure. The reindex must refuse to
        // promote and retain the prior classification instead.
        let dir = tempdir().expect("temp dir");
        std::fs::create_dir_all(dir.path().join("sources")).expect("dirs");
        std::fs::write(dir.path().join("sources/Clip.md"), "# Clip\nraw source").expect("note");
        let marker = dir.path().join("sources/.hatchdoor-layer");
        std::fs::write(&marker, "sources").expect("marker");

        let cache = SqliteCache::in_memory(384).expect("cache");
        let embedder = StubEmbedder::new(384);

        let index = VaultIndex::build(dir.path()).expect("index");
        cache
            .replace_from_index_with_embedder(&index, &embedder)
            .expect("initial populate");
        assert_eq!(
            read_layer_for_slug(&cache, "clip").as_deref(),
            Some("sources"),
            "precondition: note is demoted while the marker exists"
        );

        // The marker vanishes (a sync tool dropped it).
        std::fs::remove_file(&marker).expect("remove marker");
        let index = VaultIndex::build(dir.path()).expect("reindex");
        assert_eq!(
            index.by_slug["clip"].layer, None,
            "precondition: the fresh index would promote the note to the default surface"
        );
        cache
            .replace_from_index_with_embedder(&index, &embedder)
            .expect("reindex populate");

        // The note must keep its prior classification, not be silently promoted.
        assert_eq!(
            read_layer_for_slug(&cache, "clip").as_deref(),
            Some("sources"),
            "a vanished marker must not silently promote its notes"
        );
    }

    #[test]
    fn retain_vanished_classifications_reports_and_overrides() {
        let mut entries = vec![
            NoteEntry {
                title: "Clip".to_string(),
                slug: "clip".to_string(),
                path: "/x/sources/Clip.md".into(),
                relative_path: "sources/Clip.md".to_string(),
                layer: None,
            },
            NoteEntry {
                title: "Page".to_string(),
                slug: "page".to_string(),
                path: "/x/wiki/Page.md".into(),
                relative_path: "wiki/Page.md".to_string(),
                layer: None,
            },
        ];
        let persisted: BTreeMap<String, String> = [("sources".to_string(), "sources".to_string())]
            .into_iter()
            .collect();
        let fresh: BTreeMap<String, String> = BTreeMap::new();

        let report = retain_vanished_classifications(&mut entries, &persisted, &fresh);

        assert_eq!(report.len(), 1);
        assert_eq!(report[0].marker_path, "sources/.hatchdoor-layer");
        assert_eq!(report[0].layer, "sources");
        assert_eq!(report[0].note_count, 1);
        // The note under the vanished marker retains its layer; the unrelated
        // default-surface note is untouched.
        assert_eq!(entries[0].layer.as_deref(), Some("sources"));
        assert_eq!(entries[1].layer, None);
    }

    #[test]
    fn retain_vanished_classifications_noop_when_nothing_vanished() {
        let mut entries = vec![NoteEntry {
            title: "Clip".to_string(),
            slug: "clip".to_string(),
            path: "/x/sources/Clip.md".into(),
            relative_path: "sources/Clip.md".to_string(),
            layer: Some("sources".to_string()),
        }];
        let markers: BTreeMap<String, String> = [("sources".to_string(), "sources".to_string())]
            .into_iter()
            .collect();

        let report = retain_vanished_classifications(&mut entries, &markers, &markers);
        assert!(report.is_empty());
        assert_eq!(entries[0].layer.as_deref(), Some("sources"));
    }

    #[test]
    fn adding_a_marker_reclassifies_notes_without_any_note_edit() {
        // The whole feature no-ops without this. `upsert_note_if_changed` returns
        // Unchanged on matching slug+mtime+content-hash. Adding a `.hatchdoor-layer`
        // marker changes no note's content or mtime, so without the marker-set-hash
        // guard every note keeps `layer = NULL` after the user demotes a folder.
        let dir = tempdir().expect("temp dir");
        std::fs::create_dir_all(dir.path().join("sources")).expect("dirs");
        let note_path = dir.path().join("sources/Clip.md");
        std::fs::write(&note_path, "# Clip\nraw source").expect("note");

        let cache = SqliteCache::in_memory(384).expect("cache");
        let embedder = StubEmbedder::new(384);

        // First pass: no marker anywhere, so the note is on the default surface.
        let index = VaultIndex::build(dir.path()).expect("index");
        cache
            .replace_from_index_with_embedder(&index, &embedder)
            .expect("initial populate");
        assert_eq!(
            read_layer_for_slug(&cache, "clip"),
            None,
            "precondition: default-surface note starts NULL"
        );

        // Add a marker WITHOUT touching the note's content or mtime. Dropping a
        // file into a directory does not change the mtime of the files already in
        // it, so the incremental path would short-circuit to Unchanged.
        let mtime_before = file_snapshot(&note_path).expect("snapshot").mtime_ns;
        std::fs::write(dir.path().join("sources/.hatchdoor-layer"), "sources").expect("marker");
        let mtime_after = file_snapshot(&note_path).expect("snapshot").mtime_ns;
        assert_eq!(
            mtime_before, mtime_after,
            "precondition: the note file was not touched"
        );

        // Second pass: the marker set changed, so the note must be reclassified.
        let index = VaultIndex::build(dir.path()).expect("reindex");
        cache
            .replace_from_index_with_embedder(&index, &embedder)
            .expect("reindex populate");
        assert_eq!(
            read_layer_for_slug(&cache, "clip").as_deref(),
            Some("sources"),
            "adding a marker must force the note's layer to be rewritten"
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
        BuildOptions, embedding_reuse_hash, estimated_remaining, format_count, format_elapsed,
        format_eta, format_note_count, indexing_progress_message, progress_log_delay,
    };
    use crate::cache::SqliteCache;
    use crate::chunk::ChunkOptions;
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
        fn identity(&self) -> String {
            self.inner.identity()
        }
        fn token_count(&self, text: &str, add_special_tokens: bool) -> Result<usize, String> {
            self.inner.token_count(text, add_special_tokens)
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

    #[test]
    fn changed_note_embed_failure_never_leaves_old_chunks_current() {
        let dir = make_vault(&[("a.md", "# A\n\nold searchable body")]);
        let cache = SqliteCache::in_memory(384).expect("open");
        let working = StubEmbedder::new(384);
        let initial = VaultIndex::build(dir.path()).expect("initial index");
        cache
            .replace_from_index_with_embedder(&initial, &working)
            .expect("initial populate");
        assert!(
            note_chunk_count(&cache, "a") > 0,
            "precondition: old chunks exist"
        );

        std::fs::write(
            dir.path().join("a.md"),
            "# A\n\nnew body whose embedding fails",
        )
        .expect("change note");
        let changed = VaultIndex::build(dir.path()).expect("changed index");
        let failing = FailingEmbedder {
            inner: StubEmbedder::new(384),
        };
        cache
            .replace_from_index_with_embedder(&changed, &failing)
            .expect("a per-note failure remains recoverable");

        assert_eq!(
            note_chunk_count(&cache, "a"),
            0,
            "a changed note with failed embeddings must not publish old chunks as current"
        );
    }

    #[test]
    fn metadata_write_failure_rolls_back_the_entire_refresh_generation() {
        let dir = make_vault(&[("a.md", "# Old\n\nold body")]);
        let cache = SqliteCache::in_memory(384).expect("open");
        let embedder = StubEmbedder::new(384);
        let initial = VaultIndex::build(dir.path()).expect("initial index");
        cache
            .replace_from_index_with_embedder(&initial, &embedder)
            .expect("initial populate");

        std::fs::write(dir.path().join("a.md"), "# New\n\nnew body").expect("change note");
        let changed = VaultIndex::build(dir.path()).expect("changed index");
        {
            let conn = cache.connection().expect("connection");
            conn.execute_batch(
                r#"
                CREATE TRIGGER fail_marker_metadata
                BEFORE UPDATE OF value ON metadata
                WHEN NEW.key = 'marker_set_hash'
                BEGIN
                  SELECT RAISE(ABORT, 'simulated metadata failure');
                END;
                "#,
            )
            .expect("install metadata failure trigger");
        }

        assert!(
            cache
                .replace_from_index_with_embedder(&changed, &embedder)
                .is_err(),
            "injected generation metadata failure must fail the refresh"
        );
        let note = cache
            .read_note_by_slug("a")
            .expect("read cache")
            .expect("prior note remains");
        assert_eq!(
            note.content, "# Old\n\nold body",
            "metadata failure must not publish new note rows without matching metadata"
        );
    }

    #[test]
    fn stamped_metadata_failure_rolls_back_the_entire_refresh_generation() {
        for failed_key in ["embedder_id", "build_duration_secs"] {
            let dir = make_vault(&[("a.md", "# Old\n\nold body")]);
            let cache = SqliteCache::in_memory(384).expect("open");
            let embedder = StubEmbedder::new(384);
            let stamp = embedder.identity();
            let initial = VaultIndex::build(dir.path()).expect("initial index");
            cache
                .replace_from_index_with_options_stamped(
                    &initial,
                    &embedder,
                    &stamp,
                    &BuildOptions::default(),
                )
                .expect("initial stamped populate");
            let prior_duration = cache
                .get_metadata("build_duration_secs")
                .expect("read prior duration");

            std::fs::write(dir.path().join("a.md"), "# New\n\nnew body").expect("change note");
            let changed = VaultIndex::build(dir.path()).expect("changed index");
            {
                let conn = cache.connection().expect("connection");
                conn.execute_batch(&format!(
                    r#"
                    CREATE TRIGGER fail_stamped_metadata
                    BEFORE UPDATE OF value ON metadata
                    WHEN NEW.key = '{failed_key}'
                    BEGIN
                      SELECT RAISE(ABORT, 'simulated stamped metadata failure');
                    END;
                    "#,
                ))
                .expect("install stamped metadata failure trigger");
            }

            assert!(
                cache
                    .replace_from_index_with_options_stamped(
                        &changed,
                        &embedder,
                        &stamp,
                        &BuildOptions::default(),
                    )
                    .is_err(),
                "injected {failed_key} failure must fail the stamped refresh"
            );
            let note = cache
                .read_note_by_slug("a")
                .expect("read cache")
                .expect("prior note remains");
            assert_eq!(
                note.content, "# Old\n\nold body",
                "{failed_key} failure must not publish new rows without the full stamped generation"
            );
            assert_eq!(
                cache
                    .get_metadata("embedder_id")
                    .expect("read embedder id")
                    .as_deref(),
                Some(stamp.as_str()),
                "{failed_key} failure must retain the prior embedder stamp"
            );
            assert_eq!(
                cache
                    .get_metadata("build_duration_secs")
                    .expect("read duration"),
                prior_duration,
                "{failed_key} failure must retain the prior duration stamp"
            );
        }
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
        fn token_count(&self, text: &str, add_special_tokens: bool) -> Result<usize, String> {
            self.inner.token_count(text, add_special_tokens)
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
    fn non_utf8_note_is_skipped_without_failing_the_vault() {
        // A single unreadable file used to abort the whole indexing turn, so a
        // Vault holding one binary `.md` had no search index at all.
        let dir = make_vault(&[("a.md", "# A\n\nbody A"), ("b.md", "# B\n\nbody B")]);
        std::fs::write(dir.path().join("binary.md"), [0xff_u8, 0xfe, 0x00, 0x9c])
            .expect("write binary note");

        let cache = SqliteCache::in_memory(384).expect("open");
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
        let index = VaultIndex::build(dir.path()).expect("build");

        cache
            .replace_from_index_with_embedder(&index, embedder.as_ref())
            .expect("indexing must succeed despite the unreadable note");

        let conn = cache.connection().expect("conn");
        let note_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM notes", [], |r| r.get(0))
            .expect("count");
        assert_eq!(note_count, 2, "the two readable notes still index");
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
            fn token_count(&self, text: &str, add_special_tokens: bool) -> Result<usize, String> {
                self.inner.token_count(text, add_special_tokens)
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
            fn token_count(&self, text: &str, add_special_tokens: bool) -> Result<usize, String> {
                self.inner.token_count(text, add_special_tokens)
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
            "the default must preserve one-input embedding calls"
        );

        let batched_cache = SqliteCache::in_memory(384).expect("open batched cache");
        let batched_opts = BuildOptions {
            embedding_batch_size: 2,
            ..BuildOptions::default()
        };
        batched_cache
            .replace_from_index_with_options(&index, &embedder, &batched_opts)
            .expect("populate batched");
        assert_eq!(
            embedder
                .largest_batch
                .load(std::sync::atomic::Ordering::SeqCst),
            2,
            "a benchmark-selected batch size must reach the embedder"
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

    #[test]
    fn reuse_hash_changes_when_heading_changes_but_body_does_not() {
        // A downstream chunk keeps its body when the heading above it is edited,
        // but its embedded input (which now carries the heading path) changes,
        // so its cached vector must be invalidated rather than reused.
        let before = embedding_reuse_hash("Note", Some("Old heading"), "shared body");
        let after = embedding_reuse_hash("Note", Some("New heading"), "shared body");
        assert_ne!(before, after);
    }

    #[test]
    fn reuse_hash_changes_when_title_changes_but_body_does_not() {
        let before = embedding_reuse_hash("Alpha", None, "shared body");
        let after = embedding_reuse_hash("Bravo", None, "shared body");
        assert_ne!(before, after);
    }

    /// Records every text handed to `embed` so a test can assert the exact
    /// document-side input, delegating the vectors to a deterministic stub.
    struct RecordingEmbedder {
        inner: StubEmbedder,
        inputs: std::sync::Mutex<Vec<String>>,
    }
    impl RecordingEmbedder {
        fn new(dim: usize) -> Self {
            Self {
                inner: StubEmbedder::new(dim),
                inputs: std::sync::Mutex::new(Vec::new()),
            }
        }
        fn recorded(&self) -> Vec<String> {
            self.inputs.lock().expect("recording mutex").clone()
        }
    }
    impl Embedder for RecordingEmbedder {
        fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
            self.inputs
                .lock()
                .expect("recording mutex")
                .extend(texts.iter().cloned());
            self.inner.embed(texts)
        }
        fn embedding_dim(&self) -> usize {
            self.inner.embedding_dim()
        }
        fn token_count(&self, text: &str, add_special_tokens: bool) -> Result<usize, String> {
            self.inner.token_count(text, add_special_tokens)
        }
    }

    #[test]
    fn indexing_embeds_chunk_with_title_and_heading_context() {
        let dir = make_vault(&[(
            "Postgres runbook.md",
            "# Backups\n\nStop the service first.",
        )]);
        let cache = SqliteCache::in_memory(384).expect("open");
        let index = VaultIndex::build(dir.path()).expect("build");
        let embedder = RecordingEmbedder::new(384);
        cache
            .replace_from_index_with_embedder(&index, &embedder)
            .expect("populate");
        let recorded = embedder.recorded();
        assert!(
            recorded
                .iter()
                .any(|t| t.starts_with("Postgres runbook > Backups\n\n")),
            "embedded input should carry title + heading context, got: {recorded:?}"
        );
    }

    #[test]
    fn build_options_chunk_size_controls_chunk_count() {
        // A note that fits one default 800-token chunk splits into several under
        // a small max_tokens, proving chunk options thread through the build.
        let body = "word ".repeat(300);
        let note = format!("# Note\n\n{body}");
        let dir = make_vault(&[("Note.md", note.as_str())]);
        let embedder = StubEmbedder::new(384);
        let index = VaultIndex::build(dir.path()).expect("build");

        let default_cache = SqliteCache::in_memory(384).expect("open");
        default_cache
            .replace_from_index_with_options(&index, &embedder, &BuildOptions::default())
            .expect("default build");
        let default_chunks = note_chunk_count(&default_cache, "note");

        let small_cache = SqliteCache::in_memory(384).expect("open");
        let opts = BuildOptions {
            chunk: ChunkOptions {
                max_tokens: 60,
                overlap_tokens: 5,
            },
            context: true,
            embedding_batch_size: 1,
            embed: true,
        };
        small_cache
            .replace_from_index_with_options(&index, &embedder, &opts)
            .expect("small build");
        let small_chunks = note_chunk_count(&small_cache, "note");

        assert_eq!(default_chunks, 1, "300 tokens fit one default chunk");
        assert!(
            small_chunks > default_chunks,
            "60-token chunks must split the note: {small_chunks} vs {default_chunks}"
        );
    }

    #[test]
    fn build_options_context_off_embeds_raw_body_with_body_only_hash() {
        let dir = make_vault(&[("Runbook.md", "# Backups\n\nrestore steps")]);
        let cache = SqliteCache::in_memory(384).expect("open");
        let embedder = RecordingEmbedder::new(384);
        let index = VaultIndex::build(dir.path()).expect("build");
        let opts = BuildOptions {
            chunk: ChunkOptions::default(),
            context: false,
            embedding_batch_size: 1,
            embed: true,
        };
        cache
            .replace_from_index_with_options(&index, &embedder, &opts)
            .expect("populate");

        let recorded = embedder.recorded();
        assert!(
            recorded.iter().any(|t| t == "# Backups\n\nrestore steps"),
            "context off must embed the raw body, got: {recorded:?}"
        );
        assert!(
            !recorded.iter().any(|t| t.contains("Runbook > Backups")),
            "context off must not prepend the title/heading header: {recorded:?}"
        );

        // The stored reuse hash must be body-only when context is off, so the two
        // representations never collide in a shared cache and reuse stays correct.
        let stored: String = cache
            .connection()
            .expect("conn")
            .query_row(
                "SELECT content_hash FROM chunks WHERE note_slug = ?1",
                ["runbook"],
                |r| r.get(0),
            )
            .expect("hash");
        let body_only = blake3::hash("# Backups\n\nrestore steps".as_bytes())
            .to_hex()
            .to_string();
        assert_eq!(stored, body_only);
    }

    #[test]
    fn editing_a_heading_re_embeds_downstream_chunks_but_reuses_untouched_sections() {
        // Section Alpha is large enough to span multiple chunks: its heading
        // lives only in the first chunk, so later Alpha chunks keep their body
        // verbatim while their heading path points at "Alpha". Section Beta is a
        // separate, untouched section. Renaming the Alpha heading must re-embed
        // ALL Alpha chunks (including the downstream body-unchanged ones, whose
        // heading path changed) while Beta's chunk is reused. Under a body-only
        // reuse key the downstream Alpha chunks would be wrongly reused.
        let big_alpha_body = "alpha ".repeat(1200);
        let note = format!("# Alpha\n\n{big_alpha_body}\n\n# Beta\n\nbeta body");
        let dir = make_vault(&[("Runbook.md", note.as_str())]);
        let cache = SqliteCache::in_memory(384).expect("open");

        let first = RecordingEmbedder::new(384);
        let index = VaultIndex::build(dir.path()).expect("build");
        cache
            .replace_from_index_with_embedder(&index, &first)
            .expect("first populate");
        let first_inputs = first.recorded();
        let alpha_chunks = first_inputs
            .iter()
            .filter(|t| t.starts_with("Runbook > Alpha\n\n"))
            .count();
        assert!(
            alpha_chunks >= 2,
            "test needs Alpha to span multiple chunks, got {alpha_chunks}: {first_inputs:?}"
        );

        // Rename only the Alpha heading; every byte of body text is untouched.
        std::fs::write(
            dir.path().join("Runbook.md"),
            note.replace("# Alpha", "# Alpha Prime"),
        )
        .expect("rewrite");

        let second = RecordingEmbedder::new(384);
        let reindex = VaultIndex::build(dir.path()).expect("rebuild");
        cache
            .replace_from_index_with_embedder(&reindex, &second)
            .expect("second populate");
        let second_inputs = second.recorded();

        let re_embedded_alpha = second_inputs
            .iter()
            .filter(|t| t.starts_with("Runbook > Alpha Prime\n\n"))
            .count();
        assert_eq!(
            re_embedded_alpha, alpha_chunks,
            "every Alpha chunk must re-embed under the renamed heading, including \
             downstream chunks whose body did not change; got {second_inputs:?}"
        );
        assert!(
            !second_inputs.iter().any(|t| t.contains("> Beta\n\n")),
            "the untouched Beta section must be reused, not re-embedded; got {second_inputs:?}"
        );
    }

    #[test]
    fn indexing_stores_header_inclusive_reuse_hash() {
        let dir = make_vault(&[("Runbook.md", "# Backups\n\nrestore steps")]);
        let cache = SqliteCache::in_memory(384).expect("open");
        let index = VaultIndex::build(dir.path()).expect("build");
        let embedder = StubEmbedder::new(384);
        cache
            .replace_from_index_with_embedder(&index, &embedder)
            .expect("populate");
        let (content, heading_path, stored_hash): (String, Option<String>, String) = cache
            .connection()
            .expect("conn")
            .query_row(
                "SELECT content, heading_path, content_hash FROM chunks WHERE note_slug = ?1",
                ["runbook"],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .expect("chunk row");
        let expected = embedding_reuse_hash("Runbook", heading_path.as_deref(), &content);
        assert_eq!(
            stored_hash, expected,
            "stored chunk hash must be over the contextual document, so a heading \
             edit invalidates the cached vector instead of reusing a stale one"
        );
    }
}
