# Backend Robustness Audit — RECOVERED raw findings

> **STATUS: RAW / UNVERIFIED.** These findings were recovered from the in-memory
> transcripts of two background workflow runs that were killed at the 600 s
> `claude -p` background-wait ceiling before they could checkpoint to disk.
> The **Find** phase completed for 4 of the audit's categories; the adversarial
> **verify panel**, cross-category **dedup**, and **SUMMARY rollup** never ran.
> Treat every item below as a *candidate* to be confirmed, not a confirmed defect.
>
> - Recovered 17 findings from run `wf_28191f9c-44f` (06:30, most complete)
>   and 14 from run `wf_e1f3a69c-200` (01:31, overlapping).
> - Coverage is **partial**: only the git-sync, SQLite-cache, vault-write, and
>   concurrency categories produced finders before the kill. The MCP-surface,
>   HTTP/auth, and error-shape categories were **never run**.
> - Recovered on 2026-07-03 from workflow journals under
>   `~/.claude/projects/.../subagents/workflows/`.

## Primary set — run `wf_28191f9c-44f` (06:30)

Severity mix: **4 high · 9 medium · 4 low** (17 total).

### 1. [HIGH] Git sync holds the shared vault_write_lock across un-timed-out network fetch/push, so a slow or hanging remote blocks every HTTP and MCP vault write

- **File:** `src/git/task.rs` : 149-166
- **Triggering condition:** slow/hanging/unreachable git remote (dropped TLS, firewall blackhole, dead server); any concurrent HTTP or MCP write while a sync is in flight; a remote that keeps the TCP connection open without responding

**What happens:** run_one_sync acquires the shared vault-mutation lock (`let _guard = vault_lock.lock().await;`, line 149 — the very same Arc<Mutex<()>> that AppState.vault_write_lock points to, wired in main.rs:168) and holds it across `spawn_blocking(... runner(...) ...).await` (154-165). The runner is `git::sync`, which performs `remote.fetch(...)` (src/git/sync.rs:256) and `remote.push(...)` (src/git/sync.rs:363) via git2. Neither FetchOptions nor PushOptions sets any timeout, so these calls block for as long as the remote keeps the socket alive (potentially the OS TCP timeout, minutes). Every write handler blocks on `state.vault_write_lock.clone().lock_owned().await` (src/handlers/write_api.rs:181/212/233/276/312/364/400/435 and src/mcp/tools.rs:62) for that entire window. A syncs runs after each debounce, so a persistently slow/hanging remote makes all note creation/edit/move/delete and attachment writes hang, even though the local vault and reads are perfectly healthy.

**Why it's real:** The adverse condition 'slow/failing/rejecting git remote' is explicitly in scope. Coupling the vault write-availability of the whole server to the responsiveness of an external git remote is an availability failure a public launch must not have; a misbehaving remote should degrade sync only, not freeze writes.

**Fix sketch:** Do the network phase (fetch/push) outside the vault_write_lock — only hold the lock around the local working-tree-mutating steps (stage/commit/merge/checkout/reset). Additionally set a connect/transfer timeout on the git2 fetch/push (e.g. via RemoteCallbacks transfer-progress deadline or a bounded spawn_blocking with cancellation) so a hung remote can never pin the sync task indefinitely.

### 2. [HIGH] Per-note embedding failure commits an inconsistent cache that is never re-chunked (permanent silent divergence)

- **File:** `src/cache/populate.rs` : 57-68, 86, 188-201
- **Triggering condition:** A transient embedder error, OOM, model timeout, or file-read race for a single note during any reindex (startup, watcher, MCP write, or /api/refresh)

**What happens:** In replace_from_index_with_embedder, upsert_note_if_changed (via upsert_note_content) writes the new notes/note_fts/content_hash row FIRST, then chunk_and_embed_note is called to (re)build chunks, chunk_vectors and chunk_fts. If chunk_and_embed_note returns Err, the error is swallowed into per_note_failures with only a warn! (lines 63-67) and the loop continues; the whole transaction still commits at line 86 and the function returns Ok(()). The commit therefore contains a notes row with the NEW content_hash but chunks/chunk_vectors/chunk_fts reflecting the OLD content (or, for a brand-new note, NO chunks/vectors at all). Because change-detection in upsert_note_if_changed (lines 188-201) compares the file only against the notes row (slug + snapshot + content_hash), every subsequent reindex — including after a process restart — sees the note as Unchanged and NEVER calls chunk_and_embed_note again. The divergence is permanent until the note's file content changes again or SCHEMA_VERSION is bumped.

**Why it's real:** Semantic search (chunk_vectors) and chunk-level FTS silently return stale content or omit the note entirely, while read_note_by_slug/note_fts return the correct new content. There is no detection (no 'notes with zero chunks' check, no retry marker, no error surfaced to the caller), so the cache diverges from the source-of-truth vault indefinitely on a single transient failure. A brand-new note whose first embed fails is invisible to semantic/chunk search forever.

**Fix sketch:** Treat a per-note chunk/embed failure as fatal to the transaction (propagate the Err so the tx rolls back and is retried), OR roll back only that note's notes-row write so change-detection re-fires next time, OR persist a per-note 'chunks_dirty'/'embed_pending' flag and gate re-chunking on it rather than on the notes content_hash. At minimum, self-heal by re-chunking any note whose chunk count is 0 but content is non-empty.

### 3. [HIGH] delete_note moves assets to trash BEFORE renaming the note, with no rollback

- **File:** `src/vault/write/notes.rs` : 467-475
- **Triggering condition:** fs::rename of the note fails after assets already moved (e.g. note file concurrently gone, cross-device trash path, EIO); process crash between move_assets (l.467) and fs::rename (l.468)

**What happens:** In delete_note the order is: move_assets(&asset_moves) (l.467) -> fs::rename(entry.path -> trash) (l.468) -> apply_rewrites (l.475). The referenced attachments are physically relocated into .hatchdoor-trash BEFORE the note itself is trashed. If the note rename returns Err, the function bails with WriteError::Io but the already-moved assets are NOT moved back. This leaves the original note in place now pointing at attachments that no longer exist at their old location. Note the sibling move_or_rename_note does the safe order (rename note first at l.376, then move_assets at l.383), and move_attachment/delete_attachment install rollback_rewrites on rename failure — delete_note has neither the safe order nor any rollback.

**Why it's real:** Data-integrity: a partial failure (no crash even required) leaves a live note with dangling image/pdf references while the bytes sit in trash; the caller sees only an error and cannot tell the vault was half-mutated.

**Fix sketch:** Rename the note into trash FIRST, then move_assets, mirroring move_or_rename_note; on any post-rename failure, roll the note (and any already-moved assets) back, or move assets last and reverse them on error.

### 4. [HIGH] Crash mid-merge leaves the repo in MERGE state; every later sync fails at write_tree with no auto-recovery

- **File:** `src/git/sync.rs` : 291
- **Triggering condition:** remote diverged so a merge runs (integrate_remote -> merge_remote); process killed between repo.merge() and repo.cleanup_state()/reset (sync.rs:291-330); on restart startup_flush runs a sync while .git is still in Merge state

**What happens:** merge_remote() calls repo.merge() (line 291), which persists MERGE_HEAD and puts the repo in RepositoryState::Merge, and only clears it via reset+cleanup_state (conflict path, 308-309) or cleanup_state (clean path, 330). If the process is killed anywhere between the merge and that cleanup, the on-disk index is left half-merged/conflicted. Neither validate_repo() nor sync()/stage_and_commit() ever inspect repo.state() or call cleanup_state()/merge-abort at startup. On the next sync, stage_and_commit() loads that index (repo.index(), line 215) and calls index.write_tree() (line 231), which errors on a not-fully-merged index. Every subsequent sync then fails with GitError::Other and the subsystem is wedged until a human runs `git merge --abort` on the container.

**Why it's real:** Data-integrity/durability on a target that can be 'killed at any instant': a well-defined crash window silently and permanently stops all pushing of vault edits to the remote, recoverable only via manual shell access. The startup recovery path (startup_flush) actively re-triggers the failure instead of healing it.

**Fix sketch:** At startup (validate_repo or before startup_flush) check repo.state(); if it is Merge/RevertHead/etc., call repo.cleanup_state() and hard-reset the working index back to HEAD before syncing. Also guard sync() by aborting a stale merge state before staging.

### 5. [MEDIUM] A crash between a vault file write and its debounced git commit strands the edit out of git, and the stranded dirty file later makes every merge-requiring sync fail

- **File:** `src/git/task.rs` : 23
- **Triggering condition:** process crash/kill between atomic_write and the next debounced git sync (default several-second debounce window); then the remote moving ahead so a later sync needs a merge

**What happens:** Write handlers write the vault file to disk (src/vault/write/fs_ops.rs:31 atomic_write) and enqueue a WriteRecord onto an in-memory unbounded channel (GitSyncHandle::record, task.rs:23) that is only drained after a debounce. If the process is killed in that window the WriteRecord is lost, but the file is already on disk. On restart, startup_flush only runs when `has_unpushed` is true (task.rs:51; src/git/sync.rs:140-154), which compares local vs remote *commits* — it cannot see a working-tree edit that was never committed. And stage_and_commit only stages paths present in a batch (src/git/sync.rs:216-228), so no future sync ever commits that file on its own. Worse: the stranded file is a modified tracked file, so the next sync that must integrate remote changes hits dirty_tracked_files (src/git/sync.rs:338-355), which reports it and returns GitError::DirtyWorkingTree (src/git/sync.rs:285-288), aborting the merge. One crash-stranded edit therefore blocks all subsequent merge-requiring syncs until a human commits/cleans the working tree.

**Why it's real:** 'Process crash mid-operation' and 'a slow/failing/rejecting git remote' are named adverse conditions. The coordination between the volatile write-record queue and the durable vault leaves changes silently un-synced and can wedge sync entirely, which for a public launch means silent divergence between the vault and its git backup.

**Fix sketch:** On startup (and periodically) reconcile actual working-tree status against git, not just unpushed commits: if `git status` shows dirty tracked files under the vault, stage+commit them (or surface them) rather than only flushing on has_unpushed. Alternatively persist pending WriteRecords, or have sync stage all dirty tracked vault paths instead of only the current batch.

### 6. [MEDIUM] A panic while the SqliteCache writer Mutex is held poisons it permanently, killing all future reindexes and cache writes for the process lifetime

- **File:** `src/cache/mod.rs` : 164
- **Triggering condition:** any panic inside the reindex transaction while holding the writer lock — e.g. bytemuck::cast_slice on a corrupted/partially-written chunk_vectors blob, an embedder panic, or any rusqlite unwrap during replace_from_index_with_embedder

**What happens:** SqliteCache.conn is a std::sync::Mutex (mod.rs:32). replace_from_index_with_embedder holds that guard (`self.connection()?`, populate.rs:36) for the entire reindex transaction — including chunking, embedding, and vector reads. preserve_existing_vectors does `bytemuck::cast_slice(&bytes)` (populate.rs:557), which panics if a stored embedding blob length is not a multiple of 4 (possible after a crash-truncated WAL/blob). Any panic there unwinds through the held MutexGuard and poisons the Mutex. From then on connection() returns Err('SQLite cache connection lock poisoned', mod.rs:164-168) forever: every refresh_now/refresh_if_needed reindex fails, and set_metadata/get_metadata fail. Write handlers still write the vault file (file-backed reads use the separate pool and keep working) but refresh_after_write then returns 500 (src/handlers/write_api.rs:526-529 / src/mcp/tools.rs:719-723) — so writes appear to fail and the cache is frozen at its last snapshot until the process is restarted. The poison is handled gracefully (no crash) but is unrecoverable in-process.

**Why it's real:** A single panic converts a transient fault into a permanent, process-wide degradation of the core index-refresh path, with no self-recovery — exactly the kind of concurrency/shared-state fragility that matters when the process must survive malformed/partially-written vault or cache data.

**Fix sketch:** Recover from a poisoned lock instead of propagating the error indefinitely (e.g. `self.conn.lock().unwrap_or_else(|e| e.into_inner())` since the connection object itself is still usable), or reopen a fresh writer connection on poison. Also guard preserve_existing_vectors against non-multiple-of-4 blob lengths (use try_cast_slice / length check) so corrupted vector rows return an error rather than panicking under the lock.

### 7. [MEDIUM] Model embeddings are reused across the persisted file cache with no embedder-identity check, producing a mixed-model vector index after a model swap

- **File:** `src/cache/populate.rs` : 542-562, 475-518
- **Triggering condition:** Upgrading/replacing the embedder model (or its normalization/prefix) while keeping embedding_dim=768 and without bumping SCHEMA_VERSION; the file-backed cache from the old model survives the restart

**What happens:** preserve_existing_vectors reuses a stored embedding whenever a chunk's content_hash matches an existing row (keyed on content_hash alone). The persisted file cache (main.rs opens SqliteCache::open, 768) survives restarts, and ensure_schema only wipes when SCHEMA_VERSION differs (schema.rs:16-27). The running server calls the NON-stamped replace_from_index_with_embedder (app_state.rs:201), so the 'embedder_id' metadata is never written or validated in production. If the embedder model is changed but the dimension and schema version are unchanged, unchanged notes keep their OLD-model vectors while re-embedded notes get NEW-model vectors, mixing two incompatible embedding spaces in one vec0 index.

**Why it's real:** Cosine/L2 distances across a mixed-model vector set are meaningless, silently degrading semantic_search relevance with no error and no detection. Recovery requires an operator to know they must manually bump SCHEMA_VERSION or delete the cache DB.

**Fix sketch:** Stamp embedder_id (and dim) into metadata on every build in the server path, and in ensure_schema wipe+rebuild when the stored embedder_id/dim does not match the current embedder, exactly as is done for SCHEMA_VERSION.

### 8. [MEDIUM] Embedding runs inside the open write transaction: no incremental durability and unbounded WAL growth during large rebuilds

- **File:** `src/cache/populate.rs` : 36-39, 56-69, 86
- **Triggering condition:** First full build of a large vault, or any reindex; a crash/OOM/kill mid-build; a process that repeatedly OOMs while embedding

**What happens:** replace_from_index_with_embedder opens a single transaction on the writer connection (line 37, holding the conn Mutex) and performs ALL per-note chunking and embedder.embed() calls (line 500, potentially seconds per batch of CPU ML inference) INSIDE that transaction, committing only once at line 86. Nothing is committed until every note is processed. Under WAL (mod.rs apply_writer_pragmas), the WAL file cannot be checkpointed while the write transaction is open, so it grows for the full duration of the build.

**Why it's real:** Robustness cost: (1) a crash/OOM/SIGKILL at any point during a multi-minute build rolls the whole transaction back — zero forward progress is durable, so a vault that reliably OOMs partway through embedding can never finish building the cache across restarts; (2) the WAL can bloat toward (or beyond) the full DB size during large builds, risking disk exhaustion on a constrained container. The reused-vector optimization also cannot help across crashes because no chunk rows are ever committed until the end.

**Fix sketch:** Commit per-note or in batches (chunk+embed each note in its own transaction, or commit every N notes) so progress is durable and the WAL can checkpoint; do the CPU-bound embedding before opening the write transaction and keep the transaction limited to the DB writes.

### 9. [MEDIUM] Multi-file backlink/asset rewrites are applied non-atomically with no rollback across move/rename/delete/archive

- **File:** `src/vault/write/rewrites.rs` : 168-177
- **Triggering condition:** apply_rewrites hits an IO error (disk full / EIO) on the k-th of N notes; process crash partway through the rewrite loop; the outer op fails after the note was already renamed (rewrites applied after fs::rename in move_or_rename_note l.384 and delete_note l.475)

**What happens:** apply_rewrites iterates rewrites and calls atomic_write per file, appending to `written` as it goes. Each individual file write is atomic, but the SET is not: if write #k fails, files 1..k-1 are already committed with updated wikilinks/asset paths while k..N still hold the old references, and the function returns Err with no attempt to restore the earlier files. move_or_rename_note and delete_note additionally sequence this AFTER the note's own fs::rename (notes.rs l.384 / l.475), so a failure or crash there yields a moved/trashed note plus a mix of rewritten and stale backlinks — dangling or duplicated links that the returned error hides. Unlike move_attachment (which calls rollback_rewrites), these note paths have no compensating action.

**Why it's real:** Crash/IO between the file mutation and its link follow-up (explicitly in scope) leaves the vault link graph internally inconsistent, and the cache refresh is skipped because the caller received an error.

**Fix sketch:** Make apply_rewrites capture prior contents and reverse committed writes on failure (best-effort transaction), or stage all rewrites to temp files and rename them only after every stage succeeds; order the note rename to happen last so a rewrite failure needs no note rollback.

### 10. [MEDIUM] atomic_write does not fsync the temp file or the parent directory

- **File:** `src/vault/write/fs_ops.rs` : 31-46
- **Triggering condition:** process/container killed immediately after fs::rename returns but before the OS flushes data/metadata; host power loss

**What happens:** atomic_write writes content to `<path>.hatchdoor-tmp` with fs::write and immediately fs::rename()s it over the target, never calling File::sync_all on the temp file nor fsync on the containing directory. rename() only guarantees atomic metadata replacement, not that the freshly-written bytes or the rename itself are durable. On ext4/xfs with a crash in the window before the journal commit, the target can reappear as zero-length or with stale contents (the classic write-then-rename-without-fsync data-loss pattern). Given the stated assumption that the process can be killed at any instant, every note write (create/update/edit/append/replace_section and all backlink rewrites) is exposed.

**Why it's real:** A note that reports success can be silently truncated to empty or reverted after a crash, i.e. real data loss on the source of truth.

**Fix sketch:** Open the temp file, write, then sync_all() before rename; after rename, fsync the parent directory. Wrap in a small helper so all callers get durability.

### 11. [MEDIUM] No timed retry: a transient remote/network/auth failure strands committed edits unpushed until the next write or a full process restart

- **File:** `src/git/task.rs` : 91
- **Triggering condition:** git push/fetch fails transiently (remote unreachable, auth blip, non-fast-forward) -> GitError::Remote; no further MCP/HTTP writes occur afterward

**What happens:** run_loop() only ever calls run_one_sync() from two places: startup_flush (line 87) and after a debounce window that begins with receiver.recv().await (line 93). The outer loop blocks on recv() with no periodic timer. When sync() commits locally but push()/fetch fails (sync.rs:198-199 -> GitError::Remote), the commit is retained (good) but the failed batch is dropped via std::mem::take (task.rs:121) and there is no scheduled re-attempt. The comment 'Retried on the next batch' (sync.rs:36) only holds if another write arrives; if writes stop, the local commit sits unpushed indefinitely until the process is restarted and startup_flush notices has_unpushed.

**Why it's real:** For a launch, a brief network/remote outage can leave hours/days of vault commits unreplicated to the remote with no self-healing, defeating the backup guarantee. It is observable (status.last_ok=false) but not self-correcting.

**Fix sketch:** Add a retry timer in run_loop: when the last sync failed with a retryable kind (remote/other) or unpushed_count>0, arm a backoff sleep in the select! so run_one_sync(empty batch) is re-attempted without needing a new write.

### 12. [MEDIUM] Uncommitted working-tree changes stranded by a failed commit-stage (or crash between index.write and commit) are never re-staged

- **File:** `src/git/sync.rs` : 216
- **Triggering condition:** stage_and_commit errors after the file is on disk (e.g. index.write/write_tree/commit fails, or process killed between index.write (line 229) and repo.commit (line 241)); the same note is not written again by a later batch

**What happens:** sync() only stages the explicit paths of the current batch (stage_and_commit loop, lines 216-228). Recovery of stranded work relies solely on has_unpushed()/unpushed_count(), which count committed-but-unpushed commits via graph_ahead_behind (sync.rs:140-154, 159-175); they do NOT detect uncommitted working-tree or staged-but-uncommitted changes. If a batch's commit fails after the vault file was already written to disk, that path is dropped with the batch (task.rs:121) and is never part of any future batch, so its change is never committed/pushed unless the same file happens to be edited again. startup_flush passes an empty batch (task.rs:88) and stages nothing, so a restart does not recover it either.

**Why it's real:** The vault file on disk is safe (source of truth), but the git backup silently and permanently omits that edit with no signal beyond a stale last_error. Worse, a stranded modification to a *tracked* file becomes WT_MODIFIED and then wedges all future merges (see dirty-tree finding).

**Fix sketch:** On sync entry or startup, additionally stage any dirty tracked files (or run an add_all over the vault) so uncommitted vault state is always captured, not just the paths of the in-memory batch.

### 13. [MEDIUM] A single uncommitted tracked-file edit permanently blocks all pushes whenever the remote diverges (DirtyWorkingTree), with no auto-recovery

- **File:** `src/git/sync.rs` : 285
- **Triggering condition:** someone edits a tracked .md directly on the server without committing (or an edit was stranded uncommitted per the previous finding); remote later moves ahead so a merge is required (behind>0)

**What happens:** merge_remote() refuses to integrate when dirty_tracked_files() is non-empty (lines 285-288), returning GitError::DirtyWorkingTree so the force checkout cannot discard the edit. This is correct for avoiding data loss, but there is no automatic remediation: as long as one uncommitted tracked-file modification exists AND the remote is ahead, every batch fails at integrate_remote and nothing is pushed. Local commits accumulate unpushed (unpushed_count grows) and the only escape is a human committing/reverting the file on the container. run_one_sync merely records the error (task.rs:190) and moves on.

**Why it's real:** One stray editor save (or a stranded edit) can silently halt the entire backup pipeline indefinitely. For an unattended public deployment this converts a benign situation into a persistent, human-only-recoverable outage of git sync.

**Fix sketch:** Auto-commit dirty tracked files into their own commit before merging (they are already the source of truth on disk), instead of refusing forever; or surface an actionable recovery path. At minimum, stage+commit the dirty edit so the merge can proceed without discarding it.

### 14. [LOW] Interrupted first-time schema creation bricks startup, requiring manual cache deletion

- **File:** `src/cache/schema.rs` : 39-63, 103-217
- **Triggering condition:** Process crash/kill during the very first schema initialization, after the metadata table is created but before the final schema_version INSERT commits

**What happens:** create_schema issues its DDL via execute_batch with no wrapping transaction, so each CREATE auto-commits independently and the schema_version row is inserted only by the final statement (lines 209-211). If the process is killed after the metadata table is created but before that INSERT runs, then on the next startup existing_schema_version finds metadata_exists=true but the schema_version query returns None and it returns a hard Err ('metadata exists but schema_version is missing. Delete the cache DB...'), which main.rs turns into process::exit(1).

**Why it's real:** A crash during initial indexing (which, per finding 3, can be a long window because embedding is inside the build) can leave the cache in this half-created state; the container then fails to start on every restart until an operator manually deletes the cache DB, despite the vault being a rebuildable source of truth.

**Fix sketch:** Wrap create_schema in an explicit transaction so schema creation is all-or-nothing, and/or treat the 'metadata table but no schema_version' state as a wipe-and-rebuild case (like a version mismatch) instead of a fatal error.

### 15. [LOW] Note content is read twice per reindex (TOCTOU), so notes and chunk tables can transiently reflect different versions of the same file

- **File:** `src/cache/populate.rs` : 184, 459
- **Triggering condition:** A concurrent write to the same vault file (watcher-triggered edit, MCP write, or git-sync checkout) landing between the two reads during an in-progress reindex

**What happens:** upsert_note_if_changed reads the file at line 184 and uses that content for the notes row, note_fts, and content_hash. chunk_and_embed_note then independently re-reads the same file at line 459 to produce chunks/chunk_vectors/chunk_fts. If the file is modified between the two reads, the notes table stores content A (with hash(A)) while the chunk tables store content B, so the note-level and chunk-level views of the same note disagree within a single committed snapshot.

**Why it's real:** The two derived indexes momentarily disagree, so a chunk/semantic hit can surface text that read_note_by_slug no longer returns. This is self-healing on the next reindex (the file now hashes to B, so the note is re-chunked), so impact is transient, but it is a real single-source read consistency gap.

**Fix sketch:** Read the file content once in the transaction and thread that single String through both upsert_note_content and chunk_and_embed_note instead of re-reading from disk.

### 16. [LOW] asset_move_plan: allow_trash_collision branch is dead code, so deleting a note whose asset name already exists in trash fails

- **File:** `src/vault/write/assets.rs` : 47-58
- **Triggering condition:** deleting/trashing a second note that references an attachment whose relative path already exists under .hatchdoor-trash

**What happens:** asset_move_plan first does `if destination_asset.exists() { return Err(Conflict...) }` (l.47-52) unconditionally, and only then `if allow_trash_collision && destination_asset.exists() { return Err(...) }` (l.53-58) — which is unreachable dead code because the earlier check already returned. For delete_note the destination dir is `.hatchdoor-trash/...` and asset names are NOT uniquified (only the note filename is, via unique_trash_relative_path). So trashing a note whose referenced asset (e.g. img.png) collides with a previously-trashed img.png returns WriteError::Conflict and blocks the delete entirely, even though allow_trash_collision=true was clearly intended to tolerate/uniquify that case.

**Why it's real:** A benign, common operation (deleting notes that share attachment filenames) becomes un-performable, and the dead second branch shows the collision handling was intended but never takes effect.

**Fix sketch:** Gate the collision error on `!allow_trash_collision`, and for the trash case uniquify the asset destination (like unique_trash_attachment_relative_path) instead of erroring; remove the unreachable duplicate check.

### 17. [LOW] Spurious DirtyWorkingTree failure when a next-batch write races the sync window

- **File:** `src/git/task.rs` : 149
- **Triggering condition:** remote is ahead so a merge is required; an MCP write to an existing (tracked) note completes in the gap between the debounce timer firing (task.rs:104) and run_one_sync acquiring vault_write_lock (task.rs:149)

**What happens:** MCP writes write the file to disk then record() under vault_write_lock (mcp/tools.rs:62), releasing the lock before the git task grabs it. A write that lands after the current batch's debounce timer fires but before run_one_sync() acquires the lock leaves a WT_MODIFIED tracked file on disk that belongs to the *next* batch. When the current batch then needs a merge, dirty_tracked_files() (sync.rs:338) sees that not-yet-committed edit and merge_remote returns GitError::DirtyWorkingTree, failing the current batch's push even though nothing is actually wrong. No data is lost (the edit commits in the next batch), but the current push is spuriously blocked and status shows a dirty_tree error.

**Why it's real:** Under concurrent load with a diverging remote this produces confusing intermittent sync failures; combined with the no-timed-retry gap it can delay pushes noticeably.

**Fix sketch:** Only treat files as 'dirty manual edits' if they are not part of the pending queue, or drain newly-arrived records into the current batch (and stage them) before deciding the tree is dirty.


---

## Secondary set — run `wf_e1f3a69c-200` (01:31)

Earlier run over the same 4 categories. Heavily overlaps the primary set above,
but retained in full because it contains a few findings the later run did not
surface (e.g. FNV-1a-64 hash-collision change-detection, orphaned `.hatchdoor-tmp`
files, `import_attachment_bytes` non-atomic overwrite). Cross-check against the
primary set before acting.

### 1. [HIGH] Hung/slow git remote holds the global vault write lock across a timeout-less network fetch+push, freezing ALL vault writes

- **File:** `src/git/task.rs` : 149
- **Triggering condition:** Git remote unreachable or TCP-blackholed (no RST) so fetch/push blocks for the OS TCP timeout (minutes); Any concurrent HTTP write handler or MCP write tool that must acquire vault_write_lock; Git sync enabled (git_sync = Some)

**What happens:** run_one_sync acquires the shared vault_write_lock (`let _guard = vault_lock.lock().await`, line 149) and holds it across the entire spawn_blocking hop (dropped at line 166). That blocking closure calls the runner => sync() (src/git/sync.rs:183), which performs integrate_remote's network fetch (sync.rs:255-257) and push (sync.rs:357-364). Neither FetchOptions nor PushOptions sets any timeout (confirmed: src/git/sync.rs:253,359 configure only remote_callbacks), and libgit2's smart-transport has no default connect/IO timeout, so a remote that accepts the TCP connection but never responds blocks indefinitely. The exact same Arc<Mutex<()>> is the vault_write_lock every HTTP write handler (src/handlers/write_api.rs:181,212,233,276,312,364,400,435) and every MCP write tool (src/mcp/tools.rs:62) must acquire before touching the vault. Result: one stuck sync makes every create/update/move/delete/attachment write hang for the full TCP timeout, converting a degraded git remote into a full write outage.

**Why it's real:** The launch target explicitly includes 'a slow/failing/rejecting git remote'. Because the network I/O is done under the process-wide write mutex with no timeout, remote unavailability escalates from a background sync failure (which the design otherwise tolerates and retries) into a stall of all interactive vault mutations.

**Fix sketch:** Set a bounded transfer/connect timeout on the git operations (e.g. via FetchOptions/PushOptions callbacks or a wrapping timeout on the spawn_blocking join), and/or perform the network fetch+push WITHOUT holding vault_write_lock — hold the lock only for the local staging/commit/merge/checkout that actually touches the working tree, releasing it before the push.

### 2. [HIGH] Per-note embedding failure is swallowed and permanently poisons the chunk/vector index

- **File:** `src/cache/populate.rs` : 56-88
- **Triggering condition:** embedder.embed() returns Err for one note during a reindex (transient OOM / tokenizer / model error); the note's content actually changed on disk; next reindex sees unchanged content_hash and skips the note forever

**What happens:** In replace_from_index_with_embedder, upsert_note_if_changed writes the note's NEW content and NEW content_hash into notes/note_fts (upsert_note_content, lines 300-367) and returns Wrote{slug}. chunk_and_embed_note is then called (line 58). If embedder.embed() fails (line 500), the error is caught at lines 63-67, counted as per_note_failures, logged at warn, and the loop continues. chunk_and_embed_note returns Err BEFORE reaching replace_chunks_for_note, so the note's OLD chunks and OLD chunk_vectors are left untouched. The transaction still commits at line 86, so the note row now has the new content while the chunk/vector rows describe the old content. Because the new content_hash is committed, the next reindex classifies the note as Unchanged (cached_matches_file_and_content, lines 188-193) and never retries embedding. The divergence is permanent until the file content changes again, and refresh_now/refresh_if_needed return Ok and broadcast a new vault revision as if the refresh fully succeeded.

**Why it's real:** The chunk/vector layer feeds the primary MCP search_notes tool (src/search/retrieve.rs semantic_search + fts_search_chunks). A single transient embed failure silently and permanently makes semantic and chunk-keyword search return stale content for that note, with no error surfaced and no self-healing path. This is exactly the 'cache diverges from the on-disk vault without detection' failure the audit targets.

**Fix sketch:** Treat a per-note embed failure as a reason to NOT advance that note's persisted content_hash (or record a 'needs_embed' dirty flag) so a later reindex retries; alternatively roll back / re-queue the note. At minimum, propagate the failure so callers know the refresh was partial rather than returning Ok.

### 3. [HIGH] Uncommitted vault edits orphaned by a crash are never synced and can wedge all future syncs

- **File:** `src/git/sync.rs` : 216
- **Triggering condition:** Process killed during the debounce window (default 30s) after a file is written to disk but before the batch is committed; Restart with a modified-but-uncommitted tracked file left on disk; Remote later moves ahead so a merge is required, triggering the dirty-tree refusal

**What happens:** WriteRecords live only in memory (the mpsc channel plus the in-memory batch Vec in run_loop). A vault write persists the Markdown file immediately, but the commit only happens later in stage_and_commit, which stages ONLY the paths carried by the batch (sync.rs:216-228, index.add_path per batch path). If the process is killed during the debounce window the batch is lost. On restart, spawn_sync_task decides whether to flush purely via has_unpushed (task.rs:51), which inspects the COMMIT graph via graph_ahead_behind (sync.rs:140-154); it does not see uncommitted working-tree changes. So the orphaned edit is never staged unless a later write happens to re-touch that exact path. Worse, once the remote moves ahead and a merge is needed, merge_remote calls dirty_tracked_files (sync.rs:285-288) and returns GitError::DirtyWorkingTree, refusing to proceed, so the single orphaned file blocks EVERY subsequent sync until a human intervenes.

**Why it's real:** This is precisely the 'retention of unsynced changes across restart' failure mode. Vault content is safe on disk, but git silently drifts from the vault and the edit never reaches the remote; in the merge case the whole sync subsystem is wedged. get_git_sync_status will not even show it as pending, since pending resets to 0 on restart. There is no reconciliation of working-tree state against HEAD at startup.

**Fix sketch:** At startup (and before syncs) reconcile the working tree: run a status scan and stage/commit any dirty tracked files (or surface them in status.unpushed/pending) so a crash within the debounce window is recovered, rather than relying solely on has_unpushed which only sees already-committed commits.

### 4. [HIGH] Attachment move/rename/delete rollback is a no-op when the rename fails, leaving many notes silently pointing at a nonexistent path

- **File:** `src/vault/write/attachments.rs` : 150-161, 197-209, 238-249
- **Triggering condition:** fs::rename of the attachment fails (ENOSPC, EACCES/read-only mount, destination parent removed concurrently, or a cross-mount vault); process killed between apply_rewrites and fs::rename

**What happens:** move_attachment, delete_attachment and move_attachment_by_paths first apply_rewrites() to every referencing note (line 152/200/240) rewriting their links to the NEW/trash path, and only afterwards fs::rename() the actual file. On rename failure they call rollback_rewrites(vault, index, from=target/trash, to=source). rollback_rewrites -> asset_reference_rewrite_plan matches a note only when same_existing_path(resolved, from_path) is true (rewrites.rs:179-190, assets.rs:108, paths.rs:258-263), and same_existing_path canonicalizes BOTH paths. Because the rename failed, the target/trash file does not exist, fs::canonicalize(from_path) errors, same_existing_path returns false for every note, so the recovery plan is empty and apply_rewrites does nothing. The forward rewrites are never undone: the attachment stays at its original location while all referencing notes now link to a path that holds no file. The same corruption happens with no error at all if the process is killed in the window between apply_rewrites and the rename.

**Why it's real:** Every note that referenced the attachment is left with a broken image/embed link and the actual file is orphaned, with no error surfaced to the caller (rename-failure path even claims to roll back). This is silent, multi-note data corruption that survives cache rebuild since the vault files themselves were mutated.

**Fix sketch:** Rename the file FIRST, then apply link rewrites; on rewrite failure roll back by renaming the file back and restoring original note contents. Alternatively snapshot each rewritten note's prior bytes and restore those exact bytes on failure instead of recomputing a reverse plan that depends on the (missing) destination existing.

### 5. [MEDIUM] SQLite writer connection is a poison-prone std::sync::Mutex; one panic while indexing permanently disables cache refresh and silently serves stale data

- **File:** `src/cache/mod.rs` : 32
- **Triggering condition:** A panic while the writer MutexGuard is held during replace_from_index_with_embedder; Concretely: a corrupt/short chunk-vector blob feeding bytemuck::cast_slice (populate.rs:557), a .expect("tx") failing under memory pressure (chunk_ops.rs:138,176,193,237,259), or an allocation failure during embedding; Process kept running after the panic (spawn_blocking converts the panic to a JoinError, so the server stays up)

**What happens:** The single writer connection is `conn: Mutex<Connection>` (std::sync::Mutex, line 32). replace_from_index_with_embedder holds this guard (`let mut conn = self.connection()?`, populate.rs:36) across the whole rebuild — including embedding and bytemuck::cast_slice(&bytes) at populate.rs:557, which panics if a stored vector blob's length is not a multiple of 4 (e.g. a WAL truncated by a crash), and the several `conn.transaction().expect("tx")` sites in chunk_ops.rs. If any of these unwind, the guard's Drop poisons the mutex. From then on connection() returns Err (`SQLite cache connection lock poisoned`, mod.rs:167), so every subsequent refresh_now/run_reindex fails permanently while pooled READ connections (separate handles) keep answering — meaning the cache silently stops updating and serves stale search/read results indefinitely with the server still reporting healthy.

**Why it's real:** tokio RwLock (used for AppState.cache and git status) does not poison, but this std Mutex does. Because reads survive on the pool, the failure is silent: no crash, no restart, just a cache frozen at the pre-panic snapshot until an operator notices results are stale and restarts the process.

**Fix sketch:** Recover from poison instead of propagating it (e.g. `.unwrap_or_else(|e| e.into_inner())` in connection()/return_read_connection, or use parking_lot::Mutex which does not poison), and replace the bytemuck::cast_slice with a checked conversion that returns Err on a non-multiple-of-4 blob rather than panicking.

### 6. [MEDIUM] Embedder identity/dimension is never validated against the persisted cache (stamping is dead code in production)

- **File:** `src/cache/populate.rs` : 542-562
- **Triggering condition:** operator upgrades/swaps the embedding model across a restart; same embedding dimension but different model → reused vectors from old model; different dimension without bumping SCHEMA_VERSION → vec0 table keeps old dim

**What happens:** preserve_existing_vectors reuses stored embeddings for any chunk whose blake3 content_hash is unchanged (lines 542-562, consumed at 475-518), so unchanged notes are never re-embedded. The only code that records which embedder produced those vectors is replace_from_index_with_embedder_stamped (lines 91-103, writes metadata embedder_id), but production never calls it — main.rs:145/app_state.rs:130 and run_reindex (app_state.rs:201) call the non-stamped replace_from_index_with_embedder. Nothing ever reads embedder_id to invalidate. Consequently: (a) swapping to a different same-dim model leaves every unchanged note carrying old-model vectors mixed with new-model vectors in one index → silently corrupt similarity space; (b) changing the embedding dimension without bumping SCHEMA_VERSION (schema.rs:8) leaves chunk_vectors at the old dim because create_schema uses CREATE VIRTUAL TABLE IF NOT EXISTS (schema.rs:204-207), so every vector insert fails — and those failures are swallowed per the finding above, yielding a silently empty semantic index.

**Why it's real:** There is no detection mechanism tying the cache's vectors to the embedder that produced them, so a model/config change corrupts semantic search results (or empties them) with no warning. schema_version alone does not capture embedder identity or dim.

**Fix sketch:** Stamp embedder_id (and embedding_dim) into metadata on every real reindex, and at startup wipe/rebuild the cache when the stored embedder_id or dim differs from the active embedder (or fold them into the schema-version wipe check).

### 7. [MEDIUM] Whole-vault reindex is one long transaction with no incremental persistence (crash-loop risk on large vaults)

- **File:** `src/cache/populate.rs` : 37-88
- **Triggering condition:** first-time build of a large vault with a slow embedder; process killed / OOM / container restart before the single commit; startup build failure triggers process::exit(1)

**What happens:** replace_from_index_with_embedder opens one transaction (lines 37-39), chunks and embeds every note in the loop (56-69), then commits once at line 86. No progress is persisted until that final commit, so any kill before commit rolls back the entire build. On startup this reindex runs synchronously (main.rs:145) and a failure calls std::process::exit(1) (main.rs:146-153). For a large vault plus a slow local embedder, if the first build exceeds the container/health-check start deadline, the process is restarted and must re-embed the entire vault from scratch each time, making zero forward progress — a crash loop that never converges.

**Why it's real:** The all-or-nothing transaction is correct for atomicity but provides no checkpointing, so cold-start robustness degrades sharply with vault size and embedder latency — a realistic condition for a public launch on modest hardware.

**Fix sketch:** Persist embeddings incrementally (e.g. commit per-note or in batches within their own transactions, keyed by content_hash so completed notes are skipped on restart), so a restart resumes rather than restarts the build.

### 8. [MEDIUM] Crash during merge leaves the repo in a merging state with no startup recovery, wedging or corrupting later commits

- **File:** `src/git/sync.rs` : 291
- **Triggering condition:** Process killed inside merge_remote between repo.merge() and cleanup_state()/commit; Remote had moved ahead (behind > 0) so a real merge was in progress; Next write triggers stage_and_commit against a repo left with MERGE_HEAD and a merged/conflicted index

**What happens:** merge_remote calls repo.merge (sync.rs:291), which writes MERGE_HEAD and mutates the index/working tree, and only afterward checks conflicts, writes the tree, creates the two-parent merge commit, and calls cleanup_state (sync.rs:293-330). If the process is killed between repo.merge() and cleanup_state(), the repo is left mid-merge. Nothing in the module ever detects or clears a leftover MERGE_HEAD at startup. On the next sync, stage_and_commit (sync.rs:234-245) uses only the current HEAD target as the single parent, ignoring MERGE_HEAD entirely: if the leftover index has conflict entries, index.write_tree() at sync.rs:231 fails ('not fully merged'), so every future sync errors out permanently; if the index was cleanly merged but uncommitted, it commits the merged tree with a SINGLE parent, dropping the remote as a recorded parent so history no longer contains the remote commit, producing repeated re-merges or a non-fast-forward push rejection.

**Why it's real:** The audit explicitly assumes the process can be killed at any instant. An interrupted merge is a real crash-consistency gap: it either permanently wedges sync (conflicted index) or silently mis-records history (single-parent commit over merged content), both data-integrity problems.

**Fix sketch:** At startup detect repo.state() != Clean (or presence of MERGE_HEAD) and cleanly abort/reset to HEAD before proceeding; in stage_and_commit refuse to commit when the repo is in a merging state.

### 9. [MEDIUM] No timer-based retry: a transient remote outage leaves commits unpushed until another write arrives or the process restarts

- **File:** `src/git/task.rs` : 116
- **Triggering condition:** Push fails with GitError::Remote (remote unreachable, auth failure, or rejection) on the last batch before writes stop; No further MCP/HTTP writes occur to trigger another debounced sync; Remote later recovers but nothing re-attempts the push

**What happens:** run_loop is purely receive-driven: a sync only fires when a WriteRecord arrives (after debounce) or once at startup via startup_flush. run_one_sync consumes the batch with std::mem::take (task.rs:121) and, on error, drops those records without requeueing; the local commit created by stage_and_commit is retained but is only re-pushed when a subsequent write triggers integrate_remote+push, or at the next process restart via startup_flush. There is no periodic/back-off timer. So after a transient remote failure, if writes stop, the commits sit unpushed indefinitely even after the remote recovers.

**Why it's real:** Git-sync's purpose is durable off-box replication before a public launch. Relying on future writes or restarts to retry means a quiet vault can remain un-backed-up for an unbounded time after a recoverable outage. status.unpushed reflects it, but nothing acts on it.

**Fix sketch:** Add a periodic retry timer (with backoff) that re-attempts integrate_remote+push whenever unpushed_count > 0 or the last attempt failed with a retryable Remote error, independent of new write traffic.

### 10. [MEDIUM] move_or_rename_note / delete_note perform a multi-file mutation with no rollback; a partial failure leaves broken backlinks, and delete_note strips a live note's assets before it is trashed

- **File:** `src/vault/write/notes.rs` : 376-411, 447-496
- **Triggering condition:** asset fs::rename fails partway through move_assets (permission, ENOSPC, a destination asset appearing between plan and move); apply_rewrites fails on note N of M; process killed between the note rename, move_assets, and apply_rewrites steps

**What happens:** move_or_rename_note renames the note (376), then move_assets one-by-one (383), then apply_rewrites over all backlink/asset rewrites (384) — none of it rolled back on failure. If move_assets fails after the note is renamed, the note sits at its new path while every other note still links to the old slug and assets are half-moved. If apply_rewrites fails on the k-th note, the first k-1 are rewritten and the rest are not. delete_note is worse: it calls move_assets to the trash (467) BEFORE fs::rename(entry.path -> trash) (468); if the note rename then fails, the still-present live note has had its attachments moved into .hatchdoor-trash out from under it, so a failed delete corrupts the note it failed to delete. Unlike move_attachment there is no rollback attempt at all here.

**Why it's real:** Any I/O error mid-operation (or a crash, which the environment says can happen at any instant) yields a permanently inconsistent vault: dangling wikilinks, half-moved asset sets, or a live note whose images have vanished. Because the vault is the source of truth this inconsistency is durable across restart/cache-rebuild.

**Fix sketch:** Sequence so the note file moves last (or first with full restore), and capture prior bytes of every note apply_rewrites touches plus each asset's original location so a failure at any step restores the exact previous on-disk state; at minimum reorder delete_note to rename the note before moving its assets.

### 11. [MEDIUM] import_attachment_bytes writes the attachment non-atomically, so a failed/interrupted overwrite truncates or corrupts the existing file

- **File:** `src/vault/write/attachments.rs` : 116-121
- **Triggering condition:** import with overwrite=true where fs::write fails partway (ENOSPC, disk error) after truncating the existing file; a reader (HTTP GET / git-sync / watcher) observing the target mid-write; process killed during the write

**What happens:** import_attachment_bytes writes directly with fs::write(&target_path, bytes), which truncates the destination and streams bytes in place. This is inconsistent with note writes (fs_ops::atomic_write does write-tmp + rename) and with the staged import path (import_attachment_file uses fs::rename). fs::write opens O_TRUNC, so when overwrite=true the previously good attachment is destroyed the instant the write starts; if the write then fails or the process dies, the target is left truncated/partial. Concurrent readers can also observe a half-written binary. The image bytes come from an HTTP body, so a client disconnect / short write is realistic.

**Why it's real:** Overwriting an existing attachment risks losing the prior good copy with no recovery, and readers can serve corrupted binaries — a data-integrity regression the note path already avoids via write-then-rename.

**Fix sketch:** Write to a sibling temp file and fs::rename it into place (mirror atomic_write), so the destination is either the old bytes or the complete new bytes and never a truncated intermediate.

### 12. [LOW] Every single note write holds vault_write_lock across a full-vault reindex, serializing all concurrent writers behind an O(vault) rebuild

- **File:** `src/handlers/write_api.rs` : 494
- **Triggering condition:** Multiple concurrent write clients (HTTP + MCP) against a large vault; Each write triggers refresh_now while still holding vault_write_lock

**What happens:** Write handlers acquire vault_write_lock (e.g. line 212) and keep it through finalize_note_write_response, which calls refresh_after_write => refresh_now (line 494). MCP tools do the same, holding the lock at src/mcp/tools.rs:62 through finalize_note_write => refresh_after_write (tools.rs:731). refresh_now => run_reindex (src/app_state.rs:186-208) rebuilds the ENTIRE VaultIndex via VaultIndex::build (walks the whole vault, index.rs:19) and re-stats/re-reads every note in replace_from_index_with_embedder (populate.rs upsert_note_if_changed calls file_snapshot + read_to_string per note). So a single one-line edit blocks every other writer for a whole-vault walk + per-file snapshot pass, not just the touched note. There is no data-integrity bug (all writes serialize correctly and note writes are atomic via temp+rename in fs_ops.rs:31), but write throughput is bounded by full-index cost per mutation.

**Why it's real:** Lower severity because vault writes are typically low-frequency and the embedding step is incremental (unchanged notes skip re-embed). The concern is scalability under concurrent write load at launch, not correctness or data loss.

**Fix sketch:** Release vault_write_lock after the filesystem mutation and run the reindex without it (the reindex only reads the vault and writes the cache, which is already serialized by refresh_lock and the cache writer mutex), or make the post-write refresh incremental (reindex only the affected slugs/paths) instead of a full VaultIndex::build.

### 13. [LOW] Note-level change detection relies on non-cryptographic FNV-1a-64; a hash collision is treated as 'unchanged'

- **File:** `src/cache/parse.rs` : 50-61
- **Triggering condition:** a note whose new on-disk content FNV-1a-64 hashes identically to its previously cached content; hostile writer able to craft colliding note content

**What happens:** content_hash uses FNV-1a 64-bit (parse.rs:50-61). upsert_note_if_changed decides a note is Unchanged when slug, file snapshot, and this hash all match (populate.rs:188-193), skipping re-index and re-embed. Any content change that collides with the cached FNV value is therefore never reflected in notes/note_fts/chunks — a silent cache/vault divergence. For an honest single-user vault the probability is negligible, but for a public multi-writer deployment a 64-bit collision is precomputable (~2^32 work), letting a writer pin stale cached content for a path.

**Why it's real:** The audit explicitly asks whether the cache can diverge from the vault without detection under hostile input; a fast non-cryptographic hash used as the sole content-change signal is the concrete mechanism.

**Fix sketch:** Use a cryptographic hash (e.g. blake3, already a dependency) for note-level content_hash, or additionally compare size/mtime plus a stronger digest before classifying a note Unchanged.

### 14. [LOW] Crash between fs::write and fs::rename leaves orphaned .hatchdoor-tmp files in the vault that are never cleaned up

- **File:** `src/vault/write/fs_ops.rs` : 31-46
- **Triggering condition:** process killed after fs::write(tmp) but before fs::rename in atomic_write

**What happens:** atomic_write writes to path.with_extension("md.hatchdoor-tmp") then renames. The temp file is only removed on the rename-error branch (line 40); if the process is killed after the write succeeds but before the rename, the <note>.md.hatchdoor-tmp file is left behind. There is no startup sweep for these, so they accumulate inside the vault tree and can be picked up and committed by the git-sync task (they are not .md so the cache ignores them, but git add of the worktree would not).

**Why it's real:** Not data loss, but stray temp files pollute the vault, may be committed/pushed by git-sync, and confuse users browsing the vault on disk; minor launch hygiene issue.

**Fix sketch:** Use a unique per-operation temp suffix and add a startup/idle sweep that removes *.hatchdoor-tmp under the vault root, and/or ensure git-sync's ignore rules exclude the temp pattern.

