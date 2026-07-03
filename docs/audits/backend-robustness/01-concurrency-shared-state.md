# Concurrency & shared-state coordination

3 confirmed (1 high, 2 medium), 0 refuted. All findings were verified by consensus review.

## Confirmed findings

### HIGH: Git sync holds the shared vault_write_lock across un-timed-out network fetch/push, so a slow or hanging remote blocks every HTTP and MCP vault write

- **Trigger conditions:** slow/hanging/unreachable git remote (dropped TLS, firewall blackhole, dead server); any concurrent HTTP or MCP write while a sync is in flight; a remote that keeps the TCP connection open without responding
- **Location:** src/git/task.rs:149-166
- **What happens:** run_one_sync acquires the shared vault-mutation lock and holds it across spawn_blocking(...) calls. The runner performs git fetch/push via git2 without any timeout, so these calls block for as long as the remote keeps the socket alive. Every write handler blocks on vault_write_lock for that entire window. A persistently slow/hanging remote makes all note creation/edit/move/delete and attachment writes hang.
- **Why:** The adverse condition 'slow/failing/rejecting git remote' is explicitly in scope. Coupling the vault write-availability to the responsiveness of an external git remote is an availability failure.
- **Fix sketch:** Do the network phase (fetch/push) outside the vault_write_lock. Set a connect/transfer timeout on git2 fetch/push operations.

### MEDIUM: A crash between a vault file write and its debounced git commit strands the edit out of git, and the stranded dirty file later makes every merge-requiring sync fail

- **Trigger conditions:** process crash/kill between atomic_write and the next debounced git sync; then the remote moving ahead so a later sync needs a merge
- **Location:** src/git/task.rs:23
- **What happens:** Write handlers write the vault file to disk and enqueue a WriteRecord onto an in-memory channel that is only drained after debounce. If the process is killed in that window, the file is on disk but the WriteRecord is lost. On restart, startup_flush only runs when has_unpushed is true, which compares local vs remote commits. A stranded file is a modified tracked file, so the next sync that must integrate remote changes hits dirty_tracked_files and aborts.
- **Why:** 'Process crash mid-operation' is a named adverse condition. The coordination between volatile write-record queue and durable vault leaves changes silently un-synced.
- **Fix sketch:** On startup reconcile actual working-tree status against git: if git status shows dirty tracked files, stage+commit them. Alternatively persist pending WriteRecords.

### MEDIUM: A panic while the SqliteCache writer Mutex is held poisons it permanently, killing all future reindexes and cache writes for the process lifetime

- **Trigger conditions:** any panic inside the reindex transaction while holding the writer lock; e.g. bytemuck::cast_slice on a corrupted/partially-written chunk_vectors blob
- **Location:** src/cache/mod.rs:164
- **What happens:** SqliteCache.conn is a std::sync::Mutex. replace_from_index_with_embedder holds the guard for the entire reindex transaction. preserve_existing_vectors does bytemuck::cast_slice, which panics if blob length is not a multiple of 4. Any panic there poisons the Mutex permanently.
- **Why:** A single panic converts a transient fault into a permanent process-wide degradation with no self-recovery.
- **Fix sketch:** Recover from poisoned lock instead of propagating error indefinitely. Guard preserve_existing_vectors against non-multiple-of-4 blob lengths.

## Refuted (not real / already handled)

(No findings were refuted.)
