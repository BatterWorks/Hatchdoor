# Vault Layers and Noise Exclusions — Design

Issue: [#22 — configurable vault file and directory exclusions](https://github.com/BattermanZ/Hatchdoor/issues/22)
Date: 2026-07-23
Status: Approved, pending implementation plan

## Problem

Hatchdoor treats every `.md` file in the vault identically. `VaultIndex::build`
(`src/vault/index.rs:27`) walks the whole tree with a single hardcoded skip for
`.hatchdoor-trash`, and every downstream surface — explorer tree, search,
embeddings, link graph, MCP tools — is fed from that one index.

That breaks down for vaults where some content is deliberately secondary. The
motivating case is the LLM-wiki pattern (Karpathy), where a vault holds two
kinds of content with opposite reading disciplines:

- a **compiled layer** of agent-written, interlinked pages — dense, few, the
  thing a human browses and an agent should answer from;
- a **source layer** of raw clippings, papers and transcripts — verbose,
  numerous, immutable ground truth.

With no distinction between them, four things go wrong:

1. **Search drowns.** Forty synthesized pages compete against fifteen hundred
   verbose clippings; the compiled layer loses on nearly every query. This
   defeats the premise of compiling in the first place.
2. **Browsing is unusable.** The explorer shows a wall of source material
   instead of the pages meant to be read.
3. **The link graph is polluted.** Backlinks and orphan detection get swamped by
   source-layer links.
4. **Indexing cost is misallocated.** The source layer is most of the bytes, so
   it dominates embedding time and cache size for content nobody browses.

Separately and more simply, vaults contain files that are not content at all —
`.obsidian/`, `.trash/`, `.DS_Store`, `*.tmp`, sync-conflict files. These are
indexed today and surface in results.

## Why secondary content cannot simply be excluded

The issue as filed asks for excluded paths to be "skipped during vault discovery
and indexing". Taken literally that would make the source layer unreachable — no
slug, no chunk, no way for any tool to fetch it.

That is the wrong outcome, because compilation is lossy and one-directional. The
source layer stays load-bearing after ingest:

- **Contradiction resolution** — when a new source contradicts an existing page,
  the page cannot adjudicate it; the summary already discarded the detail that
  would settle it.
- **Citation verification** — a citation nobody can follow is decoration.
- **Re-extraction along a new axis** — a summary is written to answer the
  questions asked at ingest time. A later question along a dimension nobody
  extracted ("what sample sizes did these studies use?") is answerable only from
  the sources.
- **Recompilation** — when page conventions change, pages are regenerated from
  sources; compiling a summary of a summary compounds the loss.
- **Deferred image reads** — an agent reads a document's text first and views
  its referenced images afterward, a designed second visit to source assets.

Sources are "immutable ground truth" only if they are consulted again. This
drives the central distinction of the design: **secondary content is demoted,
not excluded.** Noise, which is never read by anyone for any purpose, is
genuinely excluded.

## Model

A vault has:

- a **default surface** — every path with no layer marker above it;
- **N named demoted layers** — declared by marker files in the vault;
- **noise** — declared by glob patterns in deployment config.

**Demoted** means: absent from default search results, from the explorer tree,
and from default link-graph and stats aggregates; reachable by explicit request,
always. Demotion removes content from defaults, never from reach.

**Noise** means: never walked, never indexed, never reachable by any surface.

Layers are user-named. Nothing about layer names is baked into the server.

### Demotion is a single boolean in v1

A demoted layer is hidden from all three surfaces (search, tree, graph/stats)
together. Per-surface control — a layer that is hidden from search but visible
in the tree, e.g. an `inbox` — is a plausible future need and is deliberately
deferred. It is forward-compatible: a later `hide_from: [search, tree, graph]`
key defaults to all three, so every marker written against this spec keeps
behaving identically, and unknown keys are already ignored.

Because v1 ships only the source-layer profile, the spec does not claim to serve
`inbox`- or `journal`-shaped layers, and does not use them as examples.

### Demotion is not access control

Demoted content is reachable by any client that asks for it: `get_note` by path
or slug takes no flag, and the web UI has a reveal toggle. Demotion changes
defaults and ranking; it is not a permission boundary and must never be
documented as one. The single exception is `demo_mode`, specified below.

### Two mechanisms, deliberately

Layer classification lives **in the vault**; noise lives **in deployment
config**. This follows the grain of what each describes:

- A layer is a property of a *place*. Folders get renamed, moved and nested, and
  the classification must survive that. A glob (`raw/**`) breaks silently on
  rename — no error, no warning, and previously-demoted content is on the
  default surface again. A marker file rides along with the rename.
- Noise is a property of *file kinds*. `*.tmp` and `.DS_Store` appear anywhere
  and belong to no folder; there is no folder to put a marker in. Globs are the
  only mechanism that can express this.

Secondary consequences, both wanted:

- Layer classification travels with the vault, so the same vault is classified
  identically across deployments without replicating env vars — subject to the
  noise/marker interaction below.
- Noise rules live outside the vault's blast radius, so nothing that happens
  inside the vault can silently re-expose them. Noise fails closed.
- `.obsidian/` is handled by a built-in default pattern, so Hatchdoor never
  writes into a folder Obsidian manages.

The cost is that "why isn't this note showing up?" has two candidate causes in
two places, one of them on another machine. The diagnostic surface below is the
mitigation.

## Marker file — `.hatchdoor-layer`

Placed in a folder; applies to that folder and everything beneath it. Resolution
walks up the **logical walked path** from the file; the nearest marker wins. No
marker anywhere up the chain means the default surface.

Full form:

```yaml
name: sources
description: >
  Immutable clippings, papers and transcripts. Ground truth — the wiki
  compiles from these. Never edit by hand. Search here to verify a
  citation, re-extract a detail the wiki didn't capture, or find material
  not yet written up.
```

Bare form — the whole file is one word:

```
sources
```

A bare word is itself a valid YAML scalar document, so a single parser handles
both forms (untagged enum: scalar string, or mapping with `name` and optional
`description`). Unknown keys are ignored so the format can grow.

### Names

`name` is normalized: NFKC, trim, lowercase, spaces to `-`. The result must be
alphanumeric characters plus `-`, starting with an alphanumeric, at most 32
**characters** (not bytes); anything else is a startup error.

Names are Unicode, not ASCII — `sources-privées` and `資料` are as valid as
`sources`, because a vault is not required to be English. The alphanumeric
whitelist is what makes that safe: layer names reach an MCP tool schema that
agents read and a URL query parameter, so the characters that matter are the
ones that can make two names *look* identical, and none of those are
alphanumeric. Zero-width spaces and joiners, bidirectional overrides,
control characters, punctuation and emoji are all refused by the whitelist
without needing rules of their own.

NFKC additionally folds compatibility variants (full-width `ＳＯＵＲＣＥＳ` becomes
`sources`) and collapses composed and decomposed spellings of the same accented
name into one layer rather than two visually identical ones.

What remains is homoglyph confusion across scripts — Cyrillic `а` against Latin
`a`. Catching that needs a UTS #39 mixed-script check and the dependency to go
with it, and is deliberately deferred: the hostile path (an agent planting a
marker) is closed separately by write tools refusing to write
`.hatchdoor-layer`, which leaves only a single-user vault owner confusing
themselves. Adding the check later is non-breaking in the safe direction only —
it would narrow what is accepted — so it needs a deliberate decision rather
than a drive-by.

Reserved names, rejected as marker names with a startup error: `default`,
`all`, `noise`, `none`. `noise` is never expressible in-vault — that would
contradict the two-mechanism split.

Multiple folders may declare the same `name`; they form one queryable layer.
When two markers for one layer carry different non-empty descriptions, the
lexicographically-first marker path wins and startup logs a warning. (Without a
deterministic rule the generated tool schema would vary with filesystem walk
order between restarts.)

### Descriptions are untrusted input

`description` is free text written into the vault, and it is rendered into the
MCP tool schema and server instructions that every agent reads — a
prompt-injection channel from vault content into the tool contract, in a server
whose own instructions already declare note content untrusted. Before rendering:
strip control characters, collapse newlines, cap at 500 characters.

### File-level override via frontmatter

Markers are folder-scoped, which cannot express individual files that are
content but not a browsing surface — `log.md`, `index.md`, `README.md`, a
`TODO.md`. Moving such a file into a demoted folder changes its path and breaks
its wikilinks.

A note may therefore carry frontmatter:

```yaml
hatchdoor:
  layer: sources
```

Frontmatter is already parsed, is Obsidian-native, and travels with the file.
File-level declaration overrides any inherited folder marker. The same name
rules and reserved names apply; `layer: default` re-includes a single file.

### Write tools must refuse to write markers

`create_note`, `import_attachment`, `move_attachment` and `rename_attachment`
hard-refuse any path whose basename is `.hatchdoor-layer`. Without this, an
agent can write a marker — silently reclassifying a subtree, or (given the
malformed-marker rules below) breaking the next reindex.

### Malformed markers

- **At startup:** the index build fails. Note carefully what that does *not*
  mean: the process does not abort. `src/server.rs:413` catches the failure,
  marks startup failed, logs, and **skips spawning both the vault watcher and
  git sync**, which live only in the success arm. The server then serves the
  previous SQLite cache indefinitely, with no watcher — so correcting the marker
  on disk has no effect and recovery requires a restart. Git sync never starts
  either, so a vault that would have self-healed by pulling a corrected marker
  cannot.

  That is a worse failure than "loud", and phase 6 owns fixing it: spawn the
  watcher in the failure arm so a corrected marker triggers a recovering
  reindex, and have a successful recovery clear the failed startup state.

  Also worth reconciling in phase 6: this codebase's established convention for
  malformed vault-authored YAML is warn-and-degrade
  (`src/cache/populate.rs:896` logs "Ignoring malformed YAML frontmatter" and
  continues). Markers hard-fail on the same class of input. Two philosophies for
  the same category of user-authored file is a seam worth closing deliberately
  rather than by accident.
- **At runtime:** `VaultIndex::build` also runs on every write and on watcher
  refresh (`src/handlers/write_api.rs:584`, `src/mcp/tools/write.rs:376`,
  `src/app_state.rs:160`), where there is no startup to abort. A marker that
  becomes malformed while running (git pull, sync) **rejects that reindex and
  retains the last-good classification**, logging loudly and surfacing in
  diagnostics. Writes are not blocked; the stale classification is strictly
  safer than promoting content.

### Vault root

A marker at the vault root with a name other than `default` is a startup error.
Otherwise everything is demoted, the default surface is empty, the UI is blank,
and under `demo_mode` — where the reveal toggle is suppressed — the deployment
shows nothing with no way to see anything.

### The `default` escape hatch

`default` re-includes a subtree onto the default surface, overriding an
inherited layer. Its semantics:

- `default` is **not a layer**. It never appears in the generated `layers` enum,
  and paths under it report `layer: null` on every surface.
- Re-included paths are **not reachable via any `layers` selection** — they are
  on the default surface, which is selected by omitting `layers` (or by the
  `default` selector token). A `default` re-include inside `sources/` therefore
  means `layers: ["sources"]` returns a subtree with a hole in it; diagnostics
  must make this visible.
- `default` on a folder already on the default surface is a no-op with a startup
  warning.
- The reserved *selector token* `default` (used in the `layers` parameter) and a
  `default`-marked folder are different things and never collide, because
  `default`-marked folders are not layers.

### Symlinks

Directory symlinks are not followed. Layer resolution uses the logical walked
path. Without this, a symlink on the default surface pointing into a demoted
folder silently places demoted content on the default surface, and a directory
symlink out of a demoted folder double-indexes its target under two
classifications.

### Why not enforce read-only

Considered and rejected: a `read_only: true` field making write tools refuse a
layer. Immutability of a source layer is an agent convention, communicated
through the agent's skill/schema file exactly as it is when Hatchdoor is not
involved. The marker's `description` states the rule; nothing enforces it.

Noted honestly: **Hatchdoor itself writes into demoted layers.**
`backlink_rewrite_plan` (`src/vault/write/rewrites.rs:11`) rewrites wikilinks in
every note, including demoted ones, on rename/move/trash. This is existing
behaviour, is required for link integrity, and is not changed here — but the
spec should not claim a layer is untouched when the server edits it.

## Noise — `HATCHDOOR_EXCLUDE`

Comma-separated **gitignore-syntax** patterns, matched against the vault-relative
path via the `ignore` crate. A leading `/` anchors to the vault root; `!`
negates. Patterns append to a built-in default set:

```
.obsidian/    .trash/    .hatchdoor-trash/
.DS_Store     *.tmp      *.sync-conflict-*
```

Negation replaces a global opt-out flag: a deployment that needs a built-in
default gone writes `!.DS_Store` rather than discarding the whole set.

Rules:

- Noise is evaluated **at the walk, before layer resolution**. Noise inside a
  demoted layer is still noise.
- `.hatchdoor-layer` is immune to every pattern. Additionally, **the walk
  descends far enough into a pruned directory to collect its marker** — file
  level immunity alone is vacuous if the containing directory is pruned first.
  When a noise pattern would exclude a directory that declares a layer,
  diagnostics report the conflict explicitly, because per-deployment noise
  silently deleting a layer would contradict the portability claim above.
- The hardcoded `.hatchdoor-trash` filters at `src/vault/index.rs:29` and
  `src/vault/seed.rs:70` fold into the default set. `has_markdown_notes` must
  not count noise or demoted files when deciding to seed the starter vault.
- Noise is applied in `should_refresh_for_event` (`src/vault_watcher.rs:87`).
  Today `.obsidian/workspace.json` churn triggers a full reindex and re-embed;
  without this the noise feature does not deliver its headline win.
- A `.hatchdoor-layer` create/modify/delete forces a **full** reindex, not an
  incremental one (see below).
- Git sync stages `add_all(["*"])` (`src/git/sync.rs:334`) and therefore commits
  noise files. Out of scope to change; stated so it is a known behaviour rather
  than a surprise.
- **Upgrade impact, requires a release note.** Three of the six built-in
  patterns can match `.md` files that existing vaults currently index:
  `.trash/` (Obsidian's locally deleted notes), `*.sync-conflict-*` (Syncthing
  conflict files, which are markdown), and `.obsidian/` (plugin documentation
  and some template setups). Notes under those paths leave the index on
  upgrade, disappearing from search and from wikilink resolution. This is
  intended, and a deployment that wants one back can negate it
  (`HATCHDOOR_EXCLUDE=!*.sync-conflict-*`), but it must not ship silently.
- Startup logs the full effective pattern list with provenance (built-in vs
  `HATCHDOOR_EXCLUDE`). No shell-expansion heuristic: `HATCHDOOR_EXCLUDE=*.tmp`
  expands only when the working directory happens to contain matching files, so
  detection would be a coin flip. The effective-pattern log makes the problem
  visible in every case instead.

## Indexing and storage

### Notes table

`VaultIndex::build` resolves layers during the walk and carries
`layer: Option<String>` on `NoteEntry`, threaded through `Note`, `NoteSummary`,
`SearchHit` and the explorer types.

The read path is SQLite, not `VaultIndex` — `VaultIndex::explorer_tree`
(`src/vault/index.rs:160`) is `#[cfg(test)]`-only. So the real change is a
`notes.layer` column (`src/cache/schema.rs:166`) plus layer filtering in every
query in `src/cache/queries/`. This bumps `SCHEMA_VERSION`
(`src/cache/schema.rs:9`), which **forces a full re-embed of every existing
vault on upgrade**. That migration cost must be stated in the release notes.

`VaultIndex::build` gains a configuration argument (noise patterns); ~45 call
sites across `src/vault/tests.rs`, `src/vault/write/tests.rs`,
`src/cache/populate.rs`, `src/eval/` and `src/bin/eval.rs` change accordingly.

### Marker changes must invalidate the incremental path

`upsert_note_if_changed` (`src/cache/populate.rs:783`) returns `Unchanged` when
slug, mtime and content hash all match. Adding a `.hatchdoor-layer` changes no
note's content or mtime, so without intervention 1,500 notes keep `layer = NULL`
and **stay on the default surface after the user demotes them**; deleting a
marker hides them forever.

A hash of the resolved marker set (path → name) is stored in `metadata`. When it
changes, the reindex forces a full note-row refresh. This is the mechanism that
makes the feature work at all.

### Persisted marker set guards against silent promotion

`.hatchdoor-layer` is a dotfile in a synced directory. `rsync --exclude='.*'`, a
`.gitignore` containing `.*`, a file-manager copy, or Obsidian's own sync
handling can drop it — and Obsidian never displays the file, so the loss is
invisible. The failure is not exotic; it is the modal one.

The same persisted marker set therefore doubles as a guard: if a previously
present marker is gone, the reindex **refuses to promote silently** and logs
loudly — `expected marker at sources/.hatchdoor-layer, not found; 1,514 notes
would move to the default surface`. Promotion proceeds only when the operator
acknowledges (an explicit refresh, or removal of the persisted entry). This
converts the modal failure from silent to loud.

### Embeddings live in per-layer vector tables

Unfiltered semantic search uses the vec0 KNN index
(`src/cache/queries/search.rs:191`). As soon as any filter is present, search
falls back to `semantic_search_filtered` (`:218`), which reads every
`chunk_vectors` row and scores in Rust. Expressing "default surface only" as a
`NoteFilters` predicate would therefore make **every default query a full scan
across exactly the demoted vectors the feature exists to avoid** — inverting the
design's primary goal.

Instead, demoted-layer vectors live in **separate vec0 tables, one per layer**.
Default search is an unfiltered KNN against the default table and keeps its
current fast path untouched. A layer search is an unfiltered KNN against that
layer's table. No filtered fallback is involved on either path.

This also makes the embedding opt-out trivial: `HATCHDOOR_EMBED_LAYERS=false`
simply does not build the layer tables, degrading demoted layers to keyword
search rather than to nothing. The flag participates in the embedding cache key
(alongside `reset_if_embedder_changed`, `src/cache/schema.rs:55`) — otherwise
flipping it back to `true` re-embeds nothing, because chunk work is gated on
content-hash change, and layers stay permanently unembedded with no error.

Per-layer embedding control (`embed only these layers`) is deferred; the flag is
global in v1.

### Startup

Indexing remains a single pass; readiness continues to wait for the whole vault.
Phased startup is out of scope and will be revisited if a large demoted layer
makes startup latency hurt in practice.

Known and unchanged: every MCP write triggers `refresh_now` → `VaultIndex::build`
→ a content hash of every note (`src/cache/populate.rs:777`), so a 15-page
ingest costs roughly 17 writes × full-vault reads. Demotion does not reduce this.
Problem statement #4 above is delivered for *embedding* cost, not for per-write
walk cost. Out of scope; stated so it is not mistaken for solved.

## Addressing: slugs, paths and precedence

This is the design's sharpest interaction with existing behaviour.

Slugs derive from the file stem, uniquified with a `-2` suffix in lexicographic
path order, and `by_title` is first-write-wins (`src/vault/index.rs:42,53-73`).
In the motivating layout `sources/` sorts before `wiki/`, so:

- `sources/Melatonin.md` → slug `melatonin`
- `wiki/Melatonin.md` → slug `melatonin-2`
- `resolve_wikilink("Melatonin")` → **the source note**

Every `[[Melatonin]]` written into a compiled page resolves to the clipping, for
the agent and for the human clicking through in the UI. This is not an edge
case: a compiled page named after the source it compiles is what a compiled
layer *is*. It is also unstable — adding a third colliding file shifts the
suffixes, changing slugs agents hold and URLs humans bookmarked.

Two changes:

1. **Default surface wins.** Slug allocation and `by_title` / `by_path_title`
   resolution prefer default-surface notes over demoted ones on any ambiguous
   match. A demoted note takes the suffixed slug.

   **This has two halves, and phase 1 delivers only the first.** Verified
   against a running server: slug *allocation* is correct (the compiled page
   takes `melatonin`, the clipping takes `melatonin-2`), but live wikilink
   *resolution* still returns the clipping. The reason is that the UI and MCP
   resolve through SQL, not through the in-memory index:
   `src/cache/queries/graph.rs:79` and `:102` both end
   `ORDER BY relative_path LIMIT 1`, and `sources/Melatonin` sorts before
   `wiki/Melatonin`.

   `VaultIndex::resolve_wikilink` (`src/vault/index.rs:151`) does honour the
   precedence, but its only production caller is backlink rewriting on rename;
   the read path never touches it. **A unit test asserting
   `index.resolve_wikilink` therefore proves nothing about live behaviour** —
   that is exactly the false confidence that let this through phase 1.

   Phase 2 owns the second half: once `notes.layer` exists, both queries must
   order by layer before `relative_path`. Until then the motivating bug
   (`[[Melatonin]]` opening the clipping) is still live, and the fix is
   structurally blocked on the column.
2. **`get_note` gains a `path` argument** (`src/mcp/tools/read.rs:317`,
   `SlugArgs` at `mod.rs:116`), accepting a vault-relative path as an
   alternative to `slug`. The earlier draft of this design asserted that
   `get_note` by path needed no flag; no path-addressed fetch existed. Since
   "reachable, always" is the core promise of demotion, and slugs are the thing
   layers make collision-prone, the promise needs a stable addressing mode.

## MCP surface

Because layer names are chosen per vault, nothing about them can be compiled in.
This is why a parameter beats dedicated tools: a parameter's JSON schema can be
generated per vault, a fixed tool set cannot.

### The `layers` parameter is a selector, not an additive flag

On `search_notes`, `query_notes`, `get_tree`, `get_note_links` and
`recently_modified`:

- omitted ≡ `["default"]` — the default surface only;
- `["sources"]` — that layer **only**;
- `["default", "sources"]` — both;
- `["all"]` — everything.

An additive reading ("demoted layers *in addition to* the default surface") was
rejected: it makes "sources only" inexpressible, which is the query ingest
actually issues, and it re-creates the drowning problem in every result list
that includes a demoted layer. Selector semantics also remove any need for
per-layer ranking weights.

Enum values are generated from the markers discovered in this vault, plus the
tokens `default` and `all`:

```jsonc
"layers": {
  "type": "array",
  "items": { "enum": ["default", "sources", "all"] },
  "description": "Which layers to search. Omit for the default surface only.\n  • default — the default surface\n  • sources — Immutable clippings, papers and transcripts. Ground truth; the wiki compiles from these. Never edit by hand. Search here to verify a citation, re-extract a detail the wiki didn't capture, or find material not yet written up.\n  • all — every layer."
}
```

**Zero-layer vaults omit the parameter entirely** and omit the rendered
instructions line, rather than exposing a parameter whose only value is a no-op.

### Schema changes must be announced

`read_tools_list()` returns compile-time literals and takes no arguments
(`src/mcp/tools/read.rs:245`), and the server advertises
`"tools": {"listChanged": false}` (`src/mcp/routes.rs:112`). Both change:
`AppState` is threaded into tool-list construction, `listChanged` becomes
`true`, and `notifications/tools/list_changed` is emitted when the marker set
changes.

A client holding a stale schema must degrade, not fail: **an unrecognized layer
name is accepted with a warning** and treated as the default surface, rather
than rejected by `"additionalProperties": false` validation. Removing a marker
must not turn connected clients' calls into hard errors.

### `path_prefix` precedence

`NoteFilters.path_prefix` already exists on both search tools
(`src/search/mod.rs:39`). Today `search_notes {"filters":{"path_prefix":
"sources"}}` works; after this change, with `layers` omitted, it would return
`[]` — reading as "no sources mention this". This is the most likely field
failure of the whole feature, and existing skill files already issue such calls.

Rule: a `path_prefix` naming a path inside a demoted layer, with that layer not
selected, is an **error naming the layer and the parameter to pass** — never a
silent empty result.

### Results and writes declare their layer

- Search hits, `get_note`, `query_notes` and `get_note_links` responses carry
  `layer: "sources"` or `layer: null`. An agent needs to know whether it holds
  compiled synthesis or ground truth before citing it.
- `create_note`, `move_note`, `move_rename_note` and `archive_note` responses
  carry the **resulting** layer. Moving a note across a layer boundary silently
  promotes or demotes it; write-side *enforcement* is out of scope, write-side
  *reporting* is one field.

### `recently_modified` is exposed over MCP

`recently_modified_notes` exists (`src/cache/queries/metadata.rs:162`) but is
wired only to an HTTP handler. It is not in the MCP tool set, so an agent's only
way to notice a new source arrived is `get_tree` plus inspection — which this
design removes, since `get_tree` without `layers` no longer shows the source
layer. Ingest begins with discovery; without this the design makes the first
step of the motivating workflow harder than the status quo.

Exposed with a `layers` parameter and mtime ordering.

## Link graph, stats and the lint data model

Lint *logic* (orphan reports, contradiction detection, stale-page checks) stays
out of scope. Its *data model* does not, because `orphan_notes` already ships
(`src/cache/queries/metadata.rs:396`) and this design would otherwise leave it
layer-blind while changing everything around it. Retrofitting later means a
cache migration plus a breaking `/api/graph` shape change; doing it now is one
join column.

- `layer` is threaded onto link/edge rows (`src/cache/queries/graph.rs`),
  `VaultStats` and `GraphResponse`.
- `get_note_links` gains the `layers` parameter.
- Orphan status is computed **per selection**, not globally.

Link semantics across the boundary, stated explicitly because the two directions
differ:

- **Forward links always resolve**, regardless of layer, and carry the target's
  `layer`. A citation from a compiled page into a source must work — "a citation
  nobody can follow is decoration" is a founding requirement of this design.
- **Backlinks from a demoted layer into the default surface are hidden by
  default** and included when `layers` selects that layer. These are precisely
  what would swamp a compiled page.

## Attachments and assets

Non-`.md` files never enter `VaultIndex` (`src/vault/index.rs:34`), so they need
explicit rules:

- An asset's layer is its containing folder's layer, resolved identically.
- `/vault-assets/{*path}` (`src/handlers/assets.rs:59`) resolves off disk with an
  extension allowlist and a root-containment check. It stays that way:
  **noise patterns do not gate asset serving.** A user glob like `*.tmp` or
  `*-old*` would otherwise silently break images already embedded in notes.
- `list_note_attachments` never filters by layer.
- Attachment fetch by path is unrestricted, like `get_note` by path, and reports
  its layer.
- A write (`import_attachment`, `move_attachment`, `rename_attachment`) whose
  target path matches a noise pattern is an error, not a silent success into a
  location nothing will ever index.

## `archive_prefix` interaction

`archive_prefix` (`90-archive/`) is an existing parallel demotion with its own
`archived` flag and UI treatment (`src/handlers/api.rs:117`,
`frontend/src/components/note-page/wikilinks.ts:72`). Folding it into the layer
model is out of scope. Stated interactions:

- A note may be both archived and demoted. Layer filtering applies first;
  `archived` remains an independent flag on the result.
- `archive_note` (`src/vault/write/notes.rs:413`) moves a note to `90-archive/`
  and therefore **promotes a demoted note onto the default surface**. The
  response reports the resulting layer and the server logs a warning. This is a
  known, documented interaction, not a silent one.
- `delete_note` moves a note to `.hatchdoor-trash/<original path>`
  (`src/vault/write/paths.rs:311`), which preserves the demoted path *inside* a
  noise folder. Trash is noise, so the note leaves every surface as intended;
  because the original path is preserved, restoring it re-resolves to the same
  layer. No special handling is required — stated so the round trip is a
  verified property rather than an accident.

## Web UI

- Demoted layers are hidden from the explorer tree, search results, graph and
  stats by default, with a reveal toggle.
- A note in a demoted layer opened directly still renders, badged with its layer
  name.
- **Wikilink autocomplete must keep demoted candidates.** Candidates derive from
  the tree (`frontend/src/hooks/useVaultTree.ts:118` →
  `components/note-page/autocomplete.ts:52`); hiding demoted layers from
  `/api/tree` would make every source note unlinkable from the editor — you
  could no longer type the citation this design exists to support. Autocomplete
  draws from a source that includes demoted layers regardless of the toggle,
  with the layer shown in the candidate row.
- **Citation links carry a layer signal.** `/api/resolve-batch`
  (`src/handlers/api.rs:117`) returns `archived` today and renders second-class
  links distinctly; `layer` joins it and renders likewise. The argument that
  layer-on-result is functional rather than cosmetic applies to the human
  reading a citation exactly as it does to the agent.
- `layer` is added to the frontend types that carry note identity:
  `ExplorerNote`, `SearchResult`, `Note`, `NoteLink`, `ModifiedNote`,
  `GraphNode`, and `ResolveBatchResponse.results`. `ExplorerNote` currently
  carries only `{title, slug}` and has no field to badge.
- Folder rows in `Explorer.tsx` are badged where the marker sits on the folder.

## demo_mode

`demo_mode` is enforced today only against writes and `/api/refresh`
(`src/handlers/write_api.rs::reject_demo_mode_write`, `src/handlers/api.rs:145`);
every read route is unauthenticated. Hiding a UI toggle is therefore not a
control — anyone can pass a layer parameter directly to `/api/tree`,
`/api/search`, `/api/graph`, `/api/stats`, `/api/recently-modified` or
`/api/note/{slug}`.

Under `demo_mode`:

- the server **rejects any layer-selecting parameter on every read route**;
- demoted paths return 404, including `get_note` by path and note downloads;
- the diagnostic surface is disabled entirely, since it necessarily reveals
  demoted paths.

`demo_mode` is the one place demotion becomes exclusion, and it is a deliberate
mode, not a property of demotion. Consequence to accept knowingly: on the public
demo every citation into a demoted layer is a dead link. MCP is already rejected
alongside `demo_mode` (`src/server.rs:64`), so no agent is affected.

## Exports

`build_note_export` (`src/handlers/downloads.rs:16`) reads by slug with no layer
check and bundles referenced assets from disk. Behaviour is unchanged — a
demoted note is downloadable by slug, like `get_note` — except under
`demo_mode`, where demoted slugs 404. Link-stripping already removes all
wikilinks and needs no change.

## Diagnostics

A route plus an MCP tool, behind the same auth as other read surfaces and
disabled under `demo_mode`. Three outputs, because forward classification alone
cannot answer the questions this exists for:

1. **Classify an arbitrary path string** by re-running the matchers, whether or
   not the path is in the index. A noise-excluded path is absent from the index
   entirely, so an index-driven lookup would answer "not found" instead of
   "excluded by pattern `*.tmp` (built-in)".
2. **Dump the active ruleset with provenance** — every noise pattern (built-in
   vs `HATCHDOOR_EXCLUDE`) and every discovered marker with its path. This is
   what makes the deployment-side half of the configuration visible from the
   vault side.
3. **Per-layer note counts**, plus any conflicts: a layer whose directory is
   excluded by a noise pattern, a marker that disappeared since the last index,
   descriptions that disagree, `default` re-includes producing holes.

```
sources/notes/x.md  → layer "sources"     (marker at sources/.hatchdoor-layer)
sources/.DS_Store   → noise               (default pattern .DS_Store)
wiki/index.md       → default surface
```

## Testing

- **Layer resolution:** nearest-marker-wins, inheritance, `default` override on
  a nested folder, two folders sharing one layer name, frontmatter override
  beating a folder marker, root marker rejected, symlinks not followed.
- **Marker parsing:** bare scalar, full mapping, unknown keys ignored, reserved
  names rejected, slug normalization (NFKC, case, spaces, over-length, empty),
  description sanitized and capped, malformed marker fails startup, malformed
  marker at runtime retains last-good classification.
- **Noise:** gitignore semantics including anchoring and `!` negation, defaults
  applied, `.hatchdoor-layer` survives a `.*` pattern, marker collected from a
  pruned directory and the conflict reported, noise beats layer, watcher ignores
  noise events, seed suppression unaffected.
- **Index and cache:** `notes.layer` populated; marker-set hash change forces a
  full refresh (the regression that would otherwise leave `layer = NULL` on
  every note); a removed marker refuses silent promotion; noise paths absent.
- **Search:** omitted `layers` returns the default surface only; `["sources"]`
  returns only that layer; `["all"]` returns everything; default search executes
  against the default vec0 table with no filtered fallback; `path_prefix` into
  an unselected layer errors rather than returning empty.
- **Addressing:** default surface wins the unsuffixed slug on a title collision;
  `get_note` by path reaches a demoted note.
- **MCP:** enum generated from discovered markers; zero-layer vault omits the
  parameter; unknown layer name degrades with a warning; `list_changed` emitted
  on marker change; write responses report the resulting layer.
- **Graph and stats:** edges carry layer; orphan status computed per selection;
  forward links resolve across the boundary while demoted backlinks are hidden
  by default.
- **Attachments:** asset serving unaffected by noise; write to a noise path
  errors; layer reported.
- **Trash round trip:** deleting a demoted note removes it from every surface;
  restoring it re-resolves to the same layer.
- **UI:** tree, search, graph and stats hide demoted layers; toggle reveals;
  direct navigation renders with a badge; autocomplete still offers demoted
  notes; citation links carry the layer signal.
- **demo_mode:** layer parameters rejected on every read route; demoted paths
  404; diagnostics disabled.

## Out of scope

- **Lint logic** — orphan reports, contradiction detection, stale-page checks.
  The data model lands here; the checks do not.
- **Per-surface demotion** (`hide_from`) — deferred, forward-compatible.
- **Per-layer embedding control** — the flag is global in v1.
- **Write-side enforcement** of layer immutability.
- **Phased startup** — serving the default surface before demoted layers finish
  embedding.
- **Per-write full-vault walk cost** — pre-existing, unchanged by this design.
- **Folding `archive_prefix` into the layer model** — interactions stated
  instead.
- **A vault-level conventions document** (the LLM-wiki pattern's third layer,
  `CLAUDE.md`-style). An agent connecting over MCP with no skill file installed
  has no way to learn a vault's conventions; a vault-level description riding
  the same rendered-instructions channel as layer descriptions would close it.
  Worth a follow-up issue.
- **Git sync scope** — noise files are still committed.
