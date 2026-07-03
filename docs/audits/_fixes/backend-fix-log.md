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
