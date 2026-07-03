# SQLite cache atomicity & index integrity

5 confirmed (1 high, 2 medium, 2 low), 0 refuted.

## Confirmed findings

### HIGH: Per-note embedding failure commits an inconsistent cache that is never re-chunked (permanent silent divergence)

- **Trigger conditions:** A transient embedder error, OOM, model timeout, or file-read race for a single note during any reindex (startup, watcher, MCP write, or /api/refresh)
- **Location:** `src/cache/populate.rs:57-68, 86, 188-201`
- **What happens:** In replace_from_index_with_embedder, upsert_note_if_changed (via upsert_note_content) writes the new notes/note_fts/content_hash row FIRST, then chunk_and_embed_note is called to (re)build chunks, chunk_vectors and chunk_fts. If chunk_and_embed_note returns Err, the error is swallowed into per_note_failures with only a warn! (lines 63-67) and the loop continues; the whole transaction still commits at line 86 and the function returns Ok(()). The commit therefore contains a notes row with the NEW content_hash but chunks/chunk_vectors/chunk_fts reflecting the OLD content (or, for a brand-new note, NO chunks/vectors at all). Because change-detection in upsert_note_if_changed (lines 188-201) compares the file only against the notes row (slug + snapshot + content_hash), every subsequent reindex — including after a process restart — sees the note as Unchanged and NEVER calls chunk_and_embed_note again. The divergence is permanent until the note's file content changes again or SCHEMA_VERSION is bumped.
- **Why:** Semantic search (chunk_vectors) and chunk-level FTS silently return stale content or omit the note entirely, while read_note_by_slug/note_fts return the correct new content. There is no detection (no 'notes with zero chunks' check, no retry marker, no error surfaced to the caller), so the cache diverges from the source-of-truth vault indefinitely on a single transient failure. A brand-new note whose first embed fails is invisible to semantic/chunk search forever.
- **Fix sketch:** Treat a per-note chunk/embed failure as fatal to the transaction (propagate the Err so the tx rolls back and is retried), OR roll back only that note's notes-row write so change-detection re-fires next time, OR persist a per-note 'chunks_dirty'/'embed_pending' flag and gate re-chunking on it rather than on the notes content_hash. At minimum, self-heal by re-chunking any note whose chunk count is 0 but content is non-empty.

### MEDIUM: Model embeddings are reused across the persisted file cache with no embedder-identity check, producing a mixed-model vector index after a model swap

- **Trigger conditions:** Upgrading/replacing the embedder model (or its normalization/prefix) while keeping embedding_dim=768 and without bumping SCHEMA_VERSION; the file-backed cache from the old model survives the restart
- **Location:** `src/cache/populate.rs:542-562, 475-518`
- **What happens:** preserve_existing_vectors reuses a stored embedding whenever a chunk's content_hash matches an existing row (keyed on content_hash alone). The persisted file cache (main.rs opens SqliteCache::open, 768) survives restarts, and ensure_schema only wipes when SCHEMA_VERSION differs (schema.rs:16-27). The running server calls the NON-stamped replace_from_index_with_embedder (app_state.rs:201), so the 'embedder_id' metadata is never written or validated in production. If the embedder model is changed but the dimension and schema version are unchanged, unchanged notes keep their OLD-model vectors while re-embedded notes get NEW-model vectors, mixing two incompatible embedding spaces in one vec0 index.
- **Why:** Cosine/L2 distances across a mixed-model vector set are meaningless, silently degrading semantic_search relevance with no error and no detection. Recovery requires an operator to know they must manually bump SCHEMA_VERSION or delete the cache DB.
- **Fix sketch:** Stamp embedder_id (and dim) into metadata on every build in the server path, and in ensure_schema wipe+rebuild when the stored embedder_id/dim does not match the current embedder, exactly as is done for SCHEMA_VERSION.

### MEDIUM: Embedding runs inside the open write transaction: no incremental durability and unbounded WAL growth during large rebuilds

- **Trigger conditions:** First full build of a large vault, or any reindex; a crash/OOM/kill mid-build; a process that repeatedly OOMs while embedding
- **Location:** `src/cache/populate.rs:36-39, 56-69, 86`
- **What happens:** replace_from_index_with_embedder opens a single transaction on the writer connection (line 37, holding the conn Mutex) and performs ALL per-note chunking and embedder.embed() calls (line 500, potentially seconds per batch of CPU ML inference) INSIDE that transaction, committing only once at line 86. Nothing is committed until every note is processed. Under WAL (mod.rs apply_writer_pragmas), the WAL file cannot be checkpointed while the write transaction is open, so it grows for the full duration of the build.
- **Why:** Robustness cost: (1) a crash/OOM/SIGKILL at any point during a multi-minute build rolls the whole transaction back — zero forward progress is durable, so a vault that reliably OOMs partway through embedding can never finish building the cache across restarts; (2) the WAL can bloat toward (or beyond) the full DB size during large builds, risking disk exhaustion on a constrained container. The reused-vector optimization also cannot help across crashes because no chunk rows are ever committed until the end.
- **Fix sketch:** Commit per-note or in batches (chunk+embed each note in its own transaction, or commit every N notes) so progress is durable and the WAL can checkpoint; do the CPU-bound embedding before opening the write transaction and keep the transaction limited to the DB writes.

### LOW: Interrupted first-time schema creation bricks startup, requiring manual cache deletion

- **Trigger conditions:** Process crash/kill during the very first schema initialization, after the metadata table is created but before the final schema_version INSERT commits
- **Location:** `src/cache/schema.rs:39-63, 103-217`
- **What happens:** create_schema issues its DDL via execute_batch with no wrapping transaction, so each CREATE auto-commits independently and the schema_version row is inserted only by the final statement (lines 209-211). If the process is killed after the metadata table is created but before that INSERT runs, then on the next startup existing_schema_version finds metadata_exists=true but the schema_version query returns None and it returns a hard Err ('metadata exists but schema_version is missing. Delete the cache DB...'), which main.rs turns into process::exit(1).
- **Why:** A crash during initial indexing (which, per finding 3, can be a long window because embedding is inside the build) can leave the cache in this half-created state; the container then fails to start on every restart until an operator manually deletes the cache DB, despite the vault being a rebuildable source of truth.
- **Fix sketch:** Wrap create_schema in an explicit transaction so schema creation is all-or-nothing, and/or treat the 'metadata table but no schema_version' state as a wipe-and-rebuild case (like a version mismatch) instead of a fatal error.

### LOW: Note content is read twice per reindex (TOCTOU), so notes and chunk tables can transiently reflect different versions of the same file

- **Trigger conditions:** A concurrent write to the same vault file (watcher-triggered edit, MCP write, or git-sync checkout) landing between the two reads during an in-progress reindex
- **Location:** `src/cache/populate.rs:184, 459`
- **What happens:** upsert_note_if_changed reads the file at line 184 and uses that content for the notes row, note_fts, and content_hash. chunk_and_embed_note then independently re-reads the same file at line 459 to produce chunks/chunk_vectors/chunk_fts. If the file is modified between the two reads, the notes table stores content A (with hash(A)) while the chunk tables store content B, so the note-level and chunk-level views of the same note disagree within a single committed snapshot.
- **Why:** The two derived indexes momentarily disagree, so a chunk/semantic hit can surface text that read_note_by_slug no longer returns. This is self-healing on the next reindex (the file now hashes to B, so the note is re-chunked), so impact is transient, but it is a real single-source read consistency gap.
- **Fix sketch:** Read the file content once in the transaction and thread that single String through both upsert_note_content and chunk_and_embed_note instead of re-reading from disk.

## Refuted (not real / already handled)

(None)
