# Backend robustness audit (Workflow 2)

Pre-public-launch audit of the Hatchdoor **Rust backend** (`src/`) for data
integrity, concurrency, and security. Sibling to the client edge-case audit;
built on the reusable engine in `docs/audits/_scaffold/audit-workflow.scaffold.js`.

## How it runs

A disk-backed, resumable workflow. Each category is found (Opus), adversarially
verified by a **3-lens panel** — `code-truth` / `failure-injection` /
`already-handled` — and written to its own file by a scribe (Haiku). Categories
run concurrently; each self-checkpoints to `state/`, so a resume picks up exactly
where an interrupt left off.

**Verification is strict:** `severityPolicy = { critical:3, high:3, medium:3, low:1 }`
— even low-severity findings get a vote, so the finder cannot self-certify a
data-loss or security bug into the report unchecked.

**A category is "done" when both its `NN-slug.md` report AND
`state/NN-slug.verdicts.json` exist.** `SUMMARY.md` is built deterministically
in-script from the verified data (with cross-category dedup) — not by an LLM.

## Running / resuming

Not started yet. To run (or resume after an interruption):

`Workflow({ scriptPath: "/home/battermanz/coding/hatchdoor/docs/audits/backend-robustness/state/_audit-workflow.js" })`

## Categories

| File | Scope | Key sources |
|---|---|---|
| `01-concurrency-shared-state.md` | AppState locks, run_blocking, refresh vs. in-flight ops, watcher races, locks across await | `app_state.rs`, `vault_watcher.rs`, `vault/index.rs` |
| `02-sqlite-cache-atomicity.md` | Transactions/WAL, crash-mid-populate, concurrent read during refresh, cache↔vault divergence | `cache/populate.rs`, `cache/queries.rs`, `cache/schema.rs` |
| `03-git-sync-failure-modes.md` | Commit/push coalescing, GitError paths, dirty/conflict/reject/unreachable remote, data-loss on force/reset | `git/sync.rs`, `git/task.rs`, `git/status.rs` |
| `04-vault-write-path-safety.md` | Atomic writes, path traversal/symlink escape, filename sanitization, concurrent writes, link rewrites | `vault/write/notes.rs`, `vault/write/paths.rs`, `vault/write/attachments.rs` |
| `05-mcp-protocol-surface.md` | MCP input validation, auth, oversized/malformed requests, shared write-layer reach | `mcp/tools.rs`, `mcp/routes.rs`, `mcp/protocol.rs` |
| `06-auth-http-handlers.md` | Bearer-token auth coverage/timing, body-size limits, download/asset path safety | `auth.rs`, `handlers/write_api.rs`, `handlers/downloads.rs` |
| `07-api-error-shape-seam.md` | ErrorResponse/status-code shapes the frontend assumes vs. what handlers return | `api_types.rs`, `handlers/api.rs`, `frontend/src/api.ts` |

`SUMMARY.md` is regenerated from the per-category verified data on each pass.
