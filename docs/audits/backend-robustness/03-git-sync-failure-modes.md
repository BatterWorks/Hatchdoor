# Git-sync failure modes

**Summary:** 5 confirmed (1 high, 3 medium, 1 low), 0 refuted. LOW/unverified findings were not voted on.

## Confirmed findings

### HIGH: Crash mid-merge leaves the repo in MERGE state; every later sync fails at write_tree with no auto-recovery

- **Trigger conditions:** remote diverged so a merge runs (integrate_remote -> merge_remote); process killed between repo.merge() and repo.cleanup_state()/reset (sync.rs:291-330); on restart startup_flush runs a sync while .git is still in Merge state
- **Location:** src/git/sync.rs:291
- **What happens:** merge_remote() calls repo.merge() (line 291), which persists MERGE_HEAD and puts the repo in RepositoryState::Merge, and only clears it via reset+cleanup_state (conflict path, 308-309) or cleanup_state (clean path, 330). If the process is killed anywhere between the merge and that cleanup, the on-disk index is left half-merged/conflicted. Neither validate_repo() nor sync()/stage_and_commit() ever inspect repo.state() or call cleanup_state()/merge-abort at startup. On the next sync, stage_and_commit() loads that index (repo.index(), line 215) and calls index.write_tree() (line 231), which errors on a not-fully-merged index. Every subsequent sync then fails with GitError::Other and the subsystem is wedged until a human runs `git merge --abort` on the container.
- **Why:** Data-integrity/durability on a target that can be 'killed at any instant': a well-defined crash window silently and permanently stops all pushing of vault edits to the remote, recoverable only via manual shell access. The startup recovery path (startup_flush) actively re-triggers the failure instead of healing it.
- **Fix sketch:** At startup (validate_repo or before startup_flush) check repo.state(); if it is Merge/RevertHead/etc., call repo.cleanup_state() and hard-reset the working index back to HEAD before syncing. Also guard sync() by aborting a stale merge state before staging.

### MEDIUM: No timed retry: a transient remote/network/auth failure strands committed edits unpushed until the next write or a full process restart

- **Trigger conditions:** git push/fetch fails transiently (remote unreachable, auth blip, non-fast-forward) -> GitError::Remote; no further MCP/HTTP writes occur afterward
- **Location:** src/git/task.rs:91
- **What happens:** run_loop() only ever calls run_one_sync() from two places: startup_flush (line 87) and after a debounce window that begins with receiver.recv().await (line 93). The outer loop blocks on recv() with no periodic timer. When sync() commits locally but push()/fetch fails (sync.rs:198-199 -> GitError::Remote), the commit is retained (good) but the failed batch is dropped via std::mem::take (task.rs:121) and there is no scheduled re-attempt. The comment 'Retried on the next batch' (sync.rs:36) only holds if another write arrives; if writes stop, the local commit sits unpushed indefinitely until the process is restarted and startup_flush notices has_unpushed.
- **Why:** For a launch, a brief network/remote outage can leave hours/days of vault commits unreplicated to the remote with no self-healing, defeating the backup guarantee. It is observable (status.last_ok=false) but not self-correcting.
- **Fix sketch:** Add a retry timer in run_loop: when the last sync failed with a retryable kind (remote/other) or unpushed_count>0, arm a backoff sleep in the select! so run_one_sync(empty batch) is re-attempted without needing a new write.

### MEDIUM: Uncommitted working-tree changes stranded by a failed commit-stage (or crash between index.write and commit) are never re-staged

- **Trigger conditions:** stage_and_commit errors after the file is on disk (e.g. index.write/write_tree/commit fails, or process killed between index.write (line 229) and repo.commit (line 241)); the same note is not written again by a later batch
- **Location:** src/git/sync.rs:216
- **What happens:** sync() only stages the explicit paths of the current batch (stage_and_commit loop, lines 216-228). Recovery of stranded work relies solely on has_unpushed()/unpushed_count(), which count committed-but-unpushed commits via graph_ahead_behind (sync.rs:140-154, 159-175); they do NOT detect uncommitted working-tree or staged-but-uncommitted changes. If a batch's commit fails after the vault file was already written to disk, that path is dropped with the batch (task.rs:121) and is never part of any future batch, so its change is never committed/pushed unless the same file happens to be edited again. startup_flush passes an empty batch (task.rs:88) and stages nothing, so a restart does not recover it either.
- **Why:** The vault file on disk is safe (source of truth), but the git backup silently and permanently omits that edit with no signal beyond a stale last_error. Worse, a stranded modification to a *tracked* file becomes WT_MODIFIED and then wedges all future merges (see dirty-tree finding).
- **Fix sketch:** On sync entry or startup, additionally stage any dirty tracked files (or run an add_all over the vault) so uncommitted vault state is always captured, not just the paths of the in-memory batch.

### MEDIUM: A single uncommitted tracked-file edit permanently blocks all pushes whenever the remote diverges (DirtyWorkingTree), with no auto-recovery

- **Trigger conditions:** someone edits a tracked .md directly on the server without committing (or an edit was stranded uncommitted per the previous finding); remote later moves ahead so a merge is required (behind>0)
- **Location:** src/git/sync.rs:285
- **What happens:** merge_remote() refuses to integrate when dirty_tracked_files() is non-empty (lines 285-288), returning GitError::DirtyWorkingTree so the force checkout cannot discard the edit. This is correct for avoiding data loss, but there is no automatic remediation: as long as one uncommitted tracked-file modification exists AND the remote is ahead, every batch fails at integrate_remote and nothing is pushed. Local commits accumulate unpushed (unpushed_count grows) and the only escape is a human committing/reverting the file on the container. run_one_sync merely records the error (task.rs:190) and moves on.
- **Why:** One stray editor save (or a stranded edit) can silently halt the entire backup pipeline indefinitely. For an unattended public deployment this converts a benign situation into a persistent, human-only-recoverable outage of git sync.
- **Fix sketch:** Auto-commit dirty tracked files into their own commit before merging (they are already the source of truth on disk), instead of refusing forever; or surface an actionable recovery path. At minimum, stage+commit the dirty edit so the merge can proceed without discarding it.

### LOW: Spurious DirtyWorkingTree failure when a next-batch write races the sync window

- **Trigger conditions:** remote is ahead so a merge is required; an MCP write to an existing (tracked) note completes in the gap between the debounce timer firing (task.rs:104) and run_one_sync acquiring vault_write_lock (task.rs:149)
- **Location:** src/git/task.rs:149
- **What happens:** MCP writes write the file to disk then record() under vault_write_lock (mcp/tools.rs:62), releasing the lock before the git task grabs it. A write that lands after the current batch's debounce timer fires but before run_one_sync() acquires the lock leaves a WT_MODIFIED tracked file on disk that belongs to the *next* batch. When the current batch then needs a merge, dirty_tracked_files() (sync.rs:338) sees that not-yet-committed edit and merge_remote returns GitError::DirtyWorkingTree, failing the current batch's push even though nothing is actually wrong. No data is lost (the edit commits in the next batch), but the current push is spuriously blocked and status shows a dirty_tree error.
- **Why:** Under concurrent load with a diverging remote this produces confusing intermittent sync failures; combined with the no-timed-retry gap it can delay pushes noticeably.
- **Fix sketch:** Only treat files as 'dirty manual edits' if they are not part of the pending queue, or drain newly-arrived records into the current batch (and stage them) before deciding the tree is dirty.

## Refuted (not real / already handled)

(None)
