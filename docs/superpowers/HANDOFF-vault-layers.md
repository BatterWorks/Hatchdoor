# Handoff — Vault Layers backend (issue #22)

**Written 2026-07-23. You are picking up mid-implementation. Read this top to bottom before doing anything.**

You are continuing a multi-group backend implementation of configurable vault layers + noise exclusion for Hatchdoor (Rust server over an Obsidian-style markdown vault). Groups A, B, C, **D** are done and reviewed. **Group E remains.** Then a final live test.

> **Update 2026-07-24 — Group D complete.** See the "Group D done → Group E" addendum at the end of this file before starting. HEAD is now `0f45b28` (was `26a9583`). The three documents below are still the source of truth; the addendum records what D delivered and the specific carry-over for E4/E6.

## The three documents that define the work

1. **Spec** (the design, approved): `docs/superpowers/specs/2026-07-23-vault-layers-and-exclusions-design.md`
2. **Plan** (the tasks, one file, no phases): `docs/superpowers/plans/2026-07-23-vault-layers-backend-implementation.md` — Groups A–E. You implement **Group D** then **Group E**.
3. **Progress ledger** (durable state, git-ignored scratch): `.superpowers/sdd/progress.md` — read it first; it records every commit, every review outcome, and every carried-forward item. If it is gone (`git clean`), reconstruct from `git log`.

## Branch and current state

- Branch: `feature/vault-layers` (pushed to `origin` = both Forgejo and GitHub; the branch is public on GitHub).
- Base of the whole feature: `06bd3c0` on `development`.
- Phase-1 (walk-level classification) + Groups A/B/C are committed. As of writing, HEAD is `26a9583`.
- **Green:** `cargo test` passes (~355 lib + 7 eval + 3 main), `cargo clippy --all-targets -- -D warnings` is clean, `cargo fmt --check` clean. Keep it that way at every group boundary.
- Toolchain is pinned to **1.97.1** in `rust-toolchain.toml` (NOT 1.96.0 — old notes say 1.96; the file is authoritative). Do not bump it.

## What each done group delivered

- **A (DB foundation):** `notes.layer TEXT NULL` column (NULL = default surface), `SCHEMA_VERSION` 7→8, layer threaded into the cache, and a **marker-set hash** that forces reclassification when markers change (without it the feature silently no-ops). A vanished-marker guard refuses silent promotion. Commits `a8cb37c 9f1ef3b 8929be3`.
- **B (read filter + wikilink fix):** `LayerSelection` selector type in `src/search/layer_selection.rs` (omitted ≡ default only; `["x"]` ≡ x only; `["default","x"]` ≡ both; `["all"]` ≡ everything; unknown name degrades to default WITH a warning, never hard-fails). Filters tree/summaries/recently-modified/stats. **Fixed the motivating bug:** `[[Melatonin]]` now resolves to the compiled page, not the clipping (`src/cache/queries/graph.rs`, `ORDER BY (layer IS NOT NULL), relative_path`). Layer threaded onto links/graph/stats via join. Commits `9e90908 0162576 668dc09 9c7910c`.
- **C (search):** demoted vectors live in a separate `chunk_vectors_demoted` vec0 table with a `layer TEXT PARTITION KEY`; default search keeps the fast unfiltered KNN on `chunk_vectors`; layer search is partition-pruned KNN. `HATCHDOOR_EMBED_LAYERS` (default true) skips demoted vectors, participates in the reset key. Commits `34029a9 26a9583`.

## CRITICAL carried-forward items — Group D and E MUST close these

These are NOT optional. They are known holes deliberately deferred, and the feature is incoherent until closed:

1. **MCP `query_notes`/`matching_note_slugs` currently pass `LayerSelection::all()`** (`src/search/mod.rs` ~line 217–222). This means demoted notes LEAK into MCP `query_notes` right now, contradicting "omitted ≡ default only". **Group D must make the MCP read tools honor the caller's selection (default = default surface).** This is the single most important correctness item left.
2. **`get_note` is slug-only.** Group D adds a `path` argument (spec "Addressing"). Demoted content must stay reachable by a stable address.
3. **`recently_modified` is HTTP-only.** Group D exposes it over MCP with a `layers` param — it is the agent's ingest-discovery path.
4. **Write tools do not refuse `.hatchdoor-layer`.** Group D: `create_note`/`import_attachment`/`move_attachment`/`rename_attachment` must hard-refuse that basename (spec "Write tools must refuse to write markers"). Currently unowned.
5. **`HATCHDOOR_EXCLUDE` env var is not wired.** The `ExcludeMatcher` supports user patterns (phase 1) but nothing reads the env var into `VaultScanConfig` on the server path. Group E, task E1. Also thread the config into `seed_empty_vault` (signature divergence flagged in phase-1 review).
6. **Startup failure is unrecoverable** (`src/server.rs:413`): a malformed marker fails the build, and the `Ok(Err(_))` arm never spawns the watcher or git sync, so fixing the marker on disk does nothing until a restart. Group E, task E3: spawn the watcher on the failure path and clear failed-state on recovery. Confirmed live — reproduces exactly.
7. **`HATCHDOOR_EMBED_LAYERS` read directly from env in populate**, not surfaced in `AppConfig`/startup log. Fold into E1's effective-config logging.
8. **Group A's WARN tells the operator to "clear the persisted marker set"** but no clear path exists yet. Group E either adds it or reword the message.

## DEPLOY-TIME rule (from Group C review — not a code fix)

The whole branch is one atomic v7→v8 schema upgrade. A cache built *mid-branch* (v8 from Group A, before C added `chunk_vectors_demoted`) mis-routes demoted vectors with no self-heal. **Therefore: ship A..E as ONE release. Before deploying, wipe any v8 cache built mid-branch (local, CI, and the demo vault host).** When live-testing D/E, always use a FRESH cache directory.

## How to execute a group (the process that has worked)

One implementer subagent per GROUP (not per task), on **opus**, doing the group's tasks in order with per-task TDD and per-task commits. Then an adversarial **opus** review at the group boundary. Fix Critical/Important via a fix subagent, re-review, then move on. This caught real bugs every group.

Scripts (in the subagent-driven-development skill dir):
`/home/battermanz/.claude/plugins/cache/claude-plugins-official/superpowers/6.1.1/skills/subagent-driven-development/scripts/`
- `review-package <BASE> <HEAD>` → writes a diff file, prints its path. BASE = the commit before the group started (from the ledger), never `HEAD~1`.
- `task-brief <PLANFILE> <N>` → extracts a task's text (optional; the plan is readable directly).

Dispatch pattern (see the prompts already used, reconstructable from this session): give the implementer the plan path + which group + the binding constraints + codegraph guidance (`codegraph explore "<symbols>"`, a `.codegraph/` index exists) + "stop and report if the plan is wrong rather than guessing" (implementers found real plan bugs repeatedly) + a report-file path under `.superpowers/sdd/groupX-report.md`. Give the reviewer the plan, the spec sections, the report, the diff-file path, and the specific risks to verify.

After each clean group: append a line to `.superpowers/sdd/progress.md`, and `git push origin feature/vault-layers`. Do NOT `git add .superpowers` (it is git-ignored; that is intentional).

## Groups remaining

- **Group D (MCP surface): DONE** (`98370b7 3475870 076b416` + review fix `0f45b28`). Closed carried items 1–4. See the addendum.
- **Group E (config/ops/attachments/demo/diagnostics):** E1 `HATCHDOOR_EXCLUDE` + startup log + seeder config; E2 watcher noise filtering + marker-triggered full reindex; E3 startup recovery; E4 attachments/assets layer + noise handling + `archive_prefix` interaction; E5 `demo_mode` server-side rejection + 404 demoted paths; E6 diagnostics surface (route + MCP tool, disabled in demo_mode). Closes carried items 5–8.

## Final live test (after E, before calling it done)

Build, run against a FRESH cache and a scratch vault with a `sources/.hatchdoor-layer` marker (see the pattern used earlier this session). Use `/api/startup-status` (`percent`, `eta_seconds`) to pace any wait instead of blind polling. Verify:
- demoted folder absent from default `/api/tree` and default search; its notes reachable via `get_note` (slug and new path arg) and via an MCP `layers:["sources"]` call;
- `[[Name]]` resolves to the default-surface note on a title collision (already fixed, confirm end-to-end);
- MCP `query_notes` with no `layers` returns default only (carried item 1 — the thing to prove);
- a noise `.obsidian/*.md` file unindexed;
- a `.hatchdoor-layer` typo no longer wedges the server permanently (carried item 6).

## Release notes to write when done

- The `SCHEMA_VERSION` 7→8 bump forces a one-time full re-embed for every existing vault on upgrade.
- The built-in noise defaults drop previously-indexed `.md` under `.trash/` (Obsidian's deleted notes), `*.sync-conflict-*` (Syncthing), and `.obsidian/`. A deployment can negate one with `HATCHDOOR_EXCLUDE=!*.sync-conflict-*`.
- New: `HATCHDOOR_EXCLUDE`, `HATCHDOOR_EMBED_LAYERS`. New vault convention: `.hatchdoor-layer` marker files.

## Scope reminders (do not reopen)

- **No web frontend.** Filtering is server-side; the UI shows default-surface only with no reveal toggle. Deliberate.
- **No file-level frontmatter demotion** — it was built then removed; folder markers only.
- **No lint logic** (orphan reports etc.); the data model landed in Group B, the checks are out of scope.
- **No per-surface `hide_from`**; demotion hides from all default surfaces together.

---

## Addendum (2026-07-24) — Group D done → starting Group E

Group D was implemented **inline** (not via a per-group implementer subagent) with per-task TDD, then an **adversarial opus subagent review at the boundary** → APPROVE-WITH-MINORS, no Critical. Gates at the boundary: `cargo test --lib` 371 pass, `cargo clippy --all-targets -- -D warnings` clean, `cargo fmt --check` clean. Branch pushed. **A fresh session can start Group E cleanly from the plan; this addendum records only what E actually needs from D.**

### State to read first
- Progress ledger `.superpowers/sdd/progress.md` — has a full "GROUP D COMPLETE" block.
- `.superpowers/sdd/groupD-review-followups.md` — the four unfixed, non-design review findings (M-2..M-5). **M-2 is relevant to E6.**
- Toolchain is **1.97.1** (`rust-toolchain.toml`), not 1.96. `SCHEMA_VERSION` is **8** — do NOT bump it again (Group A owns the one v7→v8 bump). The atomic-release / wipe-mid-branch-v8-cache deploy rule still stands; live-test E on a FRESH cache dir.

### What D added that E will reuse (don't re-derive)
- **Persisted metadata keys** (written in `src/cache/populate.rs::replace_with_options`, near the other `set_metadata` calls; read via `SqliteCache::get_metadata`): `marker_set_hash`, `marker_set` (JSON `dir→name`, from `LayerMap::named_markers()`, includes retained-vanished markers), `embed_layers` (`"true"`/`"false"`), and **new in D:** `layer_catalog` (JSON `[{name, description?}]`, read via `SqliteCache::layer_catalog() -> Vec<crate::search::LayerInfo>`). **E6 (diagnostics) should build its ruleset/marker/conflict output from these + `LayerMap` (via `VaultIndex::build(...).layers`), not from scratch.** The Group-A note stands: E6 can reuse `LayerMap::named_markers()`/`layer_names()`/`description()`/`marker_paths()`.
- **`LayerSelection`** (`src/search/layer_selection.rs`): selector semantics (omitted ≡ default only; `["x"]` ≡ x; `["default","x"]` ≡ both; `["all"]` ≡ everything; unknown name degrades to default with a warning). `parse(tokens, known_layers)`, `sql_filter(column)`, `is_all()`, `named_layers()`, `includes_default()`. E5 (`demo_mode`) must reject any layer-selecting parameter; note the web routes never accept one (B2), so most of E5 is enforced by construction — MCP is already blocked under `demo_mode` (`src/server.rs:64`).
- **Write-layer-reporting pattern** (for E4): `src/mcp/tools/write.rs::finalize_note_write` reads the note's `layer` back from the just-refreshed cache via `read_note_by_slug` and reports it in `write_success`. E4's attachment/asset layer reporting and the `archive_note` promotion-to-default signal should follow the same read-after-refresh shape. `refuse_marker_write` already exists in that file (basename-normalized, case-insensitive) — reuse it if E4 needs more marker guards.
- **Response `layer` field** already lands on `Note`/`NoteSummary`/`ModifiedNote`/`SearchResult`/`NoteLink` (`src/vault/types.rs`, `src/search/mod.rs`) with the `notes.layer` column threaded through every read query. E4's asset/attachment responses should carry `layer` the same additive way (serialize as string-or-null, no `skip_serializing_if`).
- **AppState** gained `mcp_tools_changed: broadcast::Sender<()>` (D2). There are **7 `AppState { … }` construction sites** (1 real in `server.rs`, the rest test helpers in `server.rs`/`app_state.rs`/`mcp/routes.rs`); if E adds another field, expect to touch all 7.

### D2 design decision E should be aware of
`capabilities.tools.listChanged` is advertised **true** but there is **no live delivery** over the current stateless POST-only MCP transport (GET → 405, no SSE). `run_reindex` fires `AppState.mcp_tools_changed` on a marker-set-hash change and `protocol::tools_list_changed_notification()` builds the message, but nothing consumes the broadcast yet. Left as-is deliberately (plan-mandated + MCP-spec-permitted). If E adds any streaming transport, that broadcast is the seam to wire.

### Test scaffolding to copy
- Layered MCP tests: `mcp/routes.rs` has `layered_test_state()` (a `sources/` demoted layer + a default note) and a `call_tool(...)` helper — reuse for any E MCP-facing test.
- Cache-level layer tests: `cache/queries/metadata.rs` has `build_layered_cache()`; populate tests use `demoted_vault_with_flag()`. `StubEmbedder::new(384)` + `SqliteCache::in_memory(384)` + `VaultIndex::build(dir)` is the standard fixture.

### Group E carried items to close (from the list above, still open)
5 (`HATCHDOOR_EXCLUDE` env not wired + seeder config), 6 (unrecoverable startup on a malformed marker — confirmed live), 7 (`HATCHDOOR_EMBED_LAYERS` read from env, not surfaced in AppConfig/startup log — fold into E1's effective-config logging), 8 (Group A's WARN tells the operator to "clear the persisted marker set" but no clear path exists — E adds one or rewords). Then run the **final live test** in the section above on a fresh cache.
