# Backend robustness audit — fix log

Running log of fixes applied against the findings in
`docs/audits/backend-robustness/` (SUMMARY.md + `NN-*.md`). Worked in
severity tiers (high → medium → low), TDD (test-first, watch-fail, minimal
fix), one commit per fix.

Legend: ✅ fixed · ⏭️ deferred/decision · 🔁 deduped into another finding.

## Dedup notes
- **05-HIGH ≡ 06-MED** (MCP unauthenticated / bypasses web-auth) → single fix.
- **07-HIGH ≡ 06-MED** (2 MB upload cap / no `DefaultBodyLimit`) → single fix.

---

## Medium tier

### ✅ 06-MED — Insecure-by-default web auth on a public bind (`src/main.rs`) — **user decision**
- **Decision (user):** refuse to start on a non-loopback host when no web token is set (loopback stays open). Chosen over "auto read-only" and "louder warning".
- **Fix:** added `is_loopback_host` (`127.0.0.1`/`::1`/`[::1]`/`localhost`) and `check_web_auth_posture(host, has_token)`, which returns an error when the host is non-loopback and `HATCHDOOR_WEB_BEARER_TOKEN` is unset. `run_server` logs it via `error!` and `exit(1)` instead of the previous info-level "reachable unauthenticated" note. Documented the hard requirement in `.env.example`.
- **⚠️ Deploy impact:** the batterbrain deploy runs `HOST=0.0.0.0`; it now **must** have `HATCHDOOR_WEB_BEARER_TOKEN` set in `.env` or the container won't boot.
- **Test (RED→GREEN):** `web_auth_posture_refuses_public_bind_without_token`.
- **Commit:** _(see git log)_

### ⏭️ 02-MED — Embedding inside one big write transaction (WAL growth / no incremental durability) — **skipped by user**
- **Decision (user):** skip the batch-commit refactor to keep reindex atomic (readers never see a partially-rebuilt cache); accept the WAL-growth / no-crash-progress risk on large rebuilds. The small TOCTOU read-once cleanup (02-LOW / #10) was still applied separately.

### ✅ 02-MED — Embeddings reused across a model swap, mixing incompatible vector spaces (`cache/`)
- **Fix:** added `Embedder::identity()` (model id + dim; `FastembedEmbedder` → e.g. `NomicEmbedTextV15-768`, `StubEmbedder` → `stub-384`). `replace_from_index_with_embedder` now (1) calls new `reset_if_embedder_changed`, which wipes + recreates the schema when the cache already carries a *different* embedder id (so no old-model vectors are preserved via `preserve_existing_vectors`), and (2) stamps the current `embedder.identity()` into metadata after commit. The production build/reindex paths use this base method, so identity is now recorded and validated in prod (previously only the test-only `_stamped` path wrote it). A cache with no stored id (fresh, or built by the old code) is left alone and simply stamped — there is no prior model to conflict with.
- **Test (RED→GREEN):** `swapping_the_embedder_model_rebuilds_the_vector_index` — build with `model-a`, rebuild the byte-identical vault with `model-b`, assert `model-b` actually re-embeds (call count > 0) and `embedder_id` becomes `model-b`. RED: `embedder_id` was never stamped (None) and the unchanged note reused model-a's vectors.
- **Commit:** _(see git log)_

### ✅ 04-MED — Multi-file backlink/asset rewrites applied with no rollback (`vault/write/rewrites.rs`)
- **Fix:** `apply_rewrites` is now all-or-nothing. It captures each target's current content before overwriting it and, if any later write fails, restores every file already written in the batch (reverse order). Previously a failure on the k-th of N notes left the first k-1 rewritten, leaving dangling/duplicated backlinks and asset references across the vault after a failed move/rename/delete/archive.
- **Test (RED→GREEN):** `apply_rewrites_rolls_back_written_files_when_a_later_write_fails` — first rewrite lands, second (to a non-existent dir) fails; the first must be restored to its original content. RED left it as "new A".
- **Commit:** _(see git log)_

### ✅ 07-MED — Read-path handlers discarded the server's `{error}` body, showing only the status (`frontend`)
- **Fix:** every read fetch threw `Failed loading X: <status>` without parsing the server's structured `{error}` JSON, so a 404's "Note not found: <slug>" or a 500's real cause was replaced by a bare code. Added a shared `readErrorMessage(res, fallback)` helper (mirrors writeApi's `parseError`) and used it in all read fetches: `NotePage` (note, links, reload), `App` (tree, recently-modified, search), `GraphPage`, `StatsPage`.
- **Test (RED→GREEN):** `apiError.test.ts` — returns the server error field when present, falls back to `<fallback>: <status>` otherwise. Frontend: typecheck clean, 127 tests pass.
- **Commit:** _(see git log)_

### ✅ 04-MED — `atomic_write` did not fsync the temp file or parent dir (`vault/write/fs_ops.rs`)
- **Fix:** `atomic_write` now creates the temp file, `write_all` + `sync_all()` (fsync data+metadata) it before the rename, and fsyncs the parent directory after the rename so the directory entry change is durable. Previously a crash right after `fs::rename` returned could leave the note's name pointing at never-flushed (empty/truncated) data.
- **Test:** `atomic_write_persists_content_and_leaves_no_temp_file` (content round-trips on create + overwrite, temp sidecar removed). NB: fsync *durability* across power loss is not observable in a unit test, so this is a regression guard for the rewrite, not a RED for the flush itself (same limitation noted for 04-HIGH).
- **Commit:** _(see git log)_

### ✅ 05-LOW — MCP protocol version was exact-match, locking out compatible clients (`mcp/config.rs`, `routes.rs`)
- **Fix:** `initialize` hard-coded the server version and ignored the client's requested `protocolVersion`; follow-up requests then required a byte-exact `MCP-Protocol-Version` header. Added `SUPPORTED_PROTOCOL_VERSIONS` (current + known prior revisions), `is_supported_protocol_version`, and `negotiate_protocol_version`. `validate_mcp_request` now accepts any supported header value; `handle_initialize(params)` echoes the client's requested version when supported, else the preferred one — so whatever is negotiated at initialize is accepted on later requests.
- **Test (RED→GREEN):** `supported_alternate_protocol_version_header_is_accepted` (a `2025-06-18` header now returns tools instead of `-32002`), `initialize_echoes_supported_client_protocol_version`. Updated `unsupported_protocol_version_is_rejected` to use a truly unknown version.
- **Commit:** _(see git log)_

### ✅ 05-LOW — Inconsistent "note not found" surface between MCP read and write tools (`mcp/tools.rs`)
- **Fix:** read tools return a missing note as an `isError` tool result, but write tools' `note_entry` returned a JSON-RPC `-32602` protocol error. Added `JsonRpcFailure::not_found` (carries a `tool_level` flag); `handle_tools_call` renders any `tool_level` failure as `tool_error(...)` at the single dispatch point, so all tools report "not found" the same way. Empty/invalid slug stays a protocol `invalid_params` (matching reads). No per-tool churn across the 11 write tools.
- **Test (RED→GREEN):** `write_tool_missing_note_is_a_tool_error_not_a_protocol_error` — `edit_note` on a missing slug returns `result.isError == true`, no `error` object. RED returned a `-32602`.
- **Commit:** _(see git log)_

### ✅ 07-LOW — JSON write-body rejections coerced to 400 regardless of real status (`handlers/write_api.rs`)
- **Fix:** `write_payload` now returns `rejection.status()` instead of a hardcoded `BAD_REQUEST`, keeping the `{error}` body. A body over the length limit is now 413, a bad content-type 415, and a well-formed-but-invalid body 422 — previously all flattened to 400, which misleads status-code-based clients/proxies/monitoring.
- **Test (RED→GREEN):** `write_api_oversized_json_body_reports_413_not_400` (a >2 MB JSON body → 413). Also updated `write_api_rejects_update_payload_missing_expected_hash` to expect 422 (the correct status for a missing required field); the frontend only special-cases 409 and otherwise shows the `{error}` message, so this is UX-neutral.
- **Commit:** _(see git log)_

### ✅ 04-LOW — Dead `allow_trash_collision` branch broke deleting a note whose asset name is already in trash (`vault/write/assets.rs`)
- **Fix:** the collision check was `if destination_asset.exists() { Err } if allow_trash_collision && destination_asset.exists() { Err }` — the first `if` fired unconditionally, so the second (trash-collision) branch was unreachable and a trashed asset of the same name always failed the delete. Now, when `allow_trash_collision` is set (delete-to-trash), the destination is resolved via the existing `unique_trash_attachment_relative_path` helper (e.g. `foo.png` → `foo-2.png`) instead of erroring; the hard conflict only applies to real move/rename destinations.
- **Test (RED→GREEN):** `trashing_an_asset_whose_name_already_exists_in_trash_picks_a_unique_name`. RED returned `Conflict`.
- **Commit:** _(see git log)_

### ✅ 06-LOW (partial) — Web token in `?access_token=` leaked into trace spans (`src/auth.rs`, `main.rs`)
- **Fix:** the request trace span logged the full URI, so at debug level the web token carried by `<img>`/download URLs (`?access_token=…`) was recorded. Replaced `DefaultMakeSpan` with a custom span whose `uri` field runs the query through new `redact_query_token` (`access_token=REDACTED`), so the raw token never reaches the span.
- **Deferred (feature, not a LOW-sized fix):** the token still rides in the URL itself (browser history, proxy access logs, Referer). Fully closing that needs a short-lived, scoped signed token (or a cookie derived from the Authorization header) for asset/download URLs — a frontend+backend change tracked separately, not attempted here.
- **Test (RED→GREEN):** `redact_query_token_hides_only_the_access_token`.
- **Commit:** _(see git log)_

### ✅ 02-LOW — Note content read twice per reindex (TOCTOU) (`cache/populate.rs`)
- **Fix:** `upsert_note_if_changed` read + hashed the file, then `chunk_and_embed_note` read the same file a *second* time; a mid-reindex edit between the two reads could chunk content that disagrees with the stored `content_hash`. `UpsertOutcome::Wrote` now carries the already-read `content`, threaded into `chunk_and_embed_note` (which dropped its own `fs::read_to_string` and the now-unused `entry` param). One read per note, chunk text guaranteed to match the stored hash.
- **Test:** behavior-preserving refactor — covered by the existing chunk/embed suite (`cache::` 38 tests green before and after); no isolated RED since the window is an inherently racy TOCTOU, not unit-triggerable.
- **Commit:** _(see git log)_

### ✅ 02-LOW — Interrupted first-time schema init bricked startup (`cache/schema.rs`)
- **Fix:** two parts. (1) `existing_schema_version` now returns a `SchemaState` enum; the two half-initialised states (objects but no metadata table; metadata table but no `schema_version` row) return `Corrupt` and `ensure_schema` **wipes + rebuilds** them (with a `warn!`) instead of returning a hard error that `main.rs` turned into `exit(1)` on every restart. (2) `create_schema`'s DDL is now wrapped in `BEGIN;`/`COMMIT;`, so an interrupted build rolls back to an empty DB (a clean "fresh" state) rather than leaving the half-created state at all.
- **Test (RED→GREEN):** `interrupted_schema_init_rebuilds_instead_of_bricking_startup` — open on disk, delete the `schema_version` row to mimic the crash window, reopen must rebuild (not error). RED failed with "metadata exists but schema_version is missing".
- **Commit:** _(see git log)_

### ✅ 01-MED — A panicking lock holder permanently poisons the SqliteCache writer Mutex (`cache/mod.rs`)
- **Fix:** `connection()` (and the read-pool lock) now recover a poisoned `Mutex` via `unwrap_or_else(|p| p.into_inner())` instead of returning an error. A panic while the lock was held used to wedge every future reindex/cache write for the process lifetime. The SQLite connection stays consistent across a panic (a rusqlite `Transaction` rolls back on unwind), so recovering the guard is safe.
- **Test (RED→GREEN):** `writer_lock_recovers_after_a_panicking_holder` — a thread panics while holding the writer lock; a later `set_metadata`/`get_metadata` must still succeed. RED failed with "connection lock poisoned". All 36 cache tests green.
- **Commit:** _(see git log)_

### ✅ 03-MED — No timed retry: a transient remote failure strands commits unpushed (`git/task.rs`)
- **Fix:** `run_loop` now arms a bounded exponential backoff (`RETRY_BASE=5s` → `RETRY_MAX=300s`) after a sync that failed transiently. New `next_record_or_retry` races the backoff timer against `receiver.recv()`: when the timer wins it re-runs `run_one_sync` with an empty batch (no new write needed) and grows/clears the backoff based on the outcome; a new write wins the race and resets to base. `run_one_sync` now returns whether the failure was transient (`Remote`/`Other`) — a conflict, dirty tree, or validation error is *not* retried (it needs the remote or a human to change first), so the loop never spins.
- **Test (RED→GREEN):** `failed_sync_is_retried_without_a_new_write` — a push that always fails is attempted ≥2 times after a single write, only via the backoff timer. RED had exactly 1 attempt (no retry path). All 23 git tests green.
- **Commit:** _(see git log)_

### ✅ 02/03/03/03-MED+LOW — Sync only staged the batch's paths, stranding/blocking every other on-disk vault change (`git/sync.rs`)
**One root fix resolves four findings** (03-MED re-stage, 03-MED dirty-tree-blocks-push, 01-MED crash-strands-edit, 03-LOW spurious-dirty race). Root cause: `stage_and_commit` staged only the explicit batch paths, so any other working-tree change was neither committed (→ stranded out of git) nor allowed through a merge (→ `DirtyWorkingTree` refusal blocked all pushes forever).
- **Fix:** replaced `stage_and_commit` with `commit_working_tree`, which stages the **whole** working tree (`add_all` + `update_all` = `git add -A`, honouring `.gitignore`) and commits if it differs from HEAD. Used in `commit_local` (so batch + stranded + manual + startup-flush edits are all captured) and at the top of `integrate_fetched` (so a write that raced into the lock-free fetch window, or a manual edit, is committed **before** the merge). Removed the merge-time `DirtyWorkingTree` refusal + `dirty_tracked_files` helper: pending edits are now auto-committed (they are the source of truth on disk) instead of being refused forever or force-discarded. Conflicts still abort cleanly and keep the local commit.
- **Test (RED→GREEN):** `sync_commits_uncommitted_vault_changes_not_in_the_batch` (empty batch still flushes a stranded file — RED was `NoChanges`); `sync_auto_commits_uncommitted_manual_edit_instead_of_refusing` (rewrote the old `sync_refuses_...` test — RED was `DirtyWorkingTree`, now the edit is committed + pushed). All 22 git tests green.
- **Commit:** _(see git log)_

## High tier

### ✅ 03-HIGH — Crash mid-merge wedges all future syncs (`git/sync.rs`)
- **Fix:** added `recover_interrupted_state(repo)` called at the top of `sync()`. If `repo.state()` is not `Clean` (interrupted merge/revert/etc.), it hard-resets the working tree/index to HEAD and calls `cleanup_state()` before staging, so `write_tree` no longer fails on a half-merged index. The remote integration is simply redone on the same sync. Logs a `warn!` when it recovers.
- **Test (RED→GREEN):** `sync_recovers_from_interrupted_merge_state` — leaves the repo in a conflicted `Merge` state (as a crash would) and asserts a later `sync()` surfaces a clean `Conflict` and ends in `RepositoryState::Clean`, instead of the opaque `Other("cannot create a tree from a not fully merged index")` wedge.
- **Commit:** _(see git log)_

### ✅ 01-HIGH — Git write-lock held across network I/O blocks all vault writes (`git/task.rs`, `git/sync.rs`)
- **Fix:** split `sync()` into four phase functions — `commit_local` (working tree + index), `fetch_remote` (network read), `integrate_fetched` (merge/checkout), `push_branch` (network write). The background task (`run_sync_phases`) now holds `vault_write_lock` **only** across the two local phases (`commit`, `integrate`) and **releases** it across the network phases (`fetch`, `push`). A slow/hanging remote can no longer block concurrent HTTP/MCP vault writes — at worst it delays the sync task, while writes still land on disk and enqueue. `sync()` is kept as the composed orchestrator so all existing behaviour/tests hold. Runner injection changed from a single closure to a `SyncOps` struct of four phase closures.
- **Test (RED→GREEN):** `network_phase_does_not_hold_vault_lock` — injects a `fetch` that blocks (hung remote) and asserts `vault_write_lock.try_lock()` succeeds while it blocks. Proved it bites by temporarily reintroducing the whole-op lock (test went RED/hung). All 21 git tests green; binary builds.
- **Residual:** git2-rs exposes no reliable connect-phase timeout, so a hung network op still stalls the *sync task* itself (unpushed changes accumulate — covered by the 03-MED "no timed retry" finding, fixed later). The write path is no longer affected, which was the HIGH impact.
- **Commit:** _(see git log)_

### ✅ 02-HIGH — Per-note embed failure permanently diverges the cache (`cache/populate.rs`)
- **Fix:** in `replace_from_index_with_embedder`, when `chunk_and_embed_note` fails for a note (whose `notes` row was already written with the new `content_hash`), call new `invalidate_note_content_hash(slug)` to reset the stored hash to `""`. Change-detection keys off `content_hash`, so this guarantees the note is re-processed on the next reindex (startup / watcher / MCP write / `/api/refresh`) once the embedder recovers — instead of being seen as `Unchanged` forever with stale or absent chunks. Catches both the brand-new-note (0 chunks, invisible to semantic search) and updated-note (stale chunks) cases; no schema change.
- **Test (RED→GREEN):** `per_note_embed_failure_self_heals_on_next_reindex` — first reindex with a `FailingEmbedder` leaves the note with 0 chunks (failure swallowed); a second reindex with a working embedder must re-chunk it. RED showed it stuck `Unchanged` (0 chunks); GREEN re-chunks.
- **Commit:** _(see git log)_

### ✅ 07-HIGH (≡ 06-MED upload cap) — Attachment uploads fail at axum's 2 MB default body limit, not the advertised 10 MB (`main.rs`)
- **Fix:** `build_router` now installs `DefaultBodyLimit::max(max_attachment_bytes + ATTACHMENT_MULTIPART_OVERHEAD)` scoped to the `/api/attachment` route (overhead = 64 KiB for boundary/field framing). The framework no longer rejects 2–10 MB bodies before the handler runs, so the handler's real `max_attachment_bytes` check (default 10 MB) and its clean `attachment exceeds max size` error are reachable. All other routes keep axum's small default limit, so JSON write bodies stay bounded.
- **Test (RED→GREEN):** `router_accepts_attachment_between_2mb_and_configured_max` — uploads a 3 MB file and asserts 200 + the file landing in the vault. RED returned 400 (framework `length limit exceeded`); GREEN returns 200.
- **Commit:** _(see git log)_

### ✅ 05-HIGH (≡ 06-MED auth) — MCP read-only mode is unauthenticated and `/mcp` bypasses web auth (`mcp/config.rs`)
- **Fix:** require an MCP bearer token whenever MCP is **enabled**, not only in write mode. (1) `McpConfig::validate()` now rejects `enabled && bearer_token.is_none()` — so `from_env_validated()` fails startup fast (matching write-mode behaviour). (2) `validate_mcp_request()` now rejects any request when `bearer_token.is_none()` (defense-in-depth for the running server) instead of only under `write_enabled`. Read tools (`get_tree`/`get_note`/`search_notes`/…) are no longer reachable on the un-web-authed `/mcp` route without a credential. Docs (README, `.env.example`) updated to state a token is required for read-only too.
- **Test (RED→GREEN):** `validate_rejects_enabled_without_token` (config) and `validate_mcp_request_rejects_read_only_without_token` (request-gate) — both asserted the old code allowed tokenless read-only (RED), now rejected (GREEN, 401). Test infra updated: `enabled_config()` carries a token and `post_json` sends it, so the read-only route tests now exercise the authenticated path; `bearer_token_is_enforced_when_configured` still proves a mismatched/absent token → 401. Full suite green (217+7+18).
- **Commit:** _(see git log)_

### ✅ 04-HIGH — `delete_note` moves assets before renaming the note, no rollback (`vault/write/notes.rs`, `fs_ops.rs`)
- **Fix (two parts):** (1) `move_assets` is now **all-or-nothing** — if any rename fails it rolls back the assets already moved in that call, so callers never see a partially-moved set. (2) `delete_note` now trashes the **note first**, then moves assets (mirroring the already-safe `move_or_rename_note`); if `move_assets` fails, the note is restored out of trash so the whole delete is a no-op. Previously assets were relocated into trash *before* the note rename, so a rename failure left a live note pointing at attachments that had already moved, with no rollback.
- **Test (RED→GREEN):** `move_assets_rolls_back_already_moved_on_failure` (fs_ops) — two moves where the second fails (ENOENT); asserts the first is rolled back to its source and not left at the destination. RED (old no-rollback `move_assets`) leaves it at the destination; GREEN restores it. All 21 vault::write tests still pass.
- **Note:** injecting a mid-`delete_note` fs failure through the public API isn't feasible without fs mocking or root-unsafe permission tricks, so the rollback correctness rests on the `move_assets` unit test plus structural symmetry with `move_or_rename_note`.
- **Commit:** _(see git log)_
