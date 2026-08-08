# Hatchdoor Module Map

## Purpose

This map defines collaboration boundaries for humans and coding agents. It
describes the repository as it exists today; it does not imply that every
listed boundary should become a package, crate, or feature directory.

Use this map together with
[`domain-collaboration-plan.md`](domain-collaboration-plan.md). A work
packet narrows this catalog to one task and declares any exceptions before work
starts.

## Boundary vocabulary

- **Owned paths:** implementation a module owner may change freely within the
  task.
- **Public contract:** the supported names, serialized shapes, or behavior that
  collaborators should rely on outside the module. This is narrower than every
  symbol that happens to be technically `pub` in Rust; current visibility does
  not enforce every documented boundary.
- **Coordination paths:** shared or composition files that may change only when
  the work packet lists them.
- **Consumed dependencies:** modules this boundary may call but does not own.
- **Invariant:** behavior that must remain true, usually backed by an ADR.

“Owner” means the owner of a work packet, not a permanent person or team.
Shared and composition files have no default task owner.

## Change rules

1. Internal changes may stay inside owned paths when the public contract and
   invariants do not change.
2. Public-contract changes must be declared and list affected consumers.
3. Coordination files are not implicitly writable because a module imports
   them.
4. Adapter code must not absorb domain behavior merely to avoid coordinating
   with the domain.
5. A full-stack feature can span multiple boundaries, but its work packet must
   enumerate each boundary and integration point.
6. When this map and the code disagree, stop and update the map or the work
   packet before expanding the diff.

## When to update this map

Update this map in the same change when:

- a production file is added, moved, or deleted;
- a file's owner, boundary kind, or shared/composition status changes;
- a supported public contract or invariant changes;
- a cross-module consumer, dependency, or coordination path is added or
  removed;
- the focused validation for a boundary changes.

Do not update the map for an ordinary internal edit that preserves all of the
above. Structural coverage can be checked mechanically, but contract and
invariant accuracy still require review.

Run the structural check after adding, moving, deleting, or reclassifying
production source files:

```bash
node scripts/check-module-map.mjs
```

The production inventory includes Rust `*.rs` files except standalone
`tests.rs`, plus frontend `*.ts`, `*.tsx`, and `*.css` files except
`*.test.ts`, `*.test.tsx`, and `frontend/src/test/**`. Exact assignments outside
that production inventory are still checked for stale paths and duplicates.

## Backend

### Runtime composition

**Kind:** composition/shared.

**Owned paths:** none by default.

**Paths:**

- `src/lib.rs`
- `src/main.rs`
- `src/server.rs`
- `src/app_state.rs`
- `src/config.rs`
- `src/startup.rs`
- `src/vault_runtime.rs`
- `src/model_setup.rs`
- `src/vault_watcher.rs`

**Contract and responsibility:**

- `lib.rs` exposes the application modules to the main binary and auxiliary
  binaries.
- `main.rs` selects serve, model-prefetch, and container-healthcheck modes.
- `server.rs` is the HTTP composition root: it validates startup posture,
  constructs `AppState`, builds routes, and starts background work. Unsafe
  public startup without web authentication remains a refusal; its error
  includes a freshly generated, non-persisted recovery token for the operator
  to place in `.env`.
- `AppState` and `VaultCache` carry shared runtime state; `build_cache*`,
  `sqlite_cache`, `refresh_coalescing`, and `refresh_now` coordinate reindexing.
- `VaultCollectionRuntime` reconstructs disposable background turns at startup
  and, on process shutdown, stops new work and waits only for the active turn's
  safe boundary.
  `AppState::vault_registry`, `AppState::vaults`, and
  `AppState::legacy_migration_recovery` expose the authoritative definition
  store, activated per-Vault control blocks, and safe legacy-recovery state to
  later shared-core adapters. `AppState::vault_work` and
  `AppState::managed_git` expose the same background-work coordinator and
  managed-Git scheduler `run_server()` wires into the one dispatch loop, so an
  HTTP adapter (`handlers/vaults.rs`) can reconcile a registry mutation into
  live runtime effects and request an immediate Git turn without a second
  execution lane.
  `AppState::index_status` separately tracks setting-triggered rebuild drift,
  progress, ETA, and the last failure without changing startup readiness, while
  `AppState::runtime_config` supplies the immutable settings snapshot each
  reindex binds before it starts.
- `AppConfig` is the environment-derived deployment contract and interprets the
  live values from the startup `RuntimeConfig` snapshot.
- `StartupTracker` exposes startup/model/indexing readiness.
- `VaultRuntime` and its serialized snapshot expose the current configured
  source, mode, lifecycle phase, and derived capabilities. This is the rebased
  single-configured-Vault adapter retained for the still-unmigrated application
  surfaces; it is not the collection authority.
- `VaultCollectionRuntime` reconciles registry snapshots into zero, one, or
  many Vault-ID-keyed `VaultControlBlock` values. Each enabled block owns its
  definition and resolved Markdown root, capability-specific activation/local
  content/search/Git/watcher status and errors, mutation and refresh locks, and
  independently cancellable watcher. Status changes and `reconcile()` advance a
  revisioned collection snapshot and publish a `VaultCollectionRevisionEvent`
  (`collection_revision`, the affected Vault IDs, and a broad
  `VaultChangeCategory` of `definition` or `status`) over
  `subscribe_revisions()`; a subscriber that misses an intermediate advance
  still learns the current revision from the watch channel's latest value and
  should refetch broadly rather than trust `vault_ids` as a complete history.
  Disabling, replacing, or disconnecting a block first revokes operation
  acceptance, publishes its
  cancellation signal, and stops its watcher, including through already-held
  handles. Unchanged Vaults retain their control blocks when another definition
  changes; disabled definitions remain visible with no capabilities and no
  active runtime.
- `ModelSetup` owns local model selection, terms acceptance, download integrity,
  and persistent setup records.
- `spawn_vault_change_watcher` reports Vault-ID-qualified change intent through
  an independently cancellable handle. Queueing/coalescing those intents is
  deliberately deferred to #89. The existing `spawn_vault_watcher` remains the
  transitional single-Vault adapter until later application-surface packets.
- `dispatch_managed_git_turn` is the `VaultWorkKind::Git` execution closure
  `src/server.rs`'s worker loop calls: it resolves one managed-Git Vault's
  current definition and credentials, runs `git::run_managed_git_turn` off the
  async runtime via `spawn_blocking`, and publishes the result through
  `VaultControlBlock::set_git_status`/`set_local_content_status` and
  `ManagedGitScheduler::record_outcome`. `reconcile_and_reconstruct` activates
  or deactivates a managed-Git Vault's scheduler entry alongside its
  coordinator admission. `set_local_content_status` (mirroring
  `set_search_status`/`set_git_status`) republishes authoritative
  local-content availability after a Git turn, since `activation_snapshot`
  only stats `vault_path` once, at `reconcile()` time, before a managed
  checkout exists.

**Consumed dependencies:** nearly every backend boundary. This is expected for
a composition boundary and is not a reason to introduce per-domain service
traits. Collection activation consumes redacted registry definitions and their
store-resolved local Markdown roots and does not read credentials; managed-Git
Git-turn dispatch is the one exception, reading plaintext credentials only
through the registry's crate-private `https_credentials` accessor, for Git
authentication only.

**Coordination rule:** any work packet touching these files must name the
specific field, route, startup phase, or integration being changed. Adding an
`AppState` field requires identifying every constructing test fixture.

**Invariants:**

- One binary serves HTTP, MCP, and the SPA over one shared core (ADR-02).
- Unsafe public/auth and demo configurations fail at startup (ADR-07).
- Model inference remains local and CPU-capable (ADR-04).
- Cache refresh preserves the disposable-read-model contract (ADR-01/06).
- The runtime image cannot assume a shell (ADR-12).

**Validation:** `cargo test server`, `cargo test app_state`,
`cargo test config`, `cargo test startup`, `cargo test model_setup`,
`cargo test vault_runtime`, `cargo test vault_watcher`, followed by the full
backend checks.

### Background work coordination

**Kind:** infrastructure/runtime scheduling.

**Owned paths:** `src/vault_work.rs`.

**Public contract:** `VaultWorkCoordinator` is the cloneable request side and
`VaultWorkWorker` is the unique execution side of one instance-wide in-memory
FIFO. `VaultWorkKind`, `VaultWorkRequest`, `ScheduleResult`, `VaultWorkOutcome`,
and `VaultWorkError` expose deterministic one-operation turns, request
coalescing, lifecycle rejection, and Vault-qualified returned outcomes. Index
work includes local embedding work; Git and repair remain distinct operation
kinds. A stopped worker returns `None` rather than waiting for discarded work.

**Consumed dependencies:** durable `VaultId` identity and Tokio notification.
The queue owns no Markdown, SQLite, Git, or lifecycle state.

**Consumers:** collection runtime reconstructs and drains work for lifecycle
transitions. `handlers/vaults.rs` reaches the coordinator only indirectly,
through `VaultCollectionRuntime::reconcile_and_reconstruct` after a registry
mutation, and directly through `ManagedGitScheduler::sync_now`/`retry_now` for
manual Git control — it never calls `request`/`drain_vault` itself. Later
cache and Git packets supply concrete operations without creating additional
execution lanes.

**Coordination paths:** `src/lib.rs` for the module export; runtime composition,
per-Vault watcher intent, cache refresh, Git lifecycle, and repair producers
when their owning packets integrate the coordinator.

**Invariants:** one Vault occupies at most one FIFO position; one operation runs
per turn; duplicate pending work coalesces and duplicate active work retains at
most one rerun; remaining work returns to the tail; a returned failure completes
its turn and remains attributable to one Vault. The queue stays disposable and
adds no priorities, throttling, persistence, second lane, generic timeout, or
forced cancellation. Runtime lifecycle stops new work, discards queued work,
and waits only for an active turn's safe boundary; restart reconstruction uses
durable definitions and current local-content/Git status.

**Validation:** `cargo test vault_work`, the runtime-composition tests when a
consumer is integrated, and the full backend checks.

### Live configuration foundation

**Kind:** infrastructure/runtime state.

**Owned paths:** `src/runtime_config.rs`.

**Public contract:** `RuntimeConfig`, `ConfigSnapshot`, `ResolvedSetting`,
`SettingSource`, `Environment`, `SETTINGS_SCHEMA`, `live_settings_defaults`,
`settings_file_path`, `is_truthy`, and the versioned
`settings.json` file format. `RuntimeConfig::snapshot` gives one immutable,
lock-free configuration view to bind at the start of an operation;
`RuntimeConfig::save` serializes writes, persists first, then publishes the
new view. `RuntimeConfig::remove_stored` lets the one-time legacy migration
remove only settings already copied into the Vault registry, persisting before
publishing and leaving environment pins and unrelated values untouched.
`RuntimeConfig::validate_and_save` runs a caller-supplied decision
against the snapshot current at the moment the write lock is taken and only
persists on success, so validation and persistence serialize behind the same
lock (no separate read-then-write race). `ConfigSnapshot::required` is the one
accessor for "this key's value, or a descriptive error" that `src/config.rs`,
`src/mcp/config.rs`, and `src/git/config.rs` all call rather than each keeping
its own copy; `ConfigSnapshot::pinned_count` and `RuntimeConfig::settings_path`
support the startup pinned-setting log line and local-versioning `.gitignore`
setup respectively.

**Consumers:** runtime composition constructs the startup instance. The
settings HTTP API and the archive, index, MCP, and git live consumers bind a
snapshot in their respective capability boundaries. The legacy single-Vault
import reads one startup snapshot and removes migrated stored keys only after
the Vault registry commit succeeds.

**Coordination paths:** `src/lib.rs` exports the boundary. Runtime composition,
`src/config.rs`, `src/mcp/config.rs`, `src/git/config.rs`, and `src/app_state.rs`
consume it as live settings are integrated; no consumer may re-read process
environment variables after startup.

**Invariants:** environment values that are non-empty after trimming are
captured once and remain pinned above stored values. The store lives beside the
cache database unless the deployment-only override selects another path; it is
created with `0600` permissions on Unix. Corrupt, unsupported, and future
schemas fail with recovery guidance and are never overwritten.

**Validation:** `cargo test runtime_config`, followed by the full backend
checks.

### Vault collection registry

**Kind:** infrastructure/persistent domain state.

**Owned paths:** `src/vault_registry.rs`.

**Public contract:** `DEFAULT_VAULT_REGISTRY_PATH`,
`REGISTRY_SCHEMA_VERSION`, canonical `VaultId` generation and parsing,
`VaultRegistryStore`, immutable `VaultRegistrySnapshot` values, explicit
`VaultRegistryState::Ready` versus `Recovery`, structured recovery/error
types, redacted `VaultDefinition` projections, tagged `VaultSource` values for
local, existing-Git, and managed-Git Vaults, `VaultGitMode`, credential write
inputs/updates, validated `add`/`edit`/`enable`/`disable`/`disconnect`
operations, store-owned `vault_path` resolution for runtime consumers,
the crate-private `is_safe_https_repository_url` validator shared with the
managed-checkout boundary, the crate-private `https_credentials` accessor
that returns plaintext credentials for one Vault ID (`None` for both an
absent Vault and one with none configured, so it cannot be used to probe
existence) for the managed-Git Git-turn dispatch boundary's internal use only
— never exposed to HTTP, MCP, or any other external-facing surface,
explicit confirmed-empty initialization for migration recovery, and the
versioned `/data/state/vaults.json` format. An absent file
is a complete revision-0 zero-Vault state and is not created by reads. Commits
are serialized by normalized registry path across all store handles in the
process, compare the expected persisted revision, increment it once, and
atomically replace the file with owner-only permissions. Corrupt, unsupported,
future-schema, or structurally invalid definition files expose no Vault records
and are never overwritten automatically.

**Consumers:** the legacy single-Vault import consumes the registry load, add,
and confirmed-empty initialization contracts. Runtime composition loads the
registry after migration and `VaultCollectionRuntime` consumes its safe
projections and resolved paths; its managed-Git Git-turn dispatch
(`dispatch_managed_git_turn`) is the one consumer of the crate-private
`https_credentials` accessor. `handlers/vaults.rs` is the first HTTP consumer
of the `add`/`edit`/`enable`/`disable`/`disconnect` mutation contracts and of
`load` for authenticated discovery, including its explicit `Recovery` state.
MCP, frontend, cache, and search adapters remain separately owned later
packets.

**Coordination paths:** `src/lib.rs` exports the boundary; `src/server.rs` and
`src/app_state.rs` construct and retain it; `/data/state` deployment
persistence and later management adapters require their own declared work
packets.

**Invariants:** the registry is the sole Hatchdoor-owned Vault-definition
authority; immutable IDs are UUID v4 map keys; revision conflicts save nothing;
names are unique case-insensitively; canonical Vault paths never overlap and
disabled definitions continue reserving them; identity-bearing changes require
a disabled definition plus explicit same-Vault confirmation; readable
non-writable directories remain valid; disconnect deletes no files or Git
state; HTTPS credentials persist only in the private registry record and never
appear in projections, debug output, errors, status, or repository URLs;
recovery retains the original bytes; Vault contents remain authoritative
Markdown and SQLite remains disposable (ADR-01); the store adds no service,
framework, or speculative trait (ADR-02/13); filesystem behavior assumes no
runtime shell and remains usable by the rootless image (ADR-12).

**Validation:** `cargo test vault_registry`,
`node scripts/check-module-map.mjs`, followed by the full backend checks.

### Legacy single-Vault import

**Kind:** infrastructure/migration boundary.

**Owned paths:** `src/vault_migration.rs`.

**Public contract:** `LegacyMigrationInput`, `LegacyMigrationOutcome`,
`LegacyMigrationRecovery`, `LegacyMigrationError`, `migrate_legacy_vault`, and
`start_with_no_vaults`. Inspection returns a deterministic no-deployment,
existing-registry, imported, or stable `legacy_migration_required` recovery
outcome. Any existing registry, including an intentionally empty one,
permanently suppresses legacy import. Confirmed Start with no Vaults writes an
ordinary revisioned zero-Vault registry.

**Consumers:** startup runtime composition calls this isolated adapter before
opening the disposable cache and activating Vault runtimes. Safe imports become
ordinary enabled definitions; recovery activates no Vault and remains in
`AppState` for later setup/management surfaces.

**Coordination paths:** `src/lib.rs` exports the boundary;
`docker-compose.yml`, `.env.example`, `README.md`, and
`docs/migrations/legacy-single-vault.md` document and persist the registry
required by the migration contract.

**Consumed dependencies:** the Vault collection registry owns definitions and
atomic persistence; live configuration owns precedence and stored-setting
cleanup; `src/config.rs` owns exclusion parsing; the cache boundary owns
read-only legacy-schema recognition; `git2` and filesystem metadata provide
inspection only.

**Invariants:** detection requires positive legacy evidence, so a fresh empty
default does not migrate. Registry persistence completes before migrated
settings or a recognized disposable cache are removed. Inspection never seeds,
moves, edits, clones, pulls, commits, pushes, checks out, or merges legacy
content or Git state. Unsafe conversion leaves all legacy state unchanged and
returns recovery. Markdown remains authoritative and downgrade across the
registry cutover is unsupported.

**Validation:** `cargo test vault_migration`, `cargo test vault_registry`,
`cargo test runtime_config`, `cargo test cache`,
`node scripts/check-module-map.mjs`, followed by the full backend checks.

### Web authentication

**Kind:** infrastructure/security.

**Owned paths:** `src/auth.rs`.

**Public contract:** `WebToken`, `WebOrLiveMcpToken`,
`require_web_token`, and `require_web_or_live_mcp_token`. Hatchdoor's internal
attachment middleware binds the MCP token from the current runtime snapshot
instead of retaining a token captured at startup; it accepts that token only
while MCP itself is enabled. Disabling MCP at runtime immediately revokes that
token for attachment uploads; MCP write mode does not affect this authorization.

**Consumers:** `server.rs` and protected HTTP routes.

**Consumed dependencies:** live runtime configuration and `McpConfig` parsing
for per-request attachment authorization.

**Coordination paths:** `src/server.rs`, `src/config.rs`, frontend
`frontend/src/api/api.ts`, and any route whose authentication requirements
change.

**Invariants:** constant-time token comparison, no token logging, and deliberate
query-parameter fallback for browser contexts that cannot set headers (ADR-08).

**Validation:** `cargo test auth` and server/router tests.

### HTTP wire types

**Kind:** shared contract.

**Owned paths:** `src/api_types.rs`.

**Public contract:** the shared serialized request and response structures
defined here, including note, links, resolve, refresh, recent, stats, graph,
and search query shapes. Endpoint-local wire types remain owned by their
handlers, notably write types in `handlers/write_api.rs` and diagnostics types
in `handlers/diagnostics.rs`.

**Consumers:** `src/handlers/**` and the manually corresponding frontend types
in `frontend/src/types.ts` or feature-local client types.

**Coordination rule:** serialized field changes are interface changes. The work
packet must identify backend handlers, frontend consumers, and compatibility
expectations. Additive response fields are usually compatible but still require
the frontend contract to be checked.

**Validation:** affected backend handler tests, affected frontend consumer
tests, and frontend typecheck. Rust and TypeScript wire shapes are manually
synchronized; no automated cross-language schema check currently exists.

### Vault read model and filesystem interpretation

**Kind:** product capability/domain core.

**Owned paths:**

- `src/vault.rs`
- `src/vault/exclude.rs`
- `src/vault/index.rs`
- `src/vault/layers.rs`
- `src/vault/links.rs`
- `src/vault/paths.rs`
- `src/vault/seed.rs`
- `src/vault/types.rs`
- `src/vault/tests.rs`

**Public contract:** the intentional re-exports from `src/vault.rs`, notably
`VaultIndex`, note/tree/link types, path normalization helpers, layer and
exclusion types, and `seed_empty_vault`.

**Consumed dependencies:** filesystem traversal and parsing; `cache::parse`
currently supplies content hashing to the index.

**Consumers:** cache population, handlers, MCP reads, write coordination,
watching, and application startup.

**Coordination paths:** `src/cache/**`, `src/vault_watcher.rs`,
`src/api_types.rs`, and adapters when a public vault type changes.

**Invariants:**

- Markdown files remain authoritative (ADR-01).
- Excluded/noise paths do not enter the index.
- Layer markers remain visible to classification even under broad exclusions.
- A note remains addressable while its layer is reported to callers.

**Validation:** `cargo test vault` and the full backend checks.

### Vault mutation

**Kind:** product capability/domain core; safety-critical.

**Owned paths:**

- `src/vault/write.rs`
- `src/vault/write/assets.rs`
- `src/vault/write/attachments.rs`
- `src/vault/write/fs_ops.rs`
- `src/vault/write/notes.rs`
- `src/vault/write/paths.rs`
- `src/vault/write/rewrites.rs`
- `src/vault/write/types.rs`
- `src/vault/write/tests.rs`

**Public contract:** write functions and result/error types re-exported from
`src/vault.rs`, including note CRUD-by-move, section/edit primitives, attachment
operations, allowed attachment extensions, `WriteOutcome`, and `WriteError`.

**Consumed dependencies:** vault index/types and the local filesystem.

**Consumers:** HTTP write handlers and MCP write tools.

**Coordination paths:** `src/handlers/write_api.rs`,
`src/mcp/tools/write.rs`, Git write records, frontend write API/types, and
configuration for archive or upload limits.

**Invariants:**

- All HTTP and MCP mutations use this shared layer (ADR-03).
- Optimistic concurrency uses the expected content hash.
- Delete is recoverable trash; archive is move-based (ADR-11).
- Paths remain within the canonical vault root.
- Layer marker and excluded/noise writes remain protected at adapter and domain
  boundaries as applicable.

**Validation:** `cargo test vault::write`, adapter write tests, and the full
backend checks.

### Vault-qualified read projections

**Kind:** product capability/domain core.

**Owned paths:** `src/vault_read.rs`.

**Public contract:** `VaultReadCore`, explicit `VaultScope`, the common
`VaultReadProjection` envelope, participant state/error types, and
Vault-qualified exact-note, tree, statistics, graph, and recent-note
projections.

**Consumed dependencies:** the Vault runtime's authoritative per-Vault index,
the shared cache's published Vault snapshot seam, and existing Vault note/link
types.

**Consumers:** future HTTP and MCP Vault-scoped adapters. The core has no
adapter or route ownership.

**Coordination paths:** `src/cache/vault_snapshots.rs` for read-only
Vault-qualified snapshot rows, `src/cache/mod.rs` for the crate-private seam,
and `src/vault_runtime.rs` for the authoritative exact-read index boundary.

**Invariants:**

- Exact reads inspect the requested Vault's Markdown directory; SQLite remains
  a disposable projection (ADR-01).
- Every selected or returned note identity includes an immutable Vault ID; no
  default or sole-Vault inference exists.
- One-Vault snapshots are explicit about stale availability, unavailable
  snapshots never become empty data, and all-Vault reads preserve participant
  status and Vault grouping.
- Trees, statistics, and graphs remain grouped by Vault; graph edges never
  cross a Vault boundary.

**Validation:** `cargo test vault_read`, focused cache snapshot tests, and the
full backend checks.

### Cache and query read model

**Kind:** infrastructure/read model.

**Owned paths:**

- `src/cache/mod.rs`
- `src/cache/chunk_ops.rs`
- `src/cache/parse.rs`
- `src/cache/populate.rs`
- `src/cache/schema.rs`
- `src/cache/queries/mod.rs`
- `src/cache/queries/graph.rs`
- `src/cache/queries/metadata.rs`
- `src/cache/queries/search.rs`
- `src/cache/vault_snapshots.rs`

**Public contract:** `SqliteCache`, `ReadConn`, `BuildOptions`, `SemanticHit`,
and the methods implemented on `SqliteCache`. The crate-private
`vault_snapshots` seam owns Vault-ID-qualified candidate publication,
stale/participation state, attempt ordering, and Vault-local disposal in the shared cache. `parse` is currently public and
also supplies parsing/hash behavior to vault indexing. The crate-private
`is_recognized_legacy_cache` inspection seam owns the supported legacy schema
fingerprint and opens existing files read-only for the one-time migration.

**Consumed dependencies:** Vault IDs and index/types, chunking, embeddings, SQLite,
FTS5, and sqlite-vec.

**Consumers:** application state/reindexing, Vault-qualified read projections,
Search, handlers, MCP reads, evaluation tooling, diagnostics, and the one-time
legacy single-Vault migration's read-only evidence check.

**Coordination paths:** `src/app_state.rs`, `src/vault_read.rs`,
`src/search/**`, `src/vault/index.rs`, `src/chunk/**`, and embedder
identity/dimensions.

**Invariants:**

- SQLite is rebuildable and never authoritative (ADR-01).
- Keep embedded SQLite, FTS5, sqlite-vec, WAL, one writer, and pooled
  query-only reads (ADR-06).
- Schema or embedder identity mismatch rebuilds rather than mixing data.
- A refresh commits a coherent new read snapshot.
- Every shared snapshot row and relationship is Vault-ID-qualified; failed
  replacement retains the prior snapshot as stale, disabling removes only
  participation, and disconnect deletes only that Vault's disposable rows.

**Validation:** `cargo test cache` and full backend checks. Schema/population
changes require search and application-state tests too.

### Chunking

**Kind:** infrastructure/indexing policy.

**Owned paths:**

- `src/chunk/mod.rs`
- `src/chunk/chunker.rs`
- `src/chunk/normalize.rs`

**Public contract:** `Chunk`, `ChunkOptions`, `NoteChunking`, `chunk_note`, and
normalization behavior re-exported by `src/chunk/mod.rs`.

**Consumed dependencies:** Markdown text and tokenizer-aware splitting.

**Consumers:** cache population and evaluation/index microbench tooling.

**Coordination paths:** cache population, embedder token limits, and evaluation
baselines.

**Invariants:** chunk boundaries and contextual text changes alter every
embedding and therefore require deliberate evaluation, not only unit tests.

**Validation:** `cargo test chunk`, cache population tests, and relevant eval
commands when retrieval behavior may change.

### Runtime Search

**Kind:** product capability/domain service.

**Owned paths:**

- `src/search/mod.rs`
- `src/search/assemble.rs`
- `src/search/layer_selection.rs`
- `src/search/retrieve.rs`
- `src/search/vault_scoped.rs`

**Public contract:** `SearchMode`, `SearchRequest`, `NoteFilters`,
`LayerSelection`, `LayerInfo`, `SearchResult`, `SearchResponse`, `run`, and
`query_notes`. The Vault-qualified shared-core contract is `VaultSearchCore`,
`VaultSearchRequest`, `VaultSearchResponse`, and `VaultSearchResult`; it uses
the explicit `VaultScope` and common projection/participant envelope from the
Vault-read core without owning any HTTP, MCP, or frontend adapter.

**Consumed dependencies:** `SqliteCache`, its published Vault snapshot/cache
query seam, `Embedder`, the Vault collection runtime, the explicit Vault-read
scope/envelope, and vault metadata/types.

**Consumers:** HTTP search handler, MCP search/query tools, offline evaluation
runners, and future Vault-scoped adapters.

**Coordination paths:** `src/api_types.rs`, `src/handlers/api.rs`,
`src/mcp/tools/read.rs`, cache query methods, and frontend Search contracts.

**Invariants:**

- Runtime search defaults to pure semantic retrieval; hybrid and reranking stay
  offline (ADR-05).
- Layer selection and metadata filters must never widen the eligible result
  set.
- Vault-qualified search filters before ranking, globally ranks every usable
  Vault snapshot, caps by `(Vault ID, slug)`, and never deduplicates equal
  content or note names across Vaults. Staleness is participant status, not a
  relevance penalty.
- A structure-only frontend Search pilot must not modify these paths.

**Validation:** `cargo test search`, focused Vault-scoped and cache query tests,
and evaluation-only checks when retrieval semantics change.

### Embeddings and model implementations

**Kind:** infrastructure/external-model seam.

**Owned paths:**

- `src/embed/mod.rs`
- `src/embed/candle_embedder.rs`
- `src/embed/context.rs`
- `src/embed/embedder.rs`
- `src/embed/fastembed_embedder.rs`
- `src/embed/hub.rs`
- `src/embed/matryoshka.rs`

**Public contract:** `Embedder`, `RuntimeEmbedder`, concrete embedders,
`MatryoshkaEmbedder`, `StubEmbedder`, and contextual-document formatting.

**Consumed dependencies:** local model runtimes, tokenizers, and Hugging Face
model files.

**Consumers:** cache building, runtime Search, startup/model setup, auxiliary
evaluation binaries, and tests.

**Coordination paths:** `src/model_setup.rs`, cache schema/identity handling,
chunking, Docker model prefetch, and evaluation documentation.

**Invariants:** local inference only (ADR-04); embedder identity must encode
behavior affecting stored vectors; the `Embedder` trait remains the deliberate
test seam rather than proliferating model abstractions (ADR-13).

**Validation:** `cargo test embed`; feature-gated or model-loading tests when
applicable; cache identity/rebuild tests for identity changes.

### Reranking

**Kind:** offline evaluation infrastructure.

**Owned paths:**

- `src/rerank/mod.rs`
- `src/rerank/fastembed_reranker.rs`
- `src/rerank/reranker.rs`

**Public contract:** `Reranker`, `FastembedReranker`, `StubReranker`, and
`RerankedHit`.

**Consumers:** evaluation tooling only.

**Coordination paths:** `src/eval/**` and `src/bin/eval.rs`.

**Invariant:** reranking must not enter the runtime search path without
superseding ADR-05.

**Validation:** `cargo test rerank` and relevant eval runner tests.

### Git synchronization

**Kind:** infrastructure/background capability.

**Owned paths:**

- `src/git/mod.rs`
- `src/git/config.rs`
- `src/git/managed_checkout.rs`
- `src/git/managed_sync.rs`
- `src/git/managed_task.rs`
- `src/git/message.rs`
- `src/git/status.rs`
- `src/git/sync.rs`
- `src/git/task.rs`

**Public contract:** `GitMode` (`off`/`local`/`remote` through the existing
runtime setting), `GitConfig`, write-record/message types, sync outcomes and
errors, lifecycle status, repository operations, `GitSyncHandle`, `SyncOps`,
and `spawn_sync_task`. The crate-private `parse_mode` and `non_empty_setting`
helpers keep startup and one-time migration interpretation identical. Local
mode commits without network access and discovers an enclosing existing
checkout while staging only the configured Vault subtree; remote mode
retains the safe fetch/integrate/push phases. The settings HTTP boundary owns
the preflight → bounded drain → replacement protocol and exposes it through
`GET /api/git-status`. `GitSyncHandle::stop` timing out withdraws its stop
request rather than leaving the handle dead: the still-running task keeps
accepting and committing records, and only a `stop` that truly returns `Ok`
lets a caller install a replacement task. Spawning a task flushes any
drift accumulated while versioning was off (uncommitted working-tree changes
in either mode, or unpushed commits in remote mode) under its own
distinguishable commit message. `init_local_repo` takes the vault's configured
cache-database and settings-file paths and derives `.gitignore` entries from
them (only when those paths live inside the vault), appending to an existing
`.gitignore` rather than skipping it.

`ManagedCheckoutLease`, `ManagedCheckoutRequest`, `ManagedHttpsCredentials`,
and `acquire_or_reuse` form the shared-core managed-HTTPS acquisition boundary.
It holds a per-Vault process ownership lease, clones only into an
application-owned temporary sibling, validates origin, branch, repository
shape, and canonical Vault containment before atomic installation, and writes
an application-owned receipt that retains a once-resolved default branch.
Reuse accepts only a receipt-backed matching checkout; unknown, interrupted,
damaged, mismatched, credential-bearing, or out-of-containment destinations
remain untouched and are rejected. This boundary neither fetches nor resets,
checks out, polls, pushes, or attempts automatic reacquisition/recovery.

`ManagedSyncConfig`, `ManagedSyncMode`, `ManagedSyncOutcome`,
`ManagedSyncError`, and `synchronize_managed_checkout` form the next shared-core
managed-checkout graph boundary. A caller that holds the checkout lease and
serializes Vault writes supplies the already validated repository and contained
Vault root. Pull-only refuses and preserves any local work or local-only
history, then only fast-forwards a clean checkout. Two-way commits Vault-subtree
work before every tree-changing graph operation, refuses unrelated repository
work, fast-forwards remote-only advancement, creates a merge commit for clean
divergence, aborts a verified clean conflict back to the pre-merge local commit,
and never pushes after conflict. It uses safe checkout transitions and rejects
outside-Vault dirt rather than overwriting it; the narrowly scoped conflict
abort is the only hard reset. A non-fast-forward push retries only through one
bounded fetch-integrate-push graph replay before returning a redacted push-race
error. Every configured remote and push URL must remain credential-free HTTPS.
Public HTTPS makes no credential callback; supplied credentials are callback
input only and remain redacted. This boundary does not acquire, delete,
schedule, poll, persist status, or repair checkouts.

`ManagedGitTurnConfig`, `ManagedGitOutcome`, `run_managed_git_turn`,
`ManagedGitScheduler`, `spawn_scheduler_tick`, `DEFAULT_POLL_INTERVAL`, and
`DEFAULT_TICK_INTERVAL` form the per-Vault managed-Git scheduling boundary —
the "later consumer" the two paragraphs above anticipated. `run_managed_git_turn`
is the concrete `acquire_or_reuse`-then-`synchronize_managed_checkout` operation
`VaultWorkKind::Git` executes; it classifies every `ManagedCheckoutError`/
`ManagedSyncError` into a redacted `VaultWorkError{code, message, retryable}`,
distinguishing authentication failures (`ManagedCheckoutError::AuthenticationFailed`,
`ManagedSyncError::Authentication`, detected via `git2::ErrorCode::Auth`) from
other remote failures. `ManagedGitScheduler` is one process-wide instance —
mirroring the coordinator's single-worker design, it adds no per-Vault
execution lane — that decides *when* to request a Vault's next Git turn:
`DEFAULT_POLL_INTERVAL` (24h, not yet user-configurable) after a success or any
non-retryable failure (including authentication, which never backs off — it
waits for a configuration change, a manual `sync_now`/`retry_now`, a restart, or
the normal schedule), or bounded exponential backoff after a retryable
(transient) failure. `spawn_scheduler_tick` drives it on `DEFAULT_TICK_INTERVAL`.

**Consumed dependencies:** local Git repository through `git2`, the live
configuration snapshot for startup parsing, and the registry's shared
credential-free HTTPS URL validator, `VaultId` identity, and the crate-private
`https_credentials` accessor (managed-Git turns only; never exposed further).

**Consumers:** server startup, write adapters, status handlers/tools,
`AppState`, and the one-time legacy single-Vault migration parser.
`ManagedGitScheduler`/`run_managed_git_turn` are consumed by runtime
composition (`src/vault_runtime.rs::dispatch_managed_git_turn` and
`reconcile_and_reconstruct`, which activates/deactivates a managed-Git Vault's
schedule alongside its coordinator admission) and by `src/server.rs`, which
owns the one global consumer loop driving `VaultWorkWorker::run_next` — the
worker/scheduler-tick construction and dispatch this module map previously
noted as missing. `VaultWorkKind::Index`/`Repair` dispatch remains for a later
cache/repair packet; `src/server.rs` returns an explicit non-retryable
"not yet implemented" `VaultWorkError` for those kinds so a Vault's shared FIFO
position is not blocked ahead of its Git turn.

**Coordination paths:** `src/app_state.rs`, `src/server.rs`,
`src/vault_runtime.rs`, `src/vault_registry.rs` (crate-private
`https_credentials` accessor), `src/handlers/settings.rs`, HTTP/MCP write
adapters, configuration, frontend settings UI, and vault watcher Git
exclusions.

**Invariants:** optional and debounced; writes do not block on sync; task
replacement drains before another task can start; local mode never contacts a
remote; remote mode never force-checks out over uncommitted manual vault edits
(ADR-10). Managed acquisition never writes credentials to URLs, Git
configuration, reads, logs, errors, or status; it never deletes, overwrites,
or silently adopts a checkout destination. The managed-Git scheduler adds no
persisted queue, priority, or second execution lane (ADR-13); a Git turn's
returned failure always completes that Vault's turn so the shared worker is
released for the next Vault.

**Validation:** `cargo test git`, `cargo test managed_checkout`, and affected
adapter/server tests. Managed graph changes additionally run `cargo test
managed_sync` against local bare-repository fixtures; scheduling changes run
`cargo test managed_task` and `cargo test vault_runtime`.

### HTTP adapters

**Kind:** adapter.

**Owned paths:**

- `src/handlers/mod.rs`
- `src/handlers/api.rs`
- `src/handlers/assets.rs`
- `src/handlers/diagnostics.rs`
- `src/handlers/downloads.rs`
- `src/handlers/settings.rs`
- `src/handlers/spa.rs`
- `src/handlers/vaults.rs`
- `src/handlers/write_api.rs`

**Public contract:** handler functions intentionally re-exported by
`src/handlers/mod.rs`; their route, authentication, status, and serialized HTTP
behavior. `settings.rs` owns the additive `/api/settings` document: effective
value/provenance/lock/class/kind metadata and partial PATCH saves returning the
full refreshed document. MCP enablement and its bearer token validate together
against one prospective snapshot, so an invalid combination saves nothing and
reports field errors. Its candidate-token and capability-safe secret-reveal
endpoints are `no-store`; the ordinary settings document never exposes secret
values. A save whose consequence needs consent (a reindex, initializing local
history, or downgrading away from remote versioning) is refused with `409` and
a machine-readable `confirmation_required` consequence — the server is the
authority, and sends no prose; the page owns the words and resends with a
`confirm` list that accumulates every consequence accepted so far, so a save
needing two consents does not ping-pong between them. Saves persist before
asynchronously rebuilding; `/api/index-status` reports that dedicated
rebuild's staleness, progress, ETA, and last failure without reusing startup
readiness.

`vaults.rs` owns `/api/v1/vaults` discovery, collection management (create/
edit/enable/disable/disconnect), manual Git sync/retry, and the collection-wide
SSE event stream — the first `/api/v1` surface. It is the first HTTP consumer
of the Vault collection registry's write operations and of
`VaultCollectionRuntime::reconcile_and_reconstruct`, which every mutation calls
in the same request so background Index/Git work is requested for a newly
enabled Vault without a separate reconciliation pass. Deliberately not gated by
`require_vault_ready` (a legacy single-configured-Vault signal) or reachable in
demo mode (mirrors `settings.rs`'s operator-controls posture): discovery and
creating the first Vault stay reachable at zero enabled Vaults, and discovery
reports an explicit `recovery` object rather than erroring when the persisted
registry itself needs operator recovery. Every response uses the shared
`VaultApiError{code, message, vault_id?, retryable}` shape and reuses
`vault_registry::VaultSource`/`VaultGitMode` directly on the wire rather than
duplicating them. Vault-scoped content reads/mutations and one-or-all
collection reads remain separately owned later packets (#99–#101); MCP
discovery is #103.

**Consumed dependencies:** `AppState`, HTTP wire types, vault reads,
`vault/write`, Search, cache queries, Git status, auth, and — for `vaults.rs`
only — the Vault collection registry's mutation/load operations,
`VaultCollectionRuntime::{snapshot, reconcile_and_reconstruct,
subscribe_revisions}`, `VaultWorkCoordinator`, and
`ManagedGitScheduler::{sync_now, retry_now}` via `AppState::{vault_work,
managed_git}`.

**Consumers:** route construction in `src/server.rs`.

**Coordination paths:** `src/server.rs`, `src/api_types.rs`, frontend clients,
and whichever domain a handler adapts.

**Invariants:** handlers stay thin. Write handlers never touch the vault
filesystem directly (ADR-03). Static and vault asset behavior must retain auth
and path containment. `vaults.rs` never returns HTTPS credentials, only
`credential_configured` (ADR-01/registry invariant); disconnect deletes no
files, checkouts, Git history, or credentials outside the registry record.

**Validation:** `cargo test handlers`, router tests, and affected domain tests.

### MCP adapter

**Kind:** adapter/security surface.

**Owned paths:**

- `src/mcp/mod.rs`
- `src/mcp/auth.rs`
- `src/mcp/config.rs`
- `src/mcp/protocol.rs`
- `src/mcp/routes.rs`
- `src/mcp/tools/mod.rs`
- `src/mcp/tools/read.rs`
- `src/mcp/tools/write.rs`

**Public contract:** `/mcp` Streamable HTTP behavior, `McpConfig`, protocol
version negotiation, server instructions, tool names/schemas/results, and
`mcp_get_handler`/`mcp_post_handler`.

**Consumed dependencies:** `AppState`, Search, vault reads, `vault/write`, Git
status, model setup, cache refresh, attachment limits, and the live
configuration snapshot bound at each request.

**Coordination paths:** `src/server.rs`, domains exposed as tools, and
documentation describing agent behavior.

**Invariants:** MCP is disabled by default, uses its own token, validates
Origins, and keeps read-only access credentialed (ADR-09). Token changes,
write enablement, Origins, and attachment limits apply to the next request;
attachment authorization never retains a rotated MCP token. Write tools use
`vault/write` and retain optimistic concurrency and path protections (ADR-03).

**Validation:** `cargo test mcp`, vault write tests for mutation changes, and
server router tests.

### Evaluation and development binaries

**Kind:** offline tooling; not a runtime feature.

**Owned paths:**

- `src/eval/mod.rs`
- `src/eval/compare_runner.rs`
- `src/eval/hybrid_runner.rs`
- `src/eval/metrics.rs`
- `src/eval/query.rs`
- `src/eval/report.rs`
- `src/eval/rerank_runner.rs`
- `src/bin/eval.rs`
- `src/bin/index_microbench.rs`

**Public contract:** evaluation query JSONL, metrics/report formats, CLI
arguments, and reproducible comparison behavior.

**Consumed dependencies:** cache, embeddings, chunking, Search, and Reranking.

**Coordination paths:** `eval/**`, related findings under `docs/`, and model or
chunking code when experiments become runtime decisions.

**Invariant:** hybrid and rerank experiments remain offline unless ADR-05 is
superseded.

**Validation:** `cargo test eval`, binary argument tests, and the relevant eval
command for behavioral changes.

## Frontend

The frontend currently uses technical-layer directories rather than enforced
feature boundaries. The ownership below assigns each production file to one
capability or marks it shared. Except for Search's TS/TSX façade rule,
boundaries are currently documentation-enforced.

### Application shell and navigation

**Kind:** composition/shared.

**Owned paths:** none by default.

**Paths:**

- `frontend/src/main.tsx`
- `frontend/src/App.tsx`
- `frontend/src/app/AppTopbar.tsx`
- `frontend/src/app/ExplorerPane.tsx`
- `frontend/src/app/constants.ts`
- `frontend/src/hooks/useIsMobile.ts`
- `frontend/src/hooks/useTheme.ts`
- `frontend/src/lib/storage.ts`

**Contract and responsibility:** bootstraps React/router/PWA, composes feature
hooks and routes, owns responsive shell state, navigation, persistent shell
preferences, topbar actions, and explorer placement.

**Coordination rule:** feature work may touch `App.tsx` only when the work
packet names the route, callback, shortcut, or state integration. A large prop
surface is a coordination seam, not permission to move feature behavior into
the shell.

**Validation:** the applicable `App.*.test.tsx`, `app/ExplorerPane.test.tsx`,
`useTheme.test.tsx`, storage tests, then full frontend checks. Layout changes to
the explorer pane need a browser as well as the suite: its zone structure
depends on real cascade behavior that jsdom does not reproduce.

### Frontend API, authentication, and shared wire contracts

**Kind:** infrastructure/shared contract.

**Owned paths:**

- `frontend/src/api/api.ts`
- `frontend/src/api/apiError.ts`
- `frontend/src/components/TokenPrompt.tsx`

**Shared path:** `frontend/src/types.ts`.

**Contract and responsibility:** authenticated/time-bounded fetch, unauthorized
notification, tokenized asset/download/SSE URLs, error extraction, login prompt,
and cross-capability TypeScript representations of backend payloads. A feature
may own its wire types when all consumers go through that feature's public
entry point, as Search now does.

**Consumers:** almost every data-backed frontend capability.

**Coordination rule:** `types.ts` is not owned by whichever feature needs one
new field. Contract changes must list the backend serializer and all frontend
consumers. New feature-local types should remain local unless genuinely shared.

**Invariants:** preserve bearer/header behavior and the deliberate query-token
fallback (ADR-08). Never log or render tokens.

**Validation:** API/error tests, affected feature/consumer tests, and typecheck.
`clientAuditContracts.test.ts` audits UI, PWA, and CSS source contracts; it does
not verify Rust-to-TypeScript wire compatibility.

### Startup and model setup UI

**Kind:** product capability/adapter.

**Owned paths:**

- `frontend/src/startup/StartupGate.tsx`
- `frontend/src/styles/startup.css`

**Public contract:** `StartupGate` and the `/api/startup-status` plus model
accept/decline/retry response shapes it consumes.

**Consumed dependencies:** shared API client and theme hook.

**Coordination paths:** `App.tsx`, backend startup/model setup handlers and
types, and shell-wide styles.

**Validation:** `StartupGate.test.tsx`, `App.startup-auth.test.tsx`, and full
frontend checks.

### Vault explorer

**Kind:** product capability.

**Owned paths:**

- `frontend/src/components/Explorer.tsx`
- `frontend/src/components/ChangesPanel.tsx`
- `frontend/src/hooks/useVaultTree.ts`
- `frontend/src/lib/folderPaths.ts`
- `frontend/src/lib/noteCandidates.ts`
- `frontend/src/styles/layout-explorer.css`

**Public contract:** `useVaultTree`, explorer tree/list components, derived
folder paths, and flattened note candidates. The sidebar is three zones — a
fixed rail, a scrolling nav, a fixed footer — and `.explorer-nav` is the scroll
container the shell restores scroll position against, not the pane itself.
`ChangesPanel` lists notes changed on disk; it deliberately carries no unread
count, because distinguishing external changes from the user's own edits needs
backend data that does not exist yet.

**Consumed dependencies:** shared API/error utilities, shared wire types,
router links, and shared UI components.

**Coordination paths:** `App.tsx`, `app/ExplorerPane.tsx`, `types.ts`,
`lib/stateCompare.ts`, responsive CSS, and backend tree/recent/event endpoints.

**Validation:** folder/note-candidate/state comparison tests and affected App
navigation tests; `app/ExplorerPane.test.tsx` covers the tree and list
components in composition, including the single-active-highlight invariant. The
hook still needs focused coverage.

### Search dialog

**Kind:** product capability; established feature boundary.

**Owned paths:**

- `frontend/src/features/search/index.ts`
- `frontend/src/features/search/types.ts`
- `frontend/src/features/search/SearchDialog.tsx`
- `frontend/src/features/search/useSearch.ts`
- `frontend/src/features/search/search.css`

Feature tests:

- `frontend/src/features/search/SearchDialog.test.tsx`
- `frontend/src/features/search/useSearch.test.ts`

**Public contract:** `frontend/src/features/search/index.ts` is the only public
TS/TSX entry point. It exposes `useSearch`, `SearchDialog`, Search wire and
selection types, and the `/api/search` payload consumed by the hook. Search CSS
is integrated separately through the `App.css` stylesheet aggregation seam.

**Consumed dependencies:** shared API/error utilities, shared UI components,
router navigation supplied by the shell, and backend Search.

**Coordination paths:** `App.tsx`, `App.css`, backend search HTTP contract, and
responsive CSS.

**Pilot constraint:** co-location or façade work is structure-only. It must not
change backend retrieval, ranking, cache, or MCP behavior.

**Boundary enforcement:** production TS/TSX files outside the feature must
import the directory entry point rather than its internal files; ESLint
enforces this with `no-restricted-imports`. The raw source-audit test is
explicitly exempt, and CSS aggregation remains the declared `App.css` seam.

**Validation:** the feature's `SearchDialog.test.tsx` and `useSearch.test.ts`,
`App.navigation-search.test.tsx`, and full frontend checks.

### Note reading and rendering

**Kind:** product capability.

**Owned paths:**

- `frontend/src/components/NotePage.tsx`
- `frontend/src/components/note-page/NotePreview.tsx`
- `frontend/src/components/note-page/PdfPreview.tsx`
- `frontend/src/components/note-page/RendererComponents.tsx`
- `frontend/src/components/note-page/dom.ts`
- `frontend/src/components/note-page/paragraphs.ts`
- `frontend/src/components/note-page/renderers.tsx`
- `frontend/src/components/note-page/sections.tsx`
- `frontend/src/components/note-page/text.ts`
- `frontend/src/components/note-page/wikilinks.ts`
- `frontend/src/lib/markdown.ts`
- `frontend/src/lib/noteHeadings.ts`
- `frontend/src/lib/noteSearch.ts`
- `frontend/src/noteEnhancements.css`
- `frontend/src/styles/note-content.css`

**Public contract:** `NotePage`, note preview/rendering behavior, safe asset and
wikilink resolution, heading/search-hit navigation, Markdown transformations,
note navigation/rendering behavior, the editable-block component map produced by
`createNoteMarkdownComponents`, the paragraph marker `CalloutOrQuote` uses to
recognise its own first child, and the soft-break splitter that reconstructs one
source line per rendered line for the two unit types addressed per line.

**Consumed dependencies:** API/auth helpers, router state, Markdown/rendering
libraries, shared types/UI, and note editing.

**Coordination paths:** `App.tsx`, `types.ts`, note/link/resolve/download
handlers, `NoteEditor.tsx`, Search query navigation, shared and responsive CSS.

**Invariants:** vault Markdown remains the rendered source; vault content is
data rather than trusted executable instructions; asset URLs retain auth and
path safety; **the rendered body keeps one line per source line**, since inline
editing addresses blocks by line number and a transform that collapses lines
would write to the wrong place (`linesMatch` enforces this at runtime and
disables inline editing for that note); a callout body and a wrapped list item
are rebuilt rather than passed through, so their positions do not survive and a
line's **index** is the only thing mapping it back to the file, which is why no
interior line is dropped while splitting and why a list item whose rendered line
count disagrees with the span it claims is addressed whole rather than written to
a guessed line.

**Validation:** note-page unit tests, Markdown/heading/search/state tests,
`App.content-rendering.test.tsx`, `App.enhancements.test.tsx`,
`App.links-download.test.tsx`, and full frontend checks.

### Note editing and vault actions

**Kind:** product capability/adapter; safety-sensitive.

**Owned paths:**

- `frontend/src/api/writeApi.ts`
- `frontend/src/components/NoteEditor.tsx`
- `frontend/src/components/NoteActionsDialog.tsx`
- `frontend/src/hooks/useNoteActions.ts`
- `frontend/src/hooks/useNoteAutosave.ts`
- `frontend/src/hooks/useWriteMode.ts`
- `frontend/src/lib/blockOps.ts`
- `frontend/src/lib/caretMap.ts`
- `frontend/src/lib/editHistory.ts`
- `frontend/src/lib/imageUpload.ts`
- `frontend/src/lib/linePrefix.ts`
- `frontend/src/lib/sourceMap.ts`
- `frontend/src/lib/writeDrafts.ts`
- `frontend/src/lib/writePaths.ts`
- `frontend/src/components/note-page/BlockGap.tsx`
- `frontend/src/components/note-page/BlockInput.tsx`
- `frontend/src/components/note-page/EditableBlock.tsx`
- `frontend/src/components/note-page/InlineEditorProvider.tsx`
- `frontend/src/components/note-page/blockEditorSetup.ts`
- `frontend/src/components/note-page/SaveState.tsx`
- `frontend/src/components/note-page/attachmentDrop.ts`
- `frontend/src/components/note-page/autocomplete.ts`
- `frontend/src/components/note-page/conflictDiff.ts`
- `frontend/src/components/note-page/frontmatter.ts`
- `frontend/src/components/note-page/inlineEditorContext.ts`

**Public contract:** write capability discovery and operations, editor/action
components, note-action/write-mode hooks, local draft behavior, client path
validation, upload normalization, frontmatter editing, conflict display,
wikilink autocomplete, inline block editing (the editor provider/context, the
per-block wrapper, the CodeMirror block input and its markdown syntax
highlighting, click-to-write in the space between blocks, structural block
operations, document-level undo, autosave scheduling and save state), line
mapping between rendered nodes and file lines, and attachment acceptance and
insertion.

**Consumed dependencies:** shared API/types/UI, router navigation, vault tree
note candidates, and backend HTTP write endpoints.

**Coordination paths:** `App.tsx`, `NotePage.tsx`, `types.ts`,
`noteEnhancements.css`, backend `handlers/write_api.rs`, and
`vault/write/**`.

**Invariants:** expected content hashes remain part of update concurrency;
delete stays recoverable; client validation does not replace backend path
safety; every mutation continues through backend `vault/write` (ADR-03/11);
**nothing re-serializes a note** — edits replace only the lines a block owns and
reproduce the file's own line endings; **block operations refuse rather than
guess** when a range no block owns lies between them, or when the rendered tree
is still settling behind a wikilink resolve.

**Validation:** write API, editor, action dialog, upload, draft, path,
frontmatter, conflict, and autocomplete tests; `blockOps`, `sourceMap`,
`caretMap`, `editHistory`, `linePrefix`, `useNoteAutosave`, `attachmentDrop`,
`inlineEditing`, and `properties` tests; plus `App.write-mode.test.tsx`
and full frontend checks.

### Graph

**Kind:** product capability; suitable bounded dry-run candidate.

**Owned paths:**

- `frontend/src/components/graph/GraphPage.tsx`
- `frontend/src/components/graph/graphSimulation.ts`
- `frontend/src/styles/graph.css`

**Public contract:** `GraphPage`, graph simulation helpers, and the `/api/graph`
payload.

**Consumed dependencies:** shared API/error/types/UI, router navigation, and
`d3-force`.

**Coordination paths:** `App.tsx`, `types.ts`, backend graph wire types/handler,
and responsive CSS.

**Validation:** `GraphPage.test.tsx`, `graphSimulation.test.ts`, an App route
smoke test if routing changes, and full frontend checks.

### Statistics

**Kind:** product capability.

**Owned paths:**

- `frontend/src/components/StatsPage.tsx`
- `frontend/src/styles/stats.css`

**Public contract:** `StatsPage` and the `/api/stats` payload.

**Consumed dependencies:** shared API/error/types/UI and router links.

**Coordination paths:** `App.tsx`, `types.ts`, backend stats wire types/handler,
and responsive CSS.

**Validation:** add focused component coverage for behavioral changes, affected
route tests, and full frontend checks.

### Settings

**Kind:** product capability/adapter.

**Owned paths:**

- `frontend/src/features/settings/SettingsPage.tsx`
- `frontend/src/features/settings/settings.css`
- `frontend/src/features/settings/SettingsPage.test.tsx`

**Public contract:** the Settings page presents server-provided setting metadata
at `/settings`, keeps copy and section layout in the browser, confirms saves
that rebuild indexing, generates an MCP token candidate without persisting it,
reveals an MCP secret only when it grants the authenticated viewer no new
capability, PATCHes only the active section's changed keys to `/api/settings`
before replacing its state with the complete response, confirms local Git
initialisation and remote downgrades when the server requests it, and polls
`/api/index-status` plus `/api/git-status` for dedicated background progress
without using the startup gate.

**Consumed dependencies:** authenticated `apiFetch` and the settings HTTP
contract.

**Coordination paths:** `frontend/src/App.tsx` (route),
`frontend/src/app/ExplorerPane.tsx` (normal-deployment navigation),
`frontend/src/App.css` (stylesheet aggregation), `src/server.rs` (SPA/API
routes), and `src/handlers/settings.rs` (settings, index-status, and git-status
wire producer).

**Invariants:** demo mode exposes no Settings navigation or endpoints;
environment-managed and permanently unavailable values are records rather than
disabled form controls; secret values are never rendered from the settings
document.

**Validation:** `SettingsPage.test.tsx`, affected shell tests, frontend
typecheck, then full frontend checks.

### Shared UI and styling

**Kind:** shared infrastructure.

**Owned paths:** none by default.

**Paths:**

- `frontend/src/components/ui.tsx`
- `frontend/src/components/icons.tsx`
- `frontend/src/index.css`
- `frontend/src/App.css`
- `frontend/src/styles/base.css`
- `frontend/src/styles/topbar.css`
- `frontend/src/styles/ui-common.css`
- `frontend/src/styles/responsive.css`

**Contract and responsibility:** shared primitives, global tokens/base rules,
style aggregation, topbar/shell styles, and cross-feature responsive overrides.
`icons.tsx` holds the inlined Material Symbols (Sharp) set; icons size to `1em`
and paint with `currentColor`, so callers control them through font-size and
color. Attribution lives in `THIRD_PARTY_NOTICES.md`.

**Coordination rule:** a feature work packet should prefer its owned stylesheet.
Changes to shared selectors, tokens, or responsive rules must name affected
features. `App.css` remains an aggregation/composition stylesheet; feature
styles should migrate only as part of a declared boundary pilot.

**Validation:** affected component/App tests, responsive manual or screenshot
review when layout changes, and full frontend checks.

### Small shared browser utilities

**Kind:** shared infrastructure.

**Owned paths:**

- `frontend/src/lib/clipboard.ts`
- `frontend/src/lib/stateCompare.ts`

**Consumers:** shell copy actions and rendered code-block controls consume
clipboard behavior. Vault Explorer consumes tree comparison, while Note reading
consumes note and link comparison.

**Coordination rule:** keep these utilities behavior-only. Feature-specific
copy labels, workflows, or state ownership stay with their feature.

**Validation:** `clipboard.test.ts` and `stateCompare.test.ts`.

### Frontend test infrastructure

**Kind:** test infrastructure, not production ownership.

**Paths:**

- `frontend/src/test/setup.ts`
- all `frontend/src/**/*.test.ts`
- all `frontend/src/**/*.test.tsx`

Tests follow the production boundary they cover. Cross-feature `App.*` tests
belong to composition and must be run when their named integration changes.

## Auxiliary repository paths

These paths are outside the runtime module catalog and require separate work
packet scope:

- `Dockerfile` and `docker-compose.yml`: packaging/deployment.
- `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`: Rust build and dependency
  coordination.
- `frontend/package.json`, lockfile, TypeScript/Vite/ESLint configuration:
  frontend build and dependency coordination.
- `assets/**`: project branding and screenshots.
- `docs/**`: user, contributor, architecture, research, and roadmap
  documentation.
- `eval/**`: evaluation inputs and results coordinated with offline tooling.
- `scripts/**`: repository validation and maintenance tooling.

Dependency or build configuration is never implicitly owned by the module that
wants a new dependency.

## Full validation gates

Backend:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all
```

Frontend:

```bash
cd frontend
npm ci
npm run format:check
npm run lint
npm run typecheck
npm test
npm run build
```

Use focused tests during development. Run the full gates before merging a
boundary or interface change.
