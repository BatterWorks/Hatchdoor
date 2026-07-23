# Handoff — Vault Layers backend (issue #22)

**Written 2026-07-23. You are picking up mid-implementation. Read this top to bottom before doing anything.**

You are continuing a multi-group backend implementation of configurable vault layers + noise exclusion for Hatchdoor (Rust server over an Obsidian-style markdown vault). Groups A, B, C are done and reviewed. **Groups D and E remain.** Then a final live test.

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

- **Group D (MCP surface):** D1 `layers` param + per-vault enum + runtime instructions + `tools/list_changed`; D2 flip `listChanged`; D3 layer on responses, `get_note` path arg, expose `recently_modified`, `path_prefix` precedence error, write tools report layer, write tools refuse `.hatchdoor-layer`. Closes carried items 1–4. Add the note-filter+named-layer test Group C deferred.
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
