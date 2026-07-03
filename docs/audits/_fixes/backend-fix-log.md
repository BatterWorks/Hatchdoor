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
