# Vault Layers — Remaining Backend Implementation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Make demoted layers and noise exclusion work end to end across the database, search, and the MCP surface — everything except the web frontend.

**Scope decision:** The web UI is explicitly **out of scope**. Filtering demoted notes out of the tree and search APIs server-side means the existing frontend stops showing them with no React changes. There is deliberately no reveal toggle and no layer badge; in the browser, demoted content is invisible with no in-browser way to reveal it. It stays reachable over MCP (via the `layers` selector) and by direct URL / `get_note`. This is accepted.

**Prior work:** Phase 1 (walk-level classification) is complete and merged into this branch. `VaultIndex::build` already populates `NoteEntry.layer: Option<String>` and `VaultIndex.layers: LayerMap`, prunes noise, and gives default-surface notes slug-allocation precedence. Nothing downstream reads the classification yet. This plan wires it through.

**Architecture:** The read path is SQLite, not the in-memory index. So the spine of this work is a `notes.layer` column plus a layer dimension on the vector and link tables, then a single `LayerSelection` concept threaded through every query and every MCP tool. Demoted-layer embeddings live in separate vec0 tables so default search keeps its fast unfiltered KNN path.

**Tech Stack:** Rust 1.96.0, rusqlite + sqlite-vec (vec0), the existing MCP JSON-RPC layer, `ignore` and the layer modules from phase 1.

**Spec:** `docs/superpowers/specs/2026-07-23-vault-layers-and-exclusions-design.md`. Read it. This plan implements it minus the "Web UI" section.

## Global Constraints

- Rust pinned to 1.96.0 in `rust-toolchain.toml`. Do not bump. `cargo fmt` before every commit; `cargo clippy --all-targets -- -D warnings` must stay clean.
- Fallible functions return `Result<T, String>` unless the surrounding code already uses `io::Result`. No `thiserror`, no `anyhow`.
- `LayerSelection` semantics are a **selector, not additive**: omitted ≡ default surface only; `["sources"]` ≡ that layer only; `["default","sources"]` ≡ both; `["all"]` ≡ everything.
- Demotion is not access control. `get_note` by slug or path reaches any layer with no flag. The one exception is `demo_mode`, which rejects layer parameters and 404s demoted paths.
- Reserved layer selector tokens: `default`, `all`. Layer names come from `LayerMap`, already validated in phase 1.
- No frontend files (`frontend/**`) are touched by this plan.

## Sequencing (why the order matters)

Three hard dependencies drive the task order; getting them wrong produces a feature that passes tests and silently does nothing:

1. **The `notes.layer` column and the marker-set hash must land together (Group A).** The column alone is inert: `upsert_note_if_changed` returns `Unchanged` on matching slug+mtime+hash, and adding a marker changes no note's content or mtime, so every note keeps `layer = NULL` and the feature no-ops. The marker-set hash is what forces the refresh.
2. **The vec0 restructure (Group C) must precede any search filtering.** Expressing "default surface only" as a `NoteFilters` predicate drops search onto the full-scan `semantic_search_filtered` path. Separate per-layer tables keep default search on the fast KNN path.
3. **The schema bump happens once, in Group A**, and forces a full re-embed for every existing vault on upgrade. Do not bump `SCHEMA_VERSION` again in later groups.

---

## Group A — Database foundation

### Task A1: `notes.layer` column and threading

**Files:** `src/cache/schema.rs`, `src/cache/populate.rs`, `src/cache/queries/*.rs` (read sites).

**Interfaces produced:** a `layer` column on the `notes` table (`TEXT NULL`, NULL = default surface); `SCHEMA_VERSION` bumped one step; `upsert_note` writes `NoteEntry.layer`.

- [ ] Bump `SCHEMA_VERSION` (`src/cache/schema.rs:9`) by one. Add `layer TEXT` to the `notes` table DDL (`src/cache/schema.rs:166`). Document in the commit that this forces a full re-embed on upgrade.
- [ ] Thread `layer` from `NoteEntry` into the `notes` insert/upsert in `src/cache/populate.rs`. The value is already on the entry from phase 1.
- [ ] Add a test: build an index over a vault with a `sources` marker, populate the cache, and assert the `notes` row for a demoted note has `layer = 'sources'` and a default-surface note has `layer IS NULL`.
- [ ] Verify: `cargo test`, then commit.

### Task A2: marker-set hash forces a refresh

**Files:** `src/cache/populate.rs`, `src/cache/schema.rs` (metadata table), `src/vault/layers.rs` (expose a stable hash input).

**Interfaces produced:** a `marker_set` hash stored in the cache `metadata` table; when it changes, the reindex forces a full note-row refresh instead of the incremental `Unchanged` short-circuit.

- [ ] Add a method on `LayerMap` that returns a deterministic hash **input** covering each marker's directory path *and its resolved declaration* (name + description), not just the directory keys — a renamed layer or changed description must change the hash. (The phase-1 `marker_paths()` returns only directories; this needs more.)
- [ ] Store the hash in `metadata` during populate. On the next populate, if the stored hash differs from the freshly computed one, bypass the `upsert_note_if_changed` `Unchanged` short-circuit for every note so `layer` is rewritten.
- [ ] Test the regression directly: populate a vault with no markers (notes get `layer NULL`), then add a `.hatchdoor-layer` marker without touching any note's content or mtime, populate again, and assert the affected notes now carry the layer. This is the test that would fail if the feature silently no-ops.
- [ ] Verify and commit.

### Task A3: refuse silent promotion on a vanished marker

**Files:** `src/cache/populate.rs` (or startup path in `src/server.rs`), `src/cache/schema.rs`.

**Interfaces produced:** the persisted marker set doubles as a guard; if a previously present marker is gone at reindex time, the reindex refuses to silently promote its notes and logs loudly with the count.

- [ ] Persist the resolved marker set (path → name) alongside the hash from A2.
- [ ] On reindex, if a marker present in the persisted set is absent from the freshly collected set, log at WARN naming the expected marker path and the number of notes that would move to the default surface, and (per spec) do not promote — retain the prior classification for those paths until the change is acknowledged by an explicit refresh. Keep this mechanism simple; a full ack-workflow is not required, but silent promotion is not acceptable.
- [ ] Test: persist a marker set, remove the marker file, reindex, assert the notes are not promoted and a warning is emitted.
- [ ] Verify and commit.

---

## Group B — Read-surface filtering and the wikilink fix

### Task B1: the `LayerSelection` type

**Files:** new `src/search/layer_selection.rs` or fold into `src/search/mod.rs`.

**Interfaces produced:** `LayerSelection` with selector semantics (default-only, named set, all), a parser from a `Vec<String>` of tokens that validates against the vault's known layer names, and a method producing a SQL predicate fragment / bound parameters for the `notes.layer` column.

- [ ] Define the type and its three states. Omitted/empty ≡ default only.
- [ ] Parsing: `default` and `all` are reserved tokens; any other token must match a known layer name (from `LayerMap`) or be an accepted-with-warning degrade (per spec: an unknown layer name is not a hard error, it degrades to the default surface with a warning, so a stale MCP client does not hard-fail).
- [ ] Unit-test each state and the unknown-name degrade.
- [ ] Verify and commit.

### Task B2: filter the tree, summaries, and recently-modified

**Files:** `src/cache/queries/metadata.rs`, `src/handlers/api.rs`.

**Interfaces produced:** `explorer_tree`, `note_summaries`, `note_rows_ordered`, and `recently_modified_notes` accept a `LayerSelection` and filter on `notes.layer`. Existing HTTP handlers pass the default selection (so the web UI sees default-surface only, no frontend change).

- [ ] Thread `LayerSelection` into each query. Default selection ⇒ `WHERE layer IS NULL`.
- [ ] The web HTTP handlers (`/api/tree`, etc.) always pass the default selection. Do not add a query parameter to the web routes here — the UI is out of scope, and `demo_mode` (Task E5) will depend on the web routes never accepting a selection.
- [ ] Test: a demoted note is absent from the default tree and summaries, present when the selection includes its layer.
- [ ] Verify and commit.

### Task B3: fix wikilink resolution

**Files:** `src/cache/queries/graph.rs`.

**Interfaces produced:** `resolve_wikilink` prefers default-surface notes over demoted ones on an ambiguous title/path match.

- [ ] In both queries (`src/cache/queries/graph.rs:79` and `:102`), change `ORDER BY relative_path` to order by layer first — default surface (`layer IS NULL`) before demoted — then `relative_path`. This is the fix for `[[Melatonin]]` resolving to the clipping.
- [ ] Test through the cache (not `VaultIndex::resolve_wikilink`, which is not the read path): populate a vault with `wiki/Melatonin.md` and `sources/Melatonin.md` (demoted), resolve `Melatonin`, assert it returns the `wiki/` slug. This is the test phase 1 lacked.
- [ ] Verify and commit.

### Task B4: layer on link/edge rows, stats, and graph

**Files:** `src/cache/queries/graph.rs`, `src/cache/queries/metadata.rs`, `src/cache/schema.rs`.

**Interfaces produced:** link/edge rows carry the source and target layer; `VaultStats` and `GraphResponse` carry layer; `get_note_links` accepts a `LayerSelection`; orphan status is computed per selection.

- [ ] Add layer to the edge/link representation and to `VaultStats` / `GraphResponse`. This is the "add the column" decision already approved.
- [ ] Forward links always resolve across the boundary and carry the target layer; backlinks from a demoted layer into the default surface are hidden under the default selection and included when the selection names that layer.
- [ ] Orphan counts computed per selection rather than globally.
- [ ] Tests for: an edge carrying layer; a forward link resolving into a demoted note; a demoted backlink hidden by default.
- [ ] Verify and commit.

---

## Group C — Search

### Task C1: per-layer vector tables

**Files:** `src/cache/schema.rs`, `src/cache/populate.rs`, `src/cache/queries/search.rs`.

**Interfaces produced:** demoted-layer chunk vectors live in separate vec0 tables (one per layer, or one auxiliary table partitioned by layer — implementer's call, justify in the report). Default semantic search runs an unfiltered KNN against the default table only and keeps its current fast path (`src/cache/queries/search.rs:191`). A layer search runs KNN against that layer's vectors. No query uses the `semantic_search_filtered` full-scan path for layer separation.

- [ ] Design and create the table structure. Confirm against the sqlite-vec / vec0 API that the chosen shape supports the fast MATCH path per table.
- [ ] Route chunk-vector writes by the note's layer during populate.
- [ ] `search` takes a `LayerSelection`; default ⇒ default table only; named ⇒ union of that layer's table(s); `all` ⇒ all tables.
- [ ] Test: a demoted note's content does not appear in a default search, does appear when its layer is selected; and a focused check that the default path is the unfiltered KNN, not the filtered scan.
- [ ] FTS/keyword search must apply the same selection.
- [ ] Verify and commit.

### Task C2: `HATCHDOOR_EMBED_LAYERS`

**Files:** `src/cache/populate.rs`, `src/config.rs` (or wherever C1's build reads config), `src/cache/schema.rs`.

**Interfaces produced:** `HATCHDOOR_EMBED_LAYERS=false` skips building the per-layer vector tables, degrading demoted layers to keyword search only. The flag participates in the embedding cache key so flipping it back to `true` actually re-embeds.

- [ ] Read the flag (default `true`). When false, do not build layer vector tables; keyword search over demoted layers still works.
- [ ] Include the flag in the cache-key / reset logic (alongside `reset_if_embedder_changed`, `src/cache/schema.rs:55`) so a `false`→`true` flip triggers the embed rather than leaving layers permanently unembedded.
- [ ] Test: with the flag false, a layer semantic search returns nothing while a layer keyword search returns the note.
- [ ] Verify and commit.

---

## Group D — MCP surface

### Task D1: the `layers` selector on read tools

**Files:** `src/mcp/tools/read.rs`, `src/mcp/tools/mod.rs`, `src/mcp/config.rs`, `src/mcp/routes.rs`.

**Interfaces produced:** `search_notes`, `query_notes`, `get_tree`, `get_note_links`, and `recently_modified` (newly exposed, see D3) accept a `layers` array parameter; its enum is generated per-vault from the discovered layer names plus `default` and `all`; each layer's marker `description` becomes that enum value's doc text (already sanitized in phase 1). Zero-layer vaults omit the parameter entirely.

- [ ] Thread `AppState` into tool-list construction (`read_tools_list` is currently a compile-time literal taking no arguments, `src/mcp/tools/read.rs:245`).
- [ ] Generate the enum and per-value docs from `LayerMap`. Omit the parameter and the instructions line for a zero-layer vault.
- [ ] Render a runtime line into the server instructions naming the vault's layers (currently a `const`, `src/mcp/config.rs:28`).
- [ ] An unrecognized layer name degrades with a warning (B1), it does not hard-fail schema validation.
- [ ] Tests: enum generated from markers; zero-layer vault omits the param; a `layers` call filters correctly through to the query layer.
- [ ] Verify and commit.

### Task D2: `tools/list_changed`

**Files:** `src/mcp/routes.rs`, wherever the marker set change is detected.

**Interfaces produced:** the server advertises `tools.listChanged: true` (currently `false`, `src/mcp/routes.rs:112`) and emits `notifications/tools/list_changed` when the marker set changes at runtime.

- [ ] Flip the capability. Emit the notification when the marker-set hash (A2) changes on a reindex.
- [ ] Test the capability is advertised; assert the notification path fires on a marker-set change (unit-level is fine).
- [ ] Verify and commit.

### Task D3: layer on responses; path fetch; expose recently-modified; precedence error; write reporting; marker write-refusal

**Files:** `src/mcp/tools/read.rs`, `src/mcp/tools/write.rs`, `src/mcp/tools/mod.rs`, `src/vault/write/*`, `src/search/mod.rs`.

- [ ] Search hits, `get_note`, `query_notes`, and `get_note_links` responses carry `layer` (string or null).
- [ ] `get_note` gains a `path` argument (vault-relative) as an alternative to `slug`, so a demoted note is reachable by a stable address. (`get_note` currently takes only `slug`, `src/mcp/tools/read.rs:317`.)
- [ ] Expose `recently_modified` as an MCP tool (it exists only as an HTTP handler today, `src/cache/queries/metadata.rs:162`), with a `layers` parameter and mtime ordering — this is the agent's ingest-discovery path.
- [ ] `path_prefix` naming a path inside an unselected demoted layer returns an **error** naming the layer and the parameter to pass, never a silent empty result (`NoteFilters.path_prefix`, `src/search/mod.rs:39`).
- [ ] `create_note`, `move_note`, `move_rename_note`, `archive_note` responses carry the resulting `layer`.
- [ ] `create_note`, `import_attachment`, `move_attachment`, `rename_attachment` hard-refuse any path whose basename is `.hatchdoor-layer`. (This closes the spec's write-tool refusal, which had no owner.)
- [ ] Tests for each bullet.
- [ ] Verify and commit.

---

## Group E — Config, ops, attachments, demo mode, diagnostics

### Task E1: `HATCHDOOR_EXCLUDE` wiring

**Files:** `src/config.rs`, `src/server.rs`, `src/vault/types.rs` (`VaultScanConfig`).

**Interfaces produced:** `HATCHDOOR_EXCLUDE` (comma-separated gitignore patterns) is read into `AppConfig`, passed into the `VaultScanConfig` used by every `build_with_config` call on the real server path, and appended to the built-in defaults. Startup logs the effective pattern list with provenance.

- [ ] Read the env var, build the `ExcludeMatcher` from defaults + user patterns (the matcher already supports this from phase 1), thread the `VaultScanConfig` into the server's index builds (startup, watcher, writes).
- [ ] Log the effective pattern list with provenance (built-in vs `HATCHDOOR_EXCLUDE`) at startup, using `configured_patterns()`.
- [ ] Also thread the config into `seed_empty_vault` so the seeder uses the same excludes as the index (phase-1 review flagged the signature divergence).
- [ ] Tests: a user pattern excludes a note on the real build path; the seeder honours it.
- [ ] Verify and commit.

### Task E2: watcher noise filtering and marker-triggered full reindex

**Files:** `src/vault_watcher.rs`.

- [ ] Apply the noise matcher in `should_refresh_for_event` (`src/vault_watcher.rs:87`) so `.obsidian/workspace.json` churn no longer triggers a reindex.
- [ ] A `.hatchdoor-layer` create/modify/delete forces a full reindex (which, via A2's hash, actually re-classifies).
- [ ] Tests: a noise event does not trigger refresh; a marker event does.
- [ ] Verify and commit.

### Task E3: startup recovery

**Files:** `src/server.rs`.

**Interfaces produced:** a failed index build at startup no longer leaves the server permanently stuck. The vault watcher is spawned even on the failure path, and a subsequent successful reindex clears the failed startup state.

- [ ] In the `Ok(Err(_))` arm (`src/server.rs:413`), spawn the vault watcher so a corrected marker triggers a recovering reindex. On a successful recovery reindex, transition startup out of `failed`.
- [ ] Decide git-sync's fate on this path explicitly (spawn on recovery, or document that it requires a restart) and state it in the commit.
- [ ] Test the recovery transition at whatever level is feasible (a unit test on the state machine if a full startup test is impractical).
- [ ] Verify and commit.

### Task E4: attachments, assets, and `archive_prefix`

**Files:** `src/handlers/assets.rs`, `src/vault/write/attachments.rs`, `src/vault/write/notes.rs`, `src/handlers/api.rs`.

- [ ] An asset's layer is its containing folder's layer. `list_note_attachments` does not filter by layer. Attachment fetch by path is unrestricted (like `get_note` by path) and reports its layer.
- [ ] Noise patterns do **not** gate `/vault-assets/` serving (a user glob must not silently break an embedded image). A write whose target path matches a noise pattern is an error.
- [ ] `archive_note` moving a note to the archive prefix promotes it to the default surface; the response reports the resulting layer and logs a warning. Layer filtering applies before the `archived` flag.
- [ ] Tests for asset layer reporting, noise-not-gating-serving, write-to-noise error, and the archive promotion signal.
- [ ] Verify and commit.

### Task E5: `demo_mode` server-side rejection

**Files:** `src/handlers/api.rs`, `src/server.rs`, `src/handlers/downloads.rs`.

**Interfaces produced:** under `demo_mode`, the server rejects any layer-selecting parameter on every read route and 404s demoted paths (including `get_note` by path and note downloads). Demotion becomes exclusion in this one mode.

- [ ] Reject layer parameters on read routes under `demo_mode`. Since the web routes never accept a selection (B2), this is mostly enforced by construction; add the explicit guard for any route that could receive one, and 404 demoted slugs/paths on note fetch and download.
- [ ] MCP is already rejected under `demo_mode` (`src/server.rs:64`), so no agent path is involved — note this in the commit.
- [ ] Tests: a demoted path 404s under demo_mode; a layer parameter is rejected.
- [ ] Verify and commit.

### Task E6: diagnostics surface

**Files:** new `src/handlers/diagnostics.rs` + route, `src/mcp/tools/read.rs` (MCP tool), disabled under `demo_mode`.

**Interfaces produced:** one surface, HTTP route + MCP tool, behind the same auth as other reads and disabled under `demo_mode`, with three outputs: (1) classify an arbitrary path string by re-running the matchers whether or not it is indexed; (2) dump the active ruleset with provenance (noise patterns built-in vs env, discovered markers with paths); (3) per-layer note counts and any conflicts (a layer whose directory is noise-excluded, a vanished marker, disagreeing descriptions).

- [ ] Implement the three outputs. Output (1) must re-run the noise/layer matchers on the raw string, not look the path up in the index (a noise-excluded path is absent from the index).
- [ ] Disable the surface entirely under `demo_mode` (it reveals demoted paths).
- [ ] Tests for each output and the demo_mode disablement.
- [ ] Verify and commit.

---

## Exit criteria

- `cargo test` and `cargo clippy --all-targets -- -D warnings` clean.
- Live check on a running server: a demoted folder is absent from the default tree and default search; its notes are reachable via `get_note` and via an MCP `layers` call; `[[Name]]` resolves to the default-surface note on a title collision; a noise-`.obsidian/` markdown file is unindexed; a `.hatchdoor-layer` typo no longer wedges the server permanently.
- Release notes: the `SCHEMA_VERSION` bump forces a one-time full re-embed for every existing vault on upgrade, and the built-in noise defaults drop previously-indexed `.md` under `.trash/`, `*.sync-conflict-*`, and `.obsidian/`.

## Explicitly not in this plan

- **The web frontend.** No reveal toggle, no layer badges. Demoted content is invisible in the browser (reachable via MCP / direct URL).
- **Lint logic** (orphan reports, contradiction detection). The data model lands here (B4); the checks do not.
- **Per-surface demotion** (`hide_from`). Demotion hides from all default surfaces together.
- **File-level frontmatter demotion.** Removed; folder markers only.
