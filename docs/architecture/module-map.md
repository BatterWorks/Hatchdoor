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
- `src/vault_runtime/tests.rs`
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
  and, on process shutdown, stops new work and waits only for active
  background-turn and foreground-mutation safe boundaries.
  `AppState::vault_registry`, `AppState::vaults`, and
  `AppState::legacy_migration_recovery` expose the authoritative definition
  store, activated per-Vault control blocks, and safe legacy-recovery state to
  later shared-core adapters. `AppState::vault_work` and
  `AppState::managed_git` expose the same background-work coordinator and
  managed-Git scheduler `run_server()` wires into the one dispatch loop, so an
  HTTP adapter (`handlers/vaults.rs`) can reconcile a registry mutation into
  live runtime effects and request an immediate Git or Index turn without a
  second execution lane. The same loop dispatches each Index turn through the
  Vault-qualified Markdown scan and disposable snapshot publisher; the
  runtime's watcher intents re-enter that coordinator rather than creating a
  separate indexing path. An Index turn publishes in two passes: a Vault's
  structural rows first (`VaultSearchStatus::Browsable`), then the same Vault
  again once its vectors exist (`Ready`), so browsing does not wait on
  embedding. The structure pass is skipped for a Vault that already has a
  searchable generation, which keeps search answering across a rebuild.
  `AppState::index_status` separately tracks setting-triggered rebuild drift,
  progress, ETA, and the last failure without changing startup readiness, while
  `AppState::runtime_config` supplies the immutable settings snapshot each
  reindex binds before it starts, including `HATCHDOOR_EMBED_LAYERS` for the
  per-Vault disposable candidate cache.
- `AppConfig` is the environment-derived deployment contract and interprets the
  live values from the startup `RuntimeConfig` snapshot. Its process-level
  Vault source is always the local `VAULT_PATH`; Git source identity and Git
  behavior belong only to registry Vault definitions. The removed,
  development-only `HATCHDOOR_VAULT_SOURCE`/`HATCHDOOR_VAULT_GIT_*` family is
  rejected explicitly rather than silently falling back to the local path.
- `StartupTracker` exposes startup/model/indexing readiness.
- `VaultRuntime` and its serialized snapshot expose only the process startup's
  local source/mode, lifecycle phase, and derived non-Git capabilities. Git
  source, mode, and capabilities are derived per Vault by
  `VaultCollectionRuntime`; the startup-status adapter is not collection
  authority and never serializes a managed-Git source or mode.
- `VaultCollectionRuntime` reconciles only newer registry snapshots into zero,
  one, or many Vault-ID-keyed `VaultControlBlock` values; an older asynchronous
  reconciliation cannot replace or re-admit work after a newer collection is
  live. During activation, it derives Ready or Stale search capability from a
  retained participating SQLite snapshot before its reconstructed Index turn
  runs; a missing, nonparticipating, or unreadable snapshot remains
  Unavailable. Each enabled block owns its
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
  acceptance, publishes its cancellation signal, and stops its watcher,
  including through already-held handles; retirement waits for both its active
  coordinator turn and any already-admitted foreground mutation to reach their
  safe boundaries. Unchanged Vaults retain their control blocks when another
  definition changes; disabled definitions remain visible with no capabilities
  and no active runtime.
- `ModelSetup` owns local model selection, terms acceptance, download integrity,
  and persistent setup records. Once the embedder is installed, startup queues
  each active Vault through the collection Index coordinator; it does not run a
  second legacy single-Vault cache build. Startup becomes Ready after every
  active collection Vault's Index turn settles Ready.
- `spawn_vault_change_watcher` reports Vault-ID-qualified change intent through
  an independently cancellable handle. `run_server()` coalesces those intents
  through the shared `VaultWorkCoordinator` as Index requests. The existing
  `spawn_vault_watcher` remains the transitional single-Vault adapter until
  later application-surface packets.
- `dispatch_managed_git_turn` is the `VaultWorkKind::Git` execution closure
  `src/server.rs`'s worker loop calls: for a `ManagedGit` Vault it resolves
  the current definition and credentials, obtains that Vault's checkout lease
  from `ManagedGitScheduler` (reused across turns for as long as the Vault
  stays active in this process — issue #95), holds
  `VaultControlBlock::acquire_mutation` for the duration, runs
  `git::run_managed_git_turn` off the async runtime via `spawn_blocking`,
  hands the lease back to the scheduler afterward, and publishes the result
  through `VaultControlBlock::set_git_status`/`set_local_content_status` and
  `ManagedGitScheduler::record_outcome`; a successful acquisition with usable
  local Markdown also requests that Vault's Index turn through the same
  coordinator. For an `ExistingGit` Vault in `PullOnly`/`TwoWay` mode it
  instead resolves credentials and runs `git::run_existing_git_remote_turn`
  off the async runtime via `spawn_blocking` against the checkout that
  already exists at the Vault's `repository_path` — no checkout lease: see
  the Git synchronization boundary below for why `ManagedCheckoutLease` does
  not apply to an already-existing, operator-owned checkout — but under the
  same `acquire_mutation` hold as the managed-Git path, so a foreground
  Markdown write can never race either kind of Git turn's working-tree
  phases. Both paths hold the mutation lock for the whole blocking turn
  (coarser than the legacy single-Vault task's fine-grained per-phase
  locking that releases across network-only fetch/push) rather than only
  across working-tree-mutating phases; splitting `synchronize_managed_checkout`
  into independently lockable phases to match that finer discipline was
  judged a materially larger change than issue #96's reopening warranted.
  `reconcile_and_reconstruct` activates or deactivates a scheduler-tracked
  Vault's `ManagedGitScheduler` entry (and, on deactivation, releases any held
  checkout lease) alongside its coordinator admission — `ManagedGit`, and an
  `ExistingGit` Vault in `PullOnly`/`TwoWay` mode (issue #132), both driven by
  the same `VaultSource::managed_git_poll_interval` accessor; an `ExistingGit`
  Vault in `LocalHistory` mode has no remote and is never registered.
  `set_local_content_status` (mirroring
  `set_search_status`/`set_git_status`) republishes authoritative
  local-content availability after a Git turn, since `activation_snapshot`
  only stats `vault_path` once, at `reconcile()` time, before a managed
  checkout exists. `activation_snapshot`'s Git status defaults to `Pending`
  (an immediate first sync) for a genuinely new Vault or a
  disabled-to-enabled transition only; `reconcile()`'s non-retained-
  definition branch (an in-place edit to an already-active Vault) instead
  carries the retiring control block's actual current Git status and error
  through to the replacement (issue #97's reopening findings 1/2 follow-up)
  — otherwise every edit, not just an identity change, would force `Pending`
  and trigger an unwanted immediate real Git turn, bypassing an armed
  backoff or any other real status.
- `dispatch_vault_index_turn` is the `VaultWorkKind::Index` execution closure
  the same worker loop calls. It acquires one Vault's refresh and foreground
  mutation boundaries, builds
  an authoritative Markdown index and isolated candidate cache off the async
  runtime, publishes a structure-only participating snapshot before vector
  embedding on a first build so browsing does not wait for semantic search,
  atomically publishes only that Vault's complete shared snapshot, and
  publishes Ready, Stale, or Unavailable search state without changing another
  Vault's snapshot or status.
  Disabled runtime state becomes externally nonparticipating immediately;
  reconciliation retires the corresponding disposable snapshot after admitted
  work reaches its safe boundary and before its mutation response completes:
  disable removes participation and disconnect deletes only that Vault's rows.
  A short reconciliation phase lock makes state application and immediate
  coordinator drain/activation decisions atomic across competing revisions,
  but is released before any safe-boundary wait. A retirement failure is
  returned through the mutation boundary rather than reported as ordinary
  lifecycle success.
  A current-revision retry or restart converges disabled participation and
  removes cached Vault IDs absent from the registry; an older revision fences
  itself before those cache side effects.

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
`VaultWorkCoordinator::has_work` is a read-only query over the same per-Vault
state `request` consults ("is `kind` currently active or already pending for
`vault_id`") — added for issue #97's reopening finding 1, so a caller that
must avoid adding a redundant queued turn can check first instead of
maintaining its own, independently trackable notion of "is this Vault busy."

**Consumed dependencies:** durable `VaultId` identity and Tokio notification.
The queue owns no Markdown, SQLite, Git, or lifecycle state.

**Consumers:** collection runtime reconstructs and drains work for lifecycle
transitions. `handlers/vaults.rs` reaches the coordinator only indirectly,
through `VaultCollectionRuntime::reconcile_and_reconstruct` after a registry
mutation, and directly through `ManagedGitScheduler::sync_now`/`retry_now` for
manual Git control and `VaultWorkCoordinator::request` for the one-Vault HTTP
refresh control — it never calls `drain_vault` itself. Runtime
composition dispatches Index through the Vault-qualified snapshot publisher
and Git through the managed-Git operation without additional execution lanes;
Repair remains separately owned.

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

**Owned paths:**

- `src/vault_registry.rs`
- `src/vault_registry/tests.rs`

**Public contract:** `DEFAULT_VAULT_REGISTRY_PATH`,
`REGISTRY_SCHEMA_VERSION`, canonical `VaultId` generation and parsing,
`VaultRegistryStore`, immutable `VaultRegistrySnapshot` values, explicit
`VaultRegistryState::Ready` versus `Recovery`, structured recovery/error
types, redacted `VaultDefinition` projections, tagged `VaultSource` values for
local, existing-Git, and managed-Git Vaults, `VaultGitMode`, credential write
inputs/updates, validated `add`/`edit`/`enable`/`disable`/`disconnect`
operations, store-owned `vault_path` resolution for runtime consumers,
a remote-backed source's own `poll_interval_secs` (issue #97's reopening
finding 2: per-Vault, not scheduler-wide; `#[serde(default)]`s to 24h so a
registry record written before this field existed keeps loading under the
same `REGISTRY_SCHEMA_VERSION`, and `add`/`edit` reject a value below 60s,
mirroring `git::managed_task::BACKOFF_MAX`). Issue #132 gives `ExistingGit`
the same field (also `#[serde(default)]`, also floor-checked in
`PullOnly`/`TwoWay` — unchecked and unused in `LocalHistory`, which has no
remote), alongside `ManagedGit`'s. `VaultSource::managed_git_poll_interval`,
the read accessor `ManagedGitScheduler`/`handlers/vaults.rs` use to consume it,
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

Issue #130 gives `VaultRecord`/`VaultDefinition` two further optional fields,
both `#[serde(default)]` so a registry written before they existed keeps
loading under the same `REGISTRY_SCHEMA_VERSION` (the `poll_interval_secs`
precedent above): an `archive_folder`, normalized to a single-trailing-slash
form (e.g. `"Archive/"`) and read by `VaultDefinition::archive_folder`, and a
`commit_identity` (`VaultCommitIdentity { name, email }`, not a secret —
unlike credentials it round-trips through every projection unredacted) read
by `VaultDefinition::commit_identity`. Both are absent by default; the
instance-wide `HATCHDOOR_ARCHIVE_PREFIX` setting and
`HATCHDOOR_GIT_AUTHOR_NAME`/`HATCHDOOR_GIT_AUTHOR_EMAIL` settings apply when
absent. The same issue makes `HttpsCredentials`' input username optional:
`normalize_credentials` substitutes the documented
`HTTPS_CREDENTIALS_USERNAME_PLACEHOLDER` constant when a caller supplies a
token alone, and validation now rejects only an empty token, not an empty
username.

**Consumers:** the legacy single-Vault import consumes the registry load, add,
and confirmed-empty initialization contracts. Runtime composition loads the
registry after migration and `VaultCollectionRuntime` consumes its safe
projections and resolved paths; its managed-Git Git-turn dispatch
(`dispatch_managed_git_turn`) is the one consumer of the crate-private
`https_credentials` accessor, and also resolves `commit_identity` through
`git::config::resolve_commit_identity` before every Git turn. `handlers/vaults.rs`
is the first HTTP consumer of the `add`/`edit`/`enable`/`disable`/`disconnect`
mutation contracts and of `load` for authenticated discovery, including its
explicit `Recovery` state. `ManagedGitScheduler::activate` (runtime
composition's reconcile loop) and `handlers/vaults.rs`'s manual sync/retry and
credential-replacement-retry controls consume
`VaultSource::managed_git_poll_interval`. `AppState::vault_archive_prefix`
(Vault mutation's three archive call sites: `handlers/vault_content.rs`,
`handlers/vault_write.rs`, `mcp/tools/write.rs`) consumes `archive_folder`,
falling back to `AppState::runtime_archive_prefix` when absent. MCP (its
`create_vault`/`edit_vault` tools reuse the same HTTP request/patch types),
frontend, cache, and search adapters remain separately owned later packets.

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
permanently suppresses legacy import. A safe import copies legacy exclusions,
Git behavior, credentials, and commit identity into the ordinary Vault
definition; the retired write-debounce value has no successor. Confirmed Start
with no Vaults writes an ordinary revisioned zero-Vault registry.

**Consumers:** startup runtime composition calls this isolated adapter before
opening the disposable cache and activating Vault runtimes. Safe imports become
ordinary enabled definitions; migration or environment-cleanup recovery activates no Vault and remains in
`AppState` for later setup/management surfaces.

**Coordination paths:** `src/lib.rs` exports the boundary; `src/server.rs`
turns migrated or now-ignored per-Vault environment keys into a restricted,
non-secret startup recovery response after a committed import or existing registry;
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
returns recovery. After a successful registry commit, non-empty
`HATCHDOOR_EXCLUDE` and `HATCHDOOR_GIT_*` environment values are named in a
restricted recovery UI until removed and the process is restarted; health and
the web shell remain reachable, but Vault runtime activation and mutation are
withheld. `VAULT_PATH` remains valid deployment configuration.
The development-only managed-startup variable family is rejected before it can
silently select another source. Markdown remains authoritative and downgrade
across the registry cutover is unsupported.

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
while MCP and MCP writes are enabled. Disabling MCP or MCP writes at runtime
immediately revokes that credential's attachment-upload capability; web-token
admission remains independent of MCP write mode.

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
defined here, including resolve, refresh, recent, stats, and graph shapes.
Endpoint-local wire types remain owned by their handlers, notably write types
in `handlers/vault_write.rs` and diagnostics types in
`handlers/diagnostics.rs`. `RefreshResponse` remains only for legacy internal
compatibility; #103 retires the scope-less MCP refresh tool alongside the HTTP
`/api/refresh` route. Index now has a production dispatch consumer, but this
packet adds no Vault-scoped refresh control; that remains separately owned.

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
exclusion types, `is_servable_asset`, and `seed_empty_vault`. `VaultIndex`
additionally carries an asset index (`asset_paths`, `assets_by_name`) filled by
the same walk that collects the Markdown files, and `resolve_asset` reads it:
Obsidian's default link format writes an attachment embed as a bare filename and
resolves it by searching the vault, so a purely note-relative reading broke every
embed in a vault using one top-level attachments folder (#158). `is_servable_asset`
is shared with `handlers/assets.rs`, so resolution can never name a path the
asset route would refuse.

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

**Coordination paths:** `src/handlers/vault_write.rs`,
`src/mcp/tools/write.rs`, Git write records, frontend write API/types, and
configuration for archive or upload limits.

**Invariants:**

- All HTTP and MCP mutations use this shared layer (ADR-03).
- Optimistic concurrency uses the expected content hash.
- Delete is recoverable trash; archive is move-based (ADR-11).
- Paths remain within the canonical vault root.
- Layer marker and excluded/noise writes remain protected at adapter and domain
  boundaries as applicable.
- **Known gap resolved by #103:** `vault_write.rs`
  serializes concurrent writes to one Vault through
  `VaultControlBlock::acquire_mutation` (a genuine per-Vault lock). MCP write
  tools now resolve one registered Vault and acquire that same control-block
  lock before calling `vault/write`; the older `AppState::vault_write_lock`
  remains only for the isolated legacy single-Vault Git-sync task.

**Validation:** `cargo test vault::write`, adapter write tests, and the full
backend checks.

### Vault-qualified read projections

**Kind:** product capability/domain core.

**Owned paths:** `src/vault_read.rs`.

**Public contract:** `VaultReadCore`, `BrowseSurface`, explicit `VaultScope`,
the common `VaultReadProjection` envelope, participant state/error types, and
Vault-qualified exact-note, tree, statistics, graph, and recent-note
projections. `BrowseSurface` names which layer surface a caller may read.
`Everything` is the established behavior and stays the default: a layer demotes
a Note from the default *search* surface only, and an operator still reaches it
by slug, in the explorer, and on the graph. `DefaultOnly` is demo mode's clamp
(#109), selected through `BrowseSurface::for_demo_mode` and applied by both
`VaultReadCore::on_surface` and `search::vault_scoped::VaultSearchCore::on_surface`:
a demo has no operator and no layer toggle, so a demoted Note is withheld from
exact reads, links, resolve, and download (as an ordinary not-found, so
withheld is indistinguishable from absent), and `BrowseSurface::restrict` drops
its rows from a published snapshot before any projection reads it, covering
tree, graph, recent, statistics, and a surviving search hit's outbound links. A
link is dropped when either endpoint is withheld, since a surviving edge would
name the hidden Note. `handlers/vault_collection_reads.rs` additionally clamps
the request's own `LayerSelection` so the `layers=` query is not an escape
hatch. `statistics_detail` (#137) is the exact-read counterpart to the
lean collection `statistics` projection: it returns `VaultQualifiedStats`
directly (never wrapped in `VaultReadProjection`, like `exact_note`), scoped
to exactly one Vault via `collection`'s `VaultScope::One` gating, computing
every legacy `VaultStatsResponse` field from the same published snapshot
`statistics`/`trees`/`graphs` read rather than the single-Vault-shaped SQL
cache tables `cache::queries::metadata::vault_stats` used (unreachable from
any production route since #101). `VaultScope` serializes as the flat scalar
`docs/migrations/vault-scoped-clients.md`'s envelope documents — the Vault
ID's canonical text for `One`, or the literal `"all"` — mirroring exactly what
a caller passes as the `scope` path segment, rather than serde's derived
externally-tagged shape. `resolve_wikilinks` resolves every target in a batch
against one
authoritative-index build, rather than one build per target. `resolve_batch`
generalizes it to note *and* asset targets over that same one build (#158),
taking the embedding note's Vault-relative directory because an asset target
resolves relative to the note that names it; assets are returned as
Vault-relative paths, since an asset has no slug. The browse surface does not
gate them: assets carry no layer, and an embed only resolves for a caller
already reading the note that contains it. `vault_directory`
resolves one Vault's local Markdown directory under the same
not-found/disabled/unavailable gating as exact reads (reusing
`VaultControlBlock::ensure_accepting_operations`, widened to `pub(crate)`,
rather than re-deriving that check), without building a full index, for
adapters that only need the path (contained asset/attachment/download
serving); it additionally confirms the directory exists on disk, since a
managed-Git Vault can be enabled and accepting operations before its checkout
has materialized, and reports that as the same retryable
`vault_read_unavailable` code an exact-note read's index build would rather
than a caller discovering an unrelated raw filesystem error later.
`exact_note_for_download` returns a Note together with its containing
directory from one Vault control-block fetch — required whenever a caller
needs both, since a concurrent Vault edit reconciles a *replacement* control
block rather than mutating the current one in place, so two independent
`exact_note`/`vault_directory` calls could otherwise observe different Vault
generations. The private `control_and_index` seam shares the
control-block-then-index-build sequence between `authoritative_index` and
`exact_note_for_download` so the two cannot diverge on identical failure
conditions.

**Consumed dependencies:** the Vault runtime's authoritative per-Vault index,
the shared cache's published Vault snapshot seam, and existing Vault note/link
types.

**Consumers:** `handlers/vault_content.rs` is the first HTTP consumer (exact
note/link/resolve reads and `vault_directory`). `handlers/vault_collection_reads.rs`
is the first HTTP consumer of the collection-read projections (`trees`,
`statistics`, `graphs`, `recently_modified`) — a thin adapter with no
collection-read domain logic of its own. Future MCP Vault-scoped adapters
remain a later consumer. The core has no adapter or route ownership.

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
- A Vault whose generation carries no vectors reads as
  `VaultParticipantState::NotSearchable` in semantic search only; browsing,
  keyword and tag reads use the same structural rows and report `Fresh`. It is
  never reported `Unavailable`, which would claim its Notes are missing rather
  than merely unembedded.
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
`ReadSnapshot` is the crate-private pinned-read seam used where participant
metadata and cache queries must observe one published generation.

**Consumed dependencies:** Vault IDs and index/types, chunking, embeddings, SQLite,
FTS5, and sqlite-vec.

**Consumers:** application state/reindexing, runtime composition's per-Vault
Index dispatch, Vault-qualified read projections, Search, handlers, MCP reads,
evaluation tooling, diagnostics, and the one-time legacy single-Vault
migration's read-only evidence check.

**Coordination paths:** `src/app_state.rs`, `src/vault_runtime.rs`,
`src/vault_read.rs`, `src/search/**`, `src/vault/index.rs`, `src/chunk/**`,
and embedder identity/dimensions.

**Invariants:**

- SQLite is rebuildable and never authoritative (ADR-01).
- Keep embedded SQLite, FTS5, sqlite-vec, WAL, one writer, and pooled
  query-only reads (ADR-06).
- Schema or embedder identity mismatch rebuilds rather than mixing data.
- A refresh commits a coherent new read snapshot.
- Shared semantic vectors have one embedder identity and dimension; a mismatch
  wipes the disposable cache before any partial rebuild can participate.
  The cache-wide model epoch covers snapshot and legacy builders, and stamps
  the shared identity atomically with snapshot participation.
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
`LayerSelection`, `LayerInfo`, `SearchResult`, `SearchResponse`, and `run`.
The Vault-qualified shared-core contract is `VaultSearchCore`,
`VaultSearchRequest`, `VaultSearchResponse`, and `VaultSearchResult`; it uses
the explicit `VaultScope` and common projection/participant envelope from the
Vault-read core without owning any HTTP, MCP, or frontend adapter.

**Consumed dependencies:** `SqliteCache`, its published Vault snapshot/cache
query seam, `Embedder`, the Vault collection runtime, the explicit Vault-read
scope/envelope, and vault metadata/types.

**Consumers:** the legacy HTTP search handler, `handlers/vault_collection_reads.rs`
(the first HTTP consumer of `VaultSearchCore::search`), MCP search/query
tools, offline evaluation runners, and future Vault-scoped MCP adapters.

**Coordination paths:** `src/api_types.rs`, `src/handlers/api.rs`,
`src/handlers/vault_collection_reads.rs`, `src/mcp/tools/read.rs`, cache query
methods, and frontend Search contracts.

**Invariants:**

- Runtime search defaults to pure semantic retrieval; hybrid and reranking stay
  offline (ADR-05).
- Layer selection and metadata filters must never widen the eligible result
  set.
- Vault-qualified search filters before ranking, globally ranks every usable
  Vault snapshot, caps by `(Vault ID, slug)`, and never deduplicates equal
  content or note names across Vaults. Staleness is participant status, not a
  relevance penalty.
- Participant metadata, note projections, and KNN/FTS hits for one search
  response come from one pinned SQLite generation.
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
errors (including `GitError::ManualRecovery` for repository operations that
cannot be proven Hatchdoor-owned), lifecycle status, repository operations,
`GitSyncHandle`, `SyncOps`, and `spawn_sync_task`. The crate-private
`parse_mode` and `non_empty_setting` helpers keep startup and one-time
migration interpretation identical. `resolve_commit_identity` (issue #130)
resolves one Vault's own configured `VaultCommitIdentity`
(`vault_registry.rs`) if set, else the instance-wide
`HATCHDOOR_GIT_AUTHOR_NAME`/`HATCHDOOR_GIT_AUTHOR_EMAIL` defaults; runtime
composition's `dispatch_managed_git_turn` calls it once per turn, before
dispatching to any of the branches below, so every commit this boundary makes
for a Vault — managed-Git, existing-Git remote-sync, or existing-Git
Local-history — honors that Vault's own identity. Local mode commits without
network access and discovers an enclosing existing checkout while staging only
the configured Vault subtree; remote mode retains the safe
fetch/integrate/push phases.
Automatic recovery requires a persisted Hatchdoor merge marker whose fresh
nonce remains bound to the current merge's Git-managed operation metadata and
whose index, Git-relevant tracked-worktree identity (content, type, and
executable status), and cleanup-sensitive merge metadata were predicted from
the immutable parent commits before the live merge began. The isolated
checkout supplies the worktree prediction; exact `MERGE_HEAD`, `MERGE_MODE`,
and full `MERGE_MSG` bytes are bound into the Active marker after adding the
nonce. A live merge must match those predictions before its marker becomes
Active, and every later reset or commit rechecks them; no post-merge
observation can become ownership evidence. Unknown, malformed, stale,
replayed, mismatched, chmodded, or manually changed operation state is
preserved for manual recovery. Local-history turns skip this recovery path
entirely rather than acting on state they did not create, and
`classify_local_history_error` reports an encountered `ManualRecovery` as the
non-retryable `existing_git_local_history_manual_recovery_required`.
The settings HTTP boundary owns the preflight → bounded drain → replacement
protocol and exposes it through `GET /api/git-status`.
`GitSyncHandle::stop` preempts debounce and returns `Ok` only after one
successful final drain. A timeout withdraws its stop request, and a failed
final drain keeps the old task/status active: either way the task continues
accepting records, and only a `stop` that truly returns `Ok` lets a caller
install a replacement task. Spawning a task flushes any
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
error. The uniquely selected managed remote and its push URL must remain the
configured credential-free HTTPS repository identity; unrelated remotes in an
operator-owned `ExistingGit` checkout are outside this boundary and untouched.
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
other remote failures. It takes a `&ManagedCheckoutLease` rather than acquiring
its own (issue #95): the process-lifetime ownership boundary the checkout
lease documents is held by the caller across every turn for a Vault, not
reacquired and dropped within each one. `ManagedGitScheduler` is one
process-wide instance — mirroring the coordinator's single-worker design, it
adds no per-Vault execution lane — that decides *when* to request a Vault's
next Git turn: that Vault's own configured `poll_interval_secs` (issue #97's
reopening finding 2 — previously one `poll_interval` shared by the whole
scheduler; `DEFAULT_POLL_INTERVAL`, 24h, is now only the fallback default a
Vault's registry record defaults to, mirrored by
`vault_registry::DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS`) after a success or
any non-retryable failure (including authentication, which never backs off —
it waits for a configuration change, a manual `sync_now`/`retry_now`, a
restart, or the normal schedule), or bounded exponential backoff after a
retryable (transient) failure. `activate(vault_id, poll_interval)` registers a
newly tracked Vault due immediately, or — for an already-tracked Vault — only
updates its stored interval in place, leaving an in-progress backoff or held
checkout lease untouched; `sync_now`/`retry_now` take the same `poll_interval`
so a manual control before a Vault's first turn still registers it correctly.
`tick()` skips a Vault whose Git turn is already active or already has a
pending rerun queued (via `VaultWorkCoordinator::has_work`) rather than
calling `request` unconditionally (issue #97's reopening finding 1): a Git
turn can outlast `DEFAULT_TICK_INTERVAL`, and requesting for an
already-active Vault would otherwise pre-queue a zero-delay rerun that fires
the instant that turn completes, before its outcome's backoff is armed —
defeating backoff on every retryable failure. This skip is scoped to
`tick()`'s own automatic due-check; `sync_now`/`retry_now` still coalesce a
manual request into the turn's one guaranteed rerun exactly as before.
`spawn_scheduler_tick` drives it on
`DEFAULT_TICK_INTERVAL`. `ManagedGitScheduler` also holds each active Vault's
`ManagedCheckoutLease` for that Vault's entire activation lifetime in this
process, through its crate-private `take_or_acquire_checkout_lease`/
`keep_checkout_lease` pair: the former returns an already-held lease or
acquires a fresh one, the latter hands a lease back after a turn so it stays
held (and its OS-level lock stays exclusive to this process) across turns
instead of being released at the end of each one. `deactivate` drops any held
lease, releasing the lock immediately for retirement, disable, disconnect, or
restart-reuse by a later process.

`run_local_history_git_turn` is an `ExistingGit` + `VaultGitMode::LocalHistory`
Vault's counterpart to `run_managed_git_turn`: given the Vault's already-resolved
path and commit identity, it builds its own placeholder `GitMode::Local`
`GitConfig` and calls `validate_local_repo` then `commit_local`, committing
only the contained Vault subtree of whatever enclosing checkout the Vault sits
in and never contacting a remote. It classifies every `GitError` into a
redacted `VaultWorkError`, mirroring the legacy single-Vault task's transient
split (`Remote`/`Other` retry; validation, conflict, and dirty-tree do not).
Unlike managed-Git Vaults, an `ExistingGit` Local-history Vault is never
registered with `ManagedGitScheduler`: it receives only the one `Pending`-
triggered Git turn `reconcile_and_reconstruct` already requests at activation,
with no ongoing re-commit-on-later-drift schedule.

`run_existing_git_remote_turn` is an `ExistingGit` + `VaultGitMode::PullOnly`/
`TwoWay` Vault's counterpart to `run_managed_git_turn` (issue #96's reopening
defect 1): it builds a `ManagedSyncConfig` directly from the Vault's
already-existing `repository_path`/resolved Vault path and calls
`synchronize_managed_checkout` against it — no `ManagedCheckoutLease`
acquisition: that machinery exists specifically for Hatchdoor-managed clones
into Hatchdoor-owned state directories tracked via a receipt file, and an
`ExistingGit` checkout is the operator's own pre-existing directory with
nothing to clone or track, the same reasoning that already applied to
`run_local_history_git_turn`. When the registry's `branch` is unconfigured
(`ExistingGit`, unlike `ManagedGit`, has no receipt-file-persisted resolved
branch and the registry does not require one for `PullOnly`/`TwoWay`), it
falls back to whatever branch is currently checked out at `repository_path`,
extending `validate_local_repo`'s Local-history "follows whatever branch the
operator has checked out" policy to the remote-sync target. It classifies
every `ManagedSyncError` through the same `classify_sync_error` table
`run_managed_git_turn` uses, now also carrying `DirtyWorkingCopy`/`Conflict`'s
affected paths and `LocalCommits`' count outward as structured
`VaultWorkErrorDetail` (issue #132), bounded and published as
`vault_runtime::VaultRuntimeErrorDetail` on `VaultRuntimeError`. Unlike
Local-history, an `ExistingGit` Vault in `PullOnly`/`TwoWay` mode *is*
registered with `ManagedGitScheduler` (issue #132) — it has a remote to poll
on a schedule, unlike Local-history's commit-only-on-local-drift turn.
For both managed and existing checkouts, `ManagedSyncConfig.repository_url` is
the remote identity: synchronization requires exactly one fetch remote whose
URL equals it, and uses that remote name for fetch, tracking refs, merge labels,
and push. Only that selected remote and its optional push URL are constrained to
the same credential-free HTTPS identity; unrelated operator-owned remotes in an
`ExistingGit` checkout are ignored and never contacted.

**Consumed dependencies:** local Git repository through `git2`, the live
configuration snapshot for startup parsing, and the registry's shared
credential-free HTTPS URL validator, `VaultId` identity, and the crate-private
`https_credentials` accessor (managed-Git turns only; never exposed further).

**Consumers:** server startup, write adapters, status handlers/tools,
`AppState`, and the one-time legacy single-Vault migration parser.
`ManagedGitScheduler`/`run_managed_git_turn` are consumed by runtime
composition (`src/vault_runtime.rs::dispatch_managed_git_turn` and
`reconcile_and_reconstruct`, which activates/deactivates a managed-Git Vault's
schedule alongside its coordinator admission). `dispatch_managed_git_turn_with`
obtains the Vault's checkout lease via
`ManagedGitScheduler::take_or_acquire_checkout_lease` before `spawn_blocking`,
passes it into the injected turn (`run_managed_git_turn` in production), and
hands it back with `keep_checkout_lease` once the turn completes, so the
lease survives across turns without being borrowed across the
`spawn_blocking` boundary — and by
`src/server.rs`, which
owns the one global consumer loop driving `VaultWorkWorker::run_next` — the
worker/scheduler-tick construction and dispatch this module map previously
noted as missing. `run_local_history_git_turn` is likewise consumed by
`dispatch_managed_git_turn_with`'s `ExistingGit` + `VaultGitMode::LocalHistory`
match arm, off the async runtime via `spawn_blocking`, publishing through the
same `publish_managed_git_turn_outcome` a managed-Git turn uses; that Vault is
never registered with `ManagedGitScheduler`, so this arm is its whole
Git-turn responsibility. `run_existing_git_remote_turn` is consumed the same
way by `dispatch_managed_git_turn_with`'s `ExistingGit` +
`VaultGitMode::PullOnly`/`TwoWay` match arm (issue #96's reopening defect 1):
no checkout lease, but the same `VaultControlBlock::acquire_mutation` hold
across `spawn_blocking` that the `ManagedGit` arm below now also takes
(defect 2), and publication through the same `publish_managed_git_turn_outcome`.
`VaultWorkKind::Index` is consumed by runtime composition's
`dispatch_vault_index_turn`, which publishes only that Vault's disposable
snapshot and reports its per-Vault search outcome; `Repair` remains an explicit
non-retryable "not yet implemented" `VaultWorkError` so a Vault's shared FIFO
position is not blocked ahead of its Git turn.

**Coordination paths:** `src/app_state.rs`, `src/server.rs`,
`src/vault_runtime.rs`, `src/vault_registry.rs` (crate-private
`https_credentials` accessor), `src/handlers/settings.rs`, HTTP/MCP write
adapters, configuration, frontend settings UI, and vault watcher Git
exclusions.

**Invariants:** optional and debounced; writes do not block on sync, except
while a managed-Git or `ExistingGit` remote-sync turn is in flight for that
Vault, see below; task replacement drains before another task can start;
local mode never contacts a remote; remote mode never force-checks out over
uncommitted manual vault edits (ADR-10). Managed acquisition never writes
credentials to URLs, Git configuration, reads, logs, errors, or status; it
never deletes, overwrites, or silently adopts a checkout destination. The
managed-Git scheduler adds no persisted queue, priority, or second execution
lane (ADR-13); a Git turn's returned failure always completes that Vault's
turn so the shared worker is released for the next Vault. A `ManagedGit` or
`ExistingGit` `PullOnly`/`TwoWay` Git turn holds `VaultControlBlock::acquire_mutation`
for its whole blocking duration (issue #96's reopening defect 2), so it can
never race a foreground Markdown write's own hold of the same lock — this is
the exception to "writes do not block on sync" above, scoped to exactly the
Vault whose turn is running; this is coarser than the legacy single-Vault
task's fine-grained per-phase locking (which releases across network-only
fetch/push), a deliberate trade favoring a small, low-risk diff over matching
that finer discipline. `commit_vault_drift` preserves an
operator's already-staged Vault-subtree index content across a Two-way
commit rather than overwriting it with working-tree drift (issue #96's
reopening defect 3), mirroring `sync.rs`'s `commit_working_tree`.

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
- `src/handlers/vault_collection_reads.rs`
- `src/handlers/vault_content.rs`
- `src/handlers/vault_write.rs`
- `src/handlers/vaults.rs`

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
edit/enable/disable/disconnect), manual Git sync/retry, one-Vault Index refresh,
and the collection-wide
SSE event stream — the first `/api/v1` surface. It is the first HTTP consumer
of the Vault collection registry's write operations and of
`VaultCollectionRuntime::reconcile_and_reconstruct`, which every mutation calls
in the same request through its foreground-mutation safe boundary; background
Index/Git draining may continue asynchronously, while a newly enabled Vault's
work is still requested without a separate reconciliation pass. Deliberately not gated by
`require_vault_ready` (a legacy single-configured-Vault signal): discovery and
creating the first Vault stay reachable at zero enabled Vaults, and discovery
reports an explicit `recovery` object rather than erroring when the persisted
registry itself needs operator recovery. Every response uses the shared
`VaultApiError{code, message, vault_id?, retryable}` shape and reuses
`vault_registry::VaultSource`/`VaultGitMode` directly on the wire rather than
duplicating them. MCP discovery is #103.

Discovery additionally reports `legacy_migration_recovery`
(`{code: "legacy_migration_required", message}`) when `AppState`'s
field of the same name — set once at startup from a failed safe legacy
import, `src/vault_migration.rs`'s `LegacyMigrationOutcome::Recovery` — is
still pending (#150). This is distinct from the sibling `recovery` field
above: `legacy_migration_recovery` means the registry itself loaded fine
(empty, revision 0) but automatic import could not prove the legacy
deployment, where `recovery` means the persisted registry file itself is
unreadable. Because a confirmed recovery action needs to clear this flag
without a restart, the field is `Arc<StdRwLock<Option<LegacyMigrationRecovery>>>`
rather than a plain `Option` fixed at construction.
`POST /api/v1/vaults/start-with-no-vaults` (`start_with_no_vaults_handler`)
is the confirmed recovery action: it requires `legacy_migration_recovery` to
be pending and a `{"confirm": true}` body, calls
`vault_migration::start_with_no_vaults` (an ordinary revision-0 commit,
refusing `registry_revision_conflict` if the registry already holds real
state), reconciles through `reconcile_after_commit` and responds through
`mutation_response` exactly like every other registry-mutating handler
here (so `state.vaults`'s own collection revision and the
`/api/v1/vaults/events` SSE stream never lag this commit, even though
today's transition is empty registry to empty registry), and clears the
flag on success. Demo-gated like every other collection-management route.

`edit_vault_handler` requests an immediate Git turn after a successful edit
whose `https_credentials` was `Replace` (issue #97's reopening finding 3):
`VaultDefinition`'s redaction-safe equality only ever compares
`credential_configured: bool`, never the credential value, by design — so a
replaced-but-still-configured credential leaves `reconcile()` retaining the
existing `VaultControlBlock` and never re-evaluating a prior authentication
failure. The trigger is source-agnostic (issue #132): any source with a
`VaultSource::managed_git_poll_interval` — `ManagedGit`, and an `ExistingGit`
Vault in `PullOnly`/`TwoWay` mode — retries through
`ManagedGitScheduler::retry_now` (tracked, self-registering, coalesces with
any pending turn); `Local` and an `ExistingGit` Vault in `LocalHistory` mode
carry no interval and are skipped. `edit_vault_handler` is the sole call site
of `VaultRegistryStore::edit`/`VaultDefinitionEdit` — including its MCP
`edit_vault` proxy, which calls this same handler — so this handler-level
trigger covers every path that can write `https_credentials`.

Discovery and the event stream are pure reads and stay reachable in demo mode
(#109: demo mode publishes every enabled Vault in the instance as a public
read-only collection, unlike `settings.rs`'s operator-controls posture, which
remains absent). Because that read is unauthenticated, discovery forks its
projection: demo mode lists only *enabled* definitions and builds each through
`public_vault_summary`, which withholds `source`, `exclude_patterns`,
`archive_folder`, `commit_identity`, and the four `*_error` details — the
deployment's absolute paths, tracked remotes, and operator configuration —
while keeping identity, the four independent statuses, `capabilities`, and
`credential_configured` (#133's designated credential signal). `source` is
therefore `Option` on the wire, always present on an authenticated read and
always absent on a demo one. Per-Vault `capabilities` are deliberately
unchanged in demo mode: #133 settles that the browser branches on the
instance-level `demo_mode` flag, not on a rewritten per-Vault capability. Collection management, manual Git sync/retry, and one-Vault
Index refresh are Vault-control operations, so `src/server.rs` wraps each of their routes —
individually, since some share a path with a read (`POST /api/v1/vaults`
alongside `GET`) — in `reject_demo_mutation`, which calls this file's
`demo_read_only_response` to refuse with a shared `403 demo_read_only`
`VaultApiError` before any registry mutation runs, rather than being absent.

`vault_content.rs` owns exact Vault-scoped content reads and their contained
resources, mounted in the same `/api/v1/vaults/{vault_id}/...` router group as
`vaults.rs` and sharing its demo-mode/auth posture, `VaultApiError` (including
its `new`/`respond` constructors, widened to `pub(crate)`), and
rejection-mapping helpers (`parse_vault_id`, `json_rejection_response`,
`query_rejection_response`, `internal_error_response`, widened to
`pub(crate)` for this reuse): `GET .../notes/{slug}`, `GET
.../notes/{slug}/links`, `GET .../notes/{slug}/download`, `GET .../resolve`,
`POST .../resolve-batch` (whose request additionally takes optional
`asset_targets` and `note_path`, answered by an `asset_results` array of
`{target, path}`, `path` null when nothing matched — additive, so a client
resolving note links only sees exactly what it saw before, and the batch cap
counts both target lists), `GET .../assets/{*path}` (serving both embedded
assets and imported attachments, which share one containment rule), and `GET
.../stats/detail` (#137's rich per-Vault statistics report). Every route but
the last always inspects the requested Vault's authoritative Markdown
directory through `VaultReadCore`, never the disposable cache; `stats/detail`
is the sole exception, reading the same published snapshot the collection
`{scope}/stats` route reads (word/mtime/size data `VaultReadCore`'s
authoritative-index path does not carry), so it can briefly lag a write the
way collection reads do, unlike every other route here. Exact reads run all blocking
filesystem/index work off the async runtime via `run_blocking` (one trip per
request, not one per batch entry or per path-resolution step), and are gated
per-request by that Vault's own
`vault_not_found`/`vault_disabled`/`vault_unavailable` status rather than any
single-configured-Vault readiness gate. Asset/download path resolution and response
shaping reuse `assets.rs`'s and `downloads.rs`'s existing containment, export,
and response-building logic (`resolve_asset_path`, `content_type_for_path`,
`asset_error_parts`, `asset_response`, `build_note_export`,
`download_response`, widened to `pub(crate)`, with `asset_response`/
`download_response` factored out of the legacy handlers' inline header-building
so both routes share one response shape), unchanged.

`vault_collection_reads.rs` owns one-or-all collection reads and search:
`GET /api/v1/vaults/{scope}/tree`, `.../recent`, `.../stats`, `.../graph`, and
`.../search`, mounted in the same router group and sharing the same
demo-mode/auth posture, `VaultApiError` shape, and `query_rejection_response`/
`vault_read_error_response` (the latter widened to `pub(crate)` in
`vault_content.rs` and extended with `invalid_search_query`/
`invalid_layer_selection` (`400`) and `search_unavailable` (`503`) arms for
reuse here) rather than duplicating them. `{scope}` reuses the path segment
name `vault_id` for router-tree consistency with every sibling route in this
group, parsed by this file's own `parse_vault_scope` into either a Vault ID or
`VaultScope::All`; anything else is the structured `invalid_scope` error
(`400`). This file is a thin adapter with no collection-read domain logic of
its own: `tree`/`stats`/`graph` return `vault_read.rs`'s existing
`VaultReadCore::{trees, statistics, graphs}` projections unchanged (grouped
per Vault); `recent` returns `recently_modified` (flattened across Vaults);
`search` returns `search::vault_scoped::VaultSearchCore::search`'s projection
(flattened, one global ranking). `search`'s `layers` query parameter is a
comma-separated token list parsed by this file's own `parse_layer_selection`
into a `LayerSelection` applied identically to every participant — unlike
`search::LayerSelection::parse` (built for the single-Vault MCP surface, where
an unrecognized token degrades to the default surface), it does not consult
any one Vault's known-layer catalog while parsing, since a name valid in one
Vault and absent from another is expected, not an error; only a name absent
from *every* usable participant is (`VaultSearchCore::search`'s own
`invalid_layer_selection` check).

`vault_write.rs` owns exactly-one-Vault Markdown mutations, attachment
upload, and write-capabilities discovery, retiring the entire legacy unscoped
application API in the same change (#101): `POST .../notes`, `PUT
.../notes/{slug}`, `PATCH .../notes/{slug}/rename|move|move-rename|archive`,
`DELETE .../notes/{slug}`, `POST .../attachments` (mounted separately from the
rest of this group so it can also accept a live MCP bearer token, mirroring
the retired `/api/attachment` route), and `GET .../write-capabilities`. It
calls unchanged `vault/write/**` functions exactly as the legacy write API
did, gates every mutation on the requested Vault's own control block —
`VaultControlBlock::acquire_mutation` (a genuine per-Vault lock, shared by the
MCP and HTTP write adapters; the legacy single instance-wide
`AppState::vault_write_lock` remains only for the legacy Git-sync task — see
the "Vault mutation" boundary's known-gap invariant) and
`capabilities.mutate` (a `capability_unavailable`/
`409` for a Pull-only or otherwise non-mutable Vault, new in #101 since the
legacy single-Vault write
API had no per-Vault mode to check) — and checks noise-exclusion against that
Vault's own `exclude_patterns` rather than the legacy instance-wide
`HATCHDOOR_EXCLUDE` setting. `VaultReadCore::control_block` and the free
function `runtime_error` (`vault_read.rs`) are widened to `pub(crate)` so this
file reuses the exact same not-found/disabled/no-runtime gate and error
mapping exact reads already use, instead of a duplicate copy. Archive prefix
and attachment size limit stay instance-wide settings (issue #62), read via
the same `AppState::runtime_snapshot`/`runtime_archive_prefix`/
`runtime_mcp_config` calls the legacy write API used. A mutation response
omits `git_sync_warning`: the managed-Git scheduler has no debounced-on-write
hook, unlike the legacy single `git_sync` task, so there is nothing per-write
to report that Vault discovery does not already expose. `vaults.rs` owns the
additive authenticated `POST /api/v1/vaults/{vault_id}/refresh` control: it
requires one enabled Vault with usable local Markdown, asks
`VaultWorkCoordinator` for `VaultWorkKind::Index`, and returns its immediate
`202 VaultScheduleResponse` acknowledgement (`queued` or `coalesced`) without
waiting for a snapshot build. It uses the shared `VaultApiError` conventions
for malformed IDs, missing/disabled Vaults, unavailable local capability, and
coordinator rejection; the migration guide documents external clients. The
legacy unscoped refresh and all-Vault refresh remain absent. `diagnostics`
remains retired because it needs new per-Vault cache-query domain methods.

Every route here is a content mutation, attachment upload, or write-capability
discovery, so `src/server.rs` wraps each one — individually, since Markdown
mutations share a path with a read (`PUT`/`DELETE .../notes/{slug}` alongside
`GET`) — in `reject_demo_mutation` (#109): in demo mode it refuses with
`vaults.rs`'s shared `403 demo_read_only` error before any mutation runs,
unlike `vault_content.rs`'s exact reads and `vault_collection_reads.rs`'s
one-or-all reads, which are pure reads and stay reachable in demo mode.

`vault_write.rs` binds one live configuration snapshot *before* consuming any
attachment multipart field and reads each field incrementally against its
fail-closed byte limit, so lowering the limit takes effect on the next request
rather than after the bytes are already buffered; an invalid pinned upload
limit never falls back to a larger default. A `WriteError` carrying recovery
guidance is reported as `write_recovery_required` rather than collapsing into
the sanitized generic internal error. `vault_content.rs` bounds Vault asset
and generated note-download responses so these convenience endpoints are not
unbounded transfer buffers; an over-limit asset or export receives the shared
`VaultApiError` shape and `413 Payload Too Large`.

**Consumed dependencies:** `AppState`, HTTP wire types, vault reads,
`vault/write`, Search, cache queries, Git status, auth, and — for `vaults.rs`
only — the Vault collection registry's mutation/load operations,
`VaultCollectionRuntime::{snapshot, reconcile_and_reconstruct,
subscribe_revisions}`, `VaultWorkCoordinator`, and
`ManagedGitScheduler::{sync_now, retry_now}` and `VaultWorkCoordinator::request`
via `AppState::{vault_work, managed_git}`. `vault_content.rs` is the first HTTP consumer of
Vault-qualified read projections (`vault_read.rs`'s `VaultReadCore`, including
its `vault_directory` accessor); `vault_collection_reads.rs` is the first HTTP
consumer of that core's collection-read projections and of
`search::vault_scoped::VaultSearchCore`. `vault_write.rs` is the first
consumer of `VaultControlBlock::acquire_mutation` and of
`VaultReadCore::control_block`/`vault_read::runtime_error` outside
`vault_read.rs` itself.

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
`mcp_get_handler`/`mcp_post_handler`. `list_vaults` exposes the shared redacted
Vault discovery/status/capability and revision shape. Every collection read
names `scope` (one Vault ID or `all`); every exact read, Markdown mutation, and
existing-Vault control names `vault_id`. Revisioned registry management uses
the same shared collection shapes as HTTP; `create_vault` is the only zero-ID
exception because the registry atomically generates its immutable ID. MCP
returns shared domain failures as structured error tool results. No
scope-less/default/sole-Vault tool remains reachable.
`get_attachment_import_config` names one Vault and answers under every write
posture, reporting the instance-wide write switch and that Vault's own
mutation capability as separate fields rather than refusing the call.
`list_note_attachments` is a read tool on the read catalogue, reachable without
MCP write permission and without the mutation capability. `create_vault` and
`edit_vault` advertise the `VaultSource` and credential contracts as
per-variant schemas rather than opaque objects; `edit_vault` replaces a
definition wholesale, and only its credential patch preserves a stored value
across an edit.

Each MCP request validates its live configuration, token, and Origin before
the body is collected. Read-only MCP accepts only the small ordinary JSON-RPC
request bound; write-enabled requests may use the current base64-attachment
allowance plus bounded JSON framing. Invalid pinned attachment limits fail
closed rather than widening to defaults. JSON-RPC replies are also bounded; an
oversized reply becomes a bounded protocol error rather than an unbounded
response buffer. Because the read tools delegate to the `/api/v1` handlers,
those handlers' own blocking-work offload covers the tool path too.

**Consumed dependencies:** `AppState`, the Vault discovery/content/collection
read/write HTTP adapters as shared contract producers, Vault registry/runtime,
Search, `vault/write`, model setup, attachment limits, and the live
configuration snapshot bound at each request.

**Coordination paths:** `src/server.rs`, domains exposed as tools, and
documentation describing agent behavior.

**Invariants:** MCP is disabled by default, uses its own token, validates
Origins, and keeps read-only access credentialed (ADR-09). Token changes,
write enablement, Origins, and attachment limits apply to the next request;
attachment authorization never retains a rotated MCP token. Write tools use
`vault/write`, the requested Vault's `acquire_mutation` lock, and retain
optimistic concurrency and path protections (ADR-03).

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
- `frontend/src/app/vaultSlot.tsx`
- `frontend/src/app/vaultSlotLogic.ts`
- `frontend/src/app/vaultAccordion.ts`
- `frontend/src/app/constants.ts`
- `frontend/src/hooks/useIsMobile.ts`
- `frontend/src/hooks/useTheme.ts`
- `frontend/src/hooks/useVaultScope.ts`
- `frontend/src/lib/storage.ts`
- `frontend/src/components/StartWithNoVaultsDialog.tsx`

**Contract and responsibility:** bootstraps React/router/PWA, composes feature
hooks and routes, owns responsive shell state, navigation, persistent shell
preferences, topbar actions, and explorer placement. Before the tree ever
renders, `main.tsx` calls `lib/writeDrafts.ts`'s `collectLegacyHeldDrafts`
and `lib/storage.ts`'s `clearLegacyNoteScopedBrowserState` (#151) once,
synchronously — the one-time post-#137 sweep and browser-state cleanup, so
every component's first render already reflects them regardless of which
route mounts first. `clearLegacyNoteScopedBrowserState` removes Recent
notes, the last note opened, unfolded explorer folders, and explorer scroll
position — state that named a note or folder before Vault qualification and
cannot be trusted to mean the same one after — guarded by the persisted
`LEGACY_BROWSER_STATE_CLEARED_KEY` marker so it never repeats over state a
returning user has legitimately rebuilt since; six Vault-agnostic
preferences (theme, sidebar width, drawer open state, Recent notes'
collapsed state, the touch-edit hint, the stored bearer token) are untouched.
`useVaultScope.ts` owns
the selected Vault scope (state/storage, per #137), Vault discovery
(`useVaultDiscovery`), and the Vault-less-action default
(`resolvePrimaryVaultId`). `app/ExplorerPane.tsx`'s Scope zone (#138) calls
`setScope` on the desktop; `app/AppTopbar.tsx`'s scope row and its bottom
sheet (#145) call it below 920px, where the Scope zone itself does not
render. The breakpoint keeps the two callers mutually exclusive — every other
collection-read and Vault-picking call site only reads the selected scope.
`vaultSlot.tsx`/`vaultSlotLogic.ts` (#139) derive each Vault's trailing
count-or-condition slot and the shared All-Vaults/collapsed-head aggregate
from `VaultSummary`'s status fields alone — no new endpoint.
`vaultSlotLogic.ts`'s `deriveVaultSlot` is also imported by Note reading's
`NotePage.tsx` (#141) to detect a write-blocking Git condition on the open
note's own Vault; this is a deliberate cross-capability import of one pure
function rather than a duplicated copy of the condition vocabulary.
`useVaultScope.ts`'s
`useVaultNoteCounts` is the one exception to "state/storage only": it fetches
the lean collection-scope `GET /api/v1/vaults/all/stats` (always at `"all"`,
independent of the browsing scope) to feed the slot's healthy-count reading,
gated on more than one enabled Vault and refetched on the same
`vaultRevision` `useVaultTree` already tracks rather than opening a second
SSE subscription. The topbar's `Tree Stale` badge is deleted (#139) with
nothing replacing it; `Offline` is the only condition left there, because it
is about the workspace and not about any one Vault.
`app/vaultAccordion.ts` (#142) is `app/ExplorerPane.tsx`'s per-Vault
accordion under `all`: pure derivation for the landing default (the open
note's own Vault, else the last persisted, else nothing), the unavailable-
Vault unfold gate, the `LAST_UNFOLDED_VAULT_KEY` persistence pair, and the
per-Vault namespacing of the shared `expandedFolders` record the accordion's
folder-open memory needs. Unfolding a Vault never calls `setScope`, same
invariant as the Scope zone's own narrow-scope call being the only one.

The Scope zone renders at zero enabled Vaults too, not only above one
(#150): `All Vaults` holds its place with no rows beneath it, in neutral
ink, rather than disappearing along with the last Vault. It remains absent
at exactly one enabled Vault where narrowing has nothing to offer, except
while first-run startup progress needs its slot. Its
collapsed-head and `All Vaults`-row slots also take an optional
`startupProgress` (`StartupProgress`, exported from this file) that
replaces the ordinary aggregate while the shrunk startup gate reports
`scanning`/`indexing`, reusing the per-Vault "indexing" slot's animated-bar
visual language. `App.tsx`'s `"/"` route similarly branches on genuine zero
Vaults (a neutral `Add a Vault` empty state; the action itself is Settings'
`VaultCreationDialog` (#153) — this route has no room for the flow, so
`ZeroVaultState`'s `onAddVault` navigates to `/settings` with
`{state: {openVaultCreation: true}}` instead, absent entirely in demo mode via
the `demoMode` prop threaded down from this hook — a demo instance's own
description reads "This demo has no Vaults loaded." in that state (#152),
never the ordinary "Add a Vault…" sentence with nothing left to act on it)
versus a broken start:
`useVaultScope.ts`'s `useVaultDiscovery`
now also exposes `recovery` (the persisted registry file is unreadable) and
`legacyMigrationRecovery` (the registry loaded fine but a failed safe
legacy import needs recovery) — mutually exclusive, both rendering the same
documented error block with a `Try again` action (a plain re-fetch), and
`legacyMigrationRecovery` additionally offering `components/StartWithNoVaultsDialog.tsx`'s
once-confirmed `Start with no Vaults` action against the new
`POST /api/v1/vaults/start-with-no-vaults` endpoint.

A demo instance is a faithful Hatchdoor with the operator removed, not one
with its controls greyed out (#152): `App.tsx`'s `"/settings"` route renders
`<Navigate to="/" replace>` instead of `SettingsPage` whenever `demoMode` is
true — silently, like every other withheld operator affordance, rather than
disabled-and-explained — which is what makes Vault management, `Add a
Vault`, Git behaviour/credential controls, `Sync now`, and `Unsaved drafts`
disappear together in one place instead of needing separate gates inside
`features/settings/**` (whose own vault-scoped reads stay reachable at the
API layer per #109, but are never rendered for a demo visitor). The route
first checks `vaultsLoading` (the same guard the `"/"` route already applies
below): `demoMode` starts `false` until Vault discovery's fetch resolves, so
without that guard a demo visitor opening `/settings` directly — a bookmark,
a shared link, a reload on that route — would see `SettingsPage` begin
mounting for one frame before flipping to the redirect. The sidebar footer's
own Settings link (`settingsEnabled`, passed to `app/ExplorerPane.tsx`) takes
the identical `!vaultsLoading && !demoMode` guard for the same reason: gating
on `!demoMode` alone would leave that link live and clickable for the whole
discovery fetch, not just one frame, since nothing else in the shell blocks
on `vaultsLoading` the way the `"/"` route's own content does. Everywhere a
Vault's
condition slot renders — the Scope zone, its collapsed head, the mobile
scope row/sheet (`AppTopbar.tsx`), the explorer accordion, and each graph
island caption (`GraphPage.tsx`, below) — `vaultSlotLogic.ts`'s
`deriveVaultSlot`/`deriveVaultAggregate`/`describeScopeSlot` take an optional
trailing `demoMode` parameter (default `false`) that clamps every condition
to the amber tier and swaps the Vault's own runtime message for the
instruction-free fallback sentence: nobody browsing a public demo is the one
who would act on an operator diagnostic, and the red tier's bordered ground
is reserved for something the app is not already handling. `VaultSlot`/
`VaultAggregateSlot` (`vaultSlot.tsx`) take the same optional `demoMode` prop
and thread it straight through, as does `App.tsx`'s own `describeScopeSlot`
call for the shell's scope live region. The one write-blocking use of
`deriveVaultSlot` outside a rendered slot — `NotePage.tsx`'s
`writeBlockReason` escalation (Note reading and rendering, below) — also
takes `demoMode`: the escalation banner itself still renders in demo mode
(an honest signal, same as every other Vault condition staying visible), but
never repeats the Vault's own operator-facing Git diagnostic to a visitor
who was never going to attempt the save it warns about.

**Coordination rule:** feature work may touch `App.tsx` only when the work
packet names the route, callback, shortcut, or state integration. A large prop
surface is a coordination seam, not permission to move feature behavior into
the shell.

**Validation:** the applicable `App.*.test.tsx` (including
`App.demo-mode.test.tsx`, #152), `app/ExplorerPane.test.tsx`,
`app/AppTopbar.test.tsx`, `app/vaultSlot.test.tsx`, `useVaultScope.test.ts`,
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
- `frontend/src/startup/useStartupStatus.ts`
- `frontend/src/styles/startup.css`

**Public contract:** `StartupGate` (a pure, prop-driven presentational
component — it no longer polls itself) and `useStartupStatus`, the shared
hook that polls `/api/startup-status` and owns the model-setup actions
(accept/decline Gemma, retry). Production `App.tsx` resolves Vault discovery
before enabling this polling, so broken-registry and zero-Vault workspaces
never poll or gate; it passes the resulting discovery plus startup
`status`/`retryModelSetup` to its internal `VaultWorkspace` composition and
the gate inputs to `StartupGate` (#150: the gate
shrinks to exactly the `terms_required`/first-`downloading` model step —
`hasSteppedPastGate` latches true the first time any other state is
observed and never re-arms, so a later retry-triggered `downloading` never
reopens the full-screen gate). Every other state — `scanning`, `indexing`,
`ready`, `failed`, and anything registry- or zero-Vault-related the gate
never observed in the first place — renders the ordinary workspace, which
reads the same `status` for its own surfaces: `app/ExplorerPane.tsx`'s Scope
zone slot (`StartupProgress`) and `features/search/SearchDialog.tsx`'s
work-in-flight/failed-model blocks.

**Consumed dependencies:** shared API client and theme hook.

**Coordination paths:** `App.tsx`, `app/ExplorerPane.tsx`,
`features/search/SearchDialog.tsx`, backend startup/model setup handlers and
types, and shell-wide styles.

**Validation:** `StartupGate.test.tsx`, `useStartupStatus.test.ts`,
`App.startup-auth.test.tsx`, and full frontend checks.

### Vault explorer

**Kind:** product capability.

**Owned paths:**

- `frontend/src/components/Explorer.tsx`
- `frontend/src/components/ChangesPanel.tsx`
- `frontend/src/hooks/useVaultTree.ts`
- `frontend/src/lib/folderPaths.ts`
- `frontend/src/lib/noteCandidates.ts`
- `frontend/src/lib/notePath.ts`
- `frontend/src/styles/layout-explorer.css`

**Public contract:** `useVaultTree`, explorer tree/list components, derived
folder paths, and flattened note candidates. The sidebar is three zones — a
fixed rail, a scrolling nav, a fixed footer — and `.explorer-nav` is the scroll
container the shell restores scroll position against, not the pane itself. On
desktop with more than one enabled Vault, the shell-owned Scope zone (#138,
`app/ExplorerPane.tsx`) pins a fourth zone above the rail; it shares this
file's CSS but is not part of this capability's owned React contract.
`ChangesPanel` lists notes changed on disk; it deliberately carries no unread
count, because distinguishing external changes from the user's own edits needs
backend data that does not exist yet. Recently viewed and Changed on disk both
carry the shared `VaultPrefix` provenance marker (#140) on each row when scope
is `all` and more than one Vault is enabled; a single-Vault instance renders
unchanged. `useVaultTree` also exposes `modifiedNotesPartial` and
`modifiedNotesMissingVaults` from the `/recent` read's own envelope (#141);
`ChangesPanel` never banners a partial read — a trailing warn-ink line below
the last row names only the missing Vaults, and `StateBlock tone="error"`
replaces the empty state outright when nothing is usable. The tree read's own
`partial` (`treePartial`) is deliberately left untouched: #116 rules grouped
surfaces (tree, graph, statistics) show a missing Vault as a visible missing
group, which belongs to #142/#143, not this flattened-list rule.
`useVaultTree` additionally exposes `vaultTrees` (#142): every participating
Vault's own tree, grouped rather than merged, straight off the `/tree`
read's own per-Vault array. The existing merged `tree` (via `mergeVaultTrees`)
is unchanged and still what narrowed-scope and single-Vault-instance
rendering use; `vaultTrees` exists only to feed the shell's per-Vault
accordion under `all` (`app/vaultAccordion.ts`, `app/ExplorerPane.tsx`).
`lib/notePath.ts`'s `pathToNoteIdentity` (moved out of `Explorer.tsx` to
resolve a lint rule against non-component exports from a component file) is
consumed the same way by both: `Explorer.tsx`'s own active-path folder
highlighting, and the shell's landing-Vault resolution, which needs it
synchronously off the URL rather than waiting on `activeNote`'s own content
fetch.

**Consumed dependencies:** shared API/error utilities, shared wire types,
shared UI components (`components/ui.tsx`'s `VaultPrefix` and `StateBlock`),
`lib/vaultParticipants.ts`, and router links.

**Coordination paths:** `App.tsx`, `app/ExplorerPane.tsx`, `types.ts`,
`lib/stateCompare.ts`, `lib/vaultParticipants.ts`, responsive CSS, and backend
tree/recent/event endpoints.

**Validation:** folder/note-candidate/state comparison tests and affected App
navigation tests; `app/ExplorerPane.test.tsx` covers the tree and list
components in composition, including the single-active-highlight invariant.
`hooks/useVaultTree.test.ts` covers the `/recent` read's partiality at three
and eight Vaults; the rest of the hook still needs focused coverage.

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
`SearchDialog` takes `vaults`/`scope` and shows the shared `VaultPrefix`
provenance marker (#140) on a result's path line under the same all-scope,
multi-Vault condition Vault Explorer's lists use; the path itself elides
head-first (`.result-path-text`) so the never-eliding prefix always reads.
`useSearch` also exposes `searchPartial`/`searchMissingVaultNames` from the
search envelope (#141), rendered with the same never-a-banner rule
`ChangesPanel` uses: a trailing warn-ink line naming only the missing Vaults
below the last result, or `StateBlock tone="error"` replacing "No matching
notes" outright when nothing is usable. Ranking is unchanged either way.

`SearchDialog` also carries its own Vault filter (#144) — a lens over the
answer in front of you, never the browsing scope, per #119's rule the
component structurally cannot violate (it has no `onScopeChange` prop at
all). `useSearch` exposes the raw `searchParticipants` (feeding per-Vault
facet counts) and `searchInitialVaultFilter` (pre-fills the filter from a
tag tap via `openSearchForTag(tag, vaultId)`, cleared the moment the dialog
closes). The filter itself is local `useState` inside `SearchDialog`, not
lifted to `useSearch` — it dies for free because `App.tsx` only mounts
`<SearchDialog>` while `searchOpen` is true, so the component remounts
fresh on every open. Two shapes, one meaning: a `.search-facet-rail` column
beside the results on desktop (absent when scope is narrowed or at one
enabled Vault), and a `.search-field-strip` `Scope`-beside-`Mode` pair
(§18's field grammar) that replaces the desktop Mode checkbox below 920px —
both rendered unconditionally and toggled by the same CSS breakpoint
`responsive.css` already uses, so no `isMobile` prop crosses the boundary.
Filtering is a client-side `Array.filter` over the already-fetched results;
no re-fetch, no re-ranking.

`SearchDialog` also takes `startupStatus`/`onRetryModelSetup` (#150), the
shrunk startup gate's own data (`startup/useStartupStatus.ts`): while
`scanning`/`indexing`, the result area shows a work-in-flight block
carrying the same percentage the Scope zone shows, with the query input
left enabled and the topbar's search entry point never greyed; on a failed
model download it shows the reason with a "Retry setup" action instead of
the ordinary empty/error states. Both replace the normal loading/error/empty
rendering only — the facet rail and results list underneath are unaffected
(harmlessly empty, same as any other no-data state).

**Consumed dependencies:** shared API/error utilities, shared UI components
(`components/ui.tsx`'s `VaultPrefix` and `StateBlock`), the shared
`.field`/`.field-label`/`.field-input` grammar (`App.css`),
`lib/vaultParticipants.ts`, router navigation supplied by the shell,
`startup/useStartupStatus.ts`'s status shape, and backend Search.

**Coordination paths:** `App.tsx`, `App.css`, `NotePage.tsx` (tag taps hand
`openSearchForTag` this note's own Vault id), `startup/useStartupStatus.ts`,
backend search HTTP contract, and responsive CSS.

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
wikilink resolution — `useResolvedWikilinks` now sends embed and PDF targets to
`resolve-batch` as `asset_targets` alongside the note's own path and rewrites
them to the resolved Vault-relative path (#158), keeping the note-relative
reading as the fallback for anything unresolved, including the first render
before the batch returns; its asset cache is keyed by note path as well as
target, because the same filename in notes at two depths can be two files — heading/search-hit navigation, Markdown transformations,
note navigation/rendering behavior, the editable-block component map produced by
`createNoteMarkdownComponents`, the paragraph marker `CalloutOrQuote` uses to
recognise its own first child, and the soft-break splitter that reconstructs one
source line per rendered line for the two unit types addressed per line.
A TOC click, mobile heading jump, or search deep link arms `NotePage`'s
`tailArmed` state, rendered as `data-tail` on the article; `styles/note-content.css`
reads it to add trailing scroll space only for that jump, so a heading near the
end of a note can reach the top of the pane. It resets when the note changes
and otherwise stays armed for the rest of the visit, since removing the space
would clamp `scrollTop` and pull the heading back down.
`NoteProperties` (`note-page/sections.tsx`) takes an optional `vaultName`
(#140): a synthetic, non-editable leading `Vault` row, shown whenever more than
one Vault is enabled regardless of scope — an exact read is never ambiguous
about its own Vault — including when the note carries no frontmatter at all,
which is the one case the grid renders with zero real properties. A note that
fails to load renders `StateBlock tone="error"` (#141) — the documented red
heading, not the plain empty shell "Note Unavailable" used to share with
"Not Found". `NotePage` also imports `deriveVaultSlot` from the
shell-owned `app/vaultSlotLogic.ts` (#141) to detect the open note's own
Vault being git-`unavailable` with a `dirty_working_copy`/`git_content_conflict`
condition: escalation is triggered by the write attempt, not by the
condition alone, so a stopped or conflicted Vault shows `SaveState`'s
`Not saving` and a full-bleed `.write-notice` before a save is ever
attempted (autosave's own `enabled` flag is gated on the same check), while
every other non-healthy condition — or trouble in a Vault that is not the
open note's — raises nothing here. `NotePage`'s `onTagSelect` prop is
`(tag, vaultId) => void` (#144): it wraps the raw `onTagSelect={tag =>
onTagSelect(tag, vaultId)}` when calling `<NoteProperties>`, handing Search
this note's own Vault id so a tag tap pre-selects it in the dialog's filter
— tags are per-Vault vocabularies. `NoteProperties`'s own prop to
`TagChips` is untouched. `NotePage` also reads a `?restoreEdit=1` query
param (#151): held-draft recovery in Settings seeds this note's ordinary
`lib/writeDrafts.ts` draft slot, navigates here with the marker, and
`NotePage` opens the editor the same way its own Edit button would (calling
the same `startEditing`, which reads that draft), then strips the marker via
a `replace` navigation so a refresh does not reopen it. Independently, a
dismissible (per view, not persisted — it returns on every load until the
last held draft is dealt with) notice above the note body names any drafts
`lib/writeDrafts.ts`'s `listHeldDrafts` reports and links to Settings;
ordinary post-#137 per-note draft recovery is unaffected. That notice is
additionally suppressed whenever `demoMode` is true (#152), regardless of
`listHeldDrafts`: it names and links to a Settings surface withheld from a
demo visitor entirely, and a pre-#137 held draft could in principle exist in
any browser profile a demo instance happens to be served from. `NotePage`'s
`Vault` property row (`NoteProperties`'s `vaultName`, above) is a name only
— it carries no condition slot, so #152's demo-mode amber clamp on
`deriveVaultSlot` has nothing to touch there; the one other `deriveVaultSlot`
call here, `writeBlockReason`'s write-blocking Git escalation, takes an
optional `demoMode` prop (#152) for the same instruction-free-sentence clamp
(Application shell and navigation, above) — the escalation banner itself
still renders, since it stays honest about Vault trouble independent of
whether editing is offered, but never repeats the Vault's own operator
diagnostic. `handleSave`'s catch and `handleBodyDrop`'s attachment-upload
catch both take the optional `onDemoRefusal` prop (#152, Note editing and
vault actions), checked first — `handleSave` falls back to its existing
`ConflictError`/generic-error branches on a miss, `handleBodyDrop` to its
existing generic `onWriteNotice` fallback.

**Consumed dependencies:** API/auth helpers, router state, Markdown/rendering
libraries, shared types/UI, note editing (including its held-draft recovery
model, #151), and `app/vaultSlotLogic.ts`'s `deriveVaultSlot` (Application
shell and navigation).

**Coordination paths:** `App.tsx`, `types.ts`, `app/vaultSlotLogic.ts`,
note/link/resolve/download handlers, `NoteEditor.tsx`,
`features/settings/UnsavedDrafts.tsx` (the `?restoreEdit=1` contract), Search
query navigation, shared and responsive CSS.

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

**Validation:** note-page unit tests, `NotePage.test.tsx` (write/read
escalation), Markdown/heading/search/state tests,
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
insertion. `lib/writeDrafts.ts`'s `HeldDraft`/`listHeldDrafts`/
`discardHeldDraft`/`collectLegacyHeldDrafts` (#151) are the recovery model
for drafts that predate Vault qualification, consumed by Settings'
`UnsavedDrafts.tsx`; ordinary per-note and create drafts
(`saveNoteDraft`/`loadNoteDraft`/`clearNoteDraft`/`saveCreateDraft`/
`loadCreateDraft`/`clearCreateDraft`/`pruneNoteDrafts`) are unchanged.
`hooks/useNoteActions.ts`'s `openCreateDialog` takes an optional second
`targetVaultId` parameter (#151) so a caller outside the currently open note
— draft recovery — can pin which Vault a note is created in, overriding
`resolvePrimaryVaultId`'s inference for that one dialog session.

`hooks/useWriteMode.ts` needs no demo-mode branch of its own (#152):
`GET .../write-capabilities` carries the same `demo_guard` layer every
mutation route does (`src/server.rs`'s route registration, grouped with
mutations "since it is write-capability discovery, not content browsing;
gated the same as the mutations it describes"), so the request 403s with
`demo_read_only` in demo mode before the handler's own `enabled` field is
ever computed, and this hook's existing catch already resolves
`writeEnabled` to `false` — every affordance it gates (New note, Edit,
attachment drop) is already absent with no client-side clamp needed.
`writeApi.ts` exports `DEMO_READ_ONLY_CODE`/`isDemoReadOnlyError`, reading
the `code` every write error now carries (`parseError` returns `{message,
code}` rather than a bare string) so a demo refusal can be told apart from
every other write failure. `App.tsx`'s `handleDemoRefusal` is the
defense-in-depth backstop for a write that reaches the server anyway: one
app-authored sentence into the shared `.write-notice` strip (never the
server's own message, and never the generic inline failure state a note
action's dialog or the editor would otherwise show), plus a fresh
`loadVaults()` call — "the app re-asks the server what it is permitted to
do" — and no retry affordance. It is threaded into `useNoteActions.ts`'s
five write handlers through one shared `handleDemoRefusal` closure local to
that hook (checked first in each catch block via `if (handleDemoRefusal(error))
return;`; closes the action dialog on a hit rather than leaving it open —
extracted rather than repeated five times once the fifth call site made the
duplication real, not premature); into `NotePage.tsx`'s `handleSave` catch
(exits editing rather than showing `ConflictError`'s or a generic error's
inline banner); into its `handleBodyDrop` attachment-upload catch (Note
reading and rendering, above); into `NoteEditor.tsx`'s own `uploadEditorFile`
catch via a new `onDemoRefusal` prop `NotePage.tsx` passes straight through,
so a demo refusal on an in-editor attachment drop or paste clears the
editor's own inline `attachmentNotice` rather than showing it there; and into
the block-editor autosave `save` callback `useNoteAutosave` wraps (`NotePage.tsx`
sets a local `autosaveDemoRefusal` flag on a hit, rethrows so the hook still
halts autosave for the rest of this note session the same as any other
failure, and that flag suppresses only the generic "could not reach the
vault" banner the hook's own `"error"` status would otherwise show —
`SaveState`'s terse "Not saving" pill is untouched, since it names no
message and carries no instruction either way).

**Consumed dependencies:** shared API/types/UI, router navigation, vault tree
note candidates, and backend HTTP write endpoints.

**Coordination paths:** `App.tsx`, `NotePage.tsx`, `types.ts`,
`noteEnhancements.css`, `features/settings/UnsavedDrafts.tsx` (consumes the
held-draft model and `openCreateDialog`'s target-Vault override), backend
`handlers/vault_write.rs`, and `vault/write/**`.

**Invariants:** expected content hashes remain part of update concurrency;
delete stays recoverable; client validation does not replace backend path
safety; every mutation continues through backend `vault/write` (ADR-03/11);
**nothing re-serializes a note** — edits replace only the lines a block owns and
reproduce the file's own line endings; **block operations refuse rather than
guess** when a range no block owns lies between them, or when the rendered tree
is still settling behind a wikilink resolve.

**Validation:** write API (`writeApi.test.ts`, including the demo_read_only
code-carrying cases), editor, action dialog, upload, draft, path,
frontmatter, conflict, and autocomplete tests; `blockOps`, `sourceMap`,
`caretMap`, `editHistory`, `linePrefix`, `useNoteAutosave`, `attachmentDrop`,
`inlineEditing`, and `properties` tests; `useNoteActions.test.tsx` (#152);
plus `App.write-mode.test.tsx`, `App.demo-mode.test.tsx` (#152), and full
frontend checks.

### Graph

**Kind:** product capability; suitable bounded dry-run candidate.

**Owned paths:**

- `frontend/src/components/graph/GraphPage.tsx`
- `frontend/src/components/graph/graphSimulation.ts`
- `frontend/src/styles/graph.css`

**Public contract:** `GraphPage`, graph simulation helpers (including the
island layout primitives `computeIslandCenters`, `buildIslandGraphs`, and
`createIslandSimulation` — #143), and the `/api/v1/vaults/{scope}/graph`
payload. Under `all` with more than one participating Vault, every Vault's
component is laid out on its own and placed as a labelled, dash-enclosed
island on one shared canvas (one zoom, one pan); at zero or one participating
component — including a single-enabled-Vault instance under `all` — the page
is byte-identical to the narrowed single-Vault graph (#118's resolution).

**Consumed dependencies:** shared API/error/types/UI, router navigation,
`d3-force`, `useVaultDiscovery` (Vault-management order, `demoMode`, and
per-Vault condition, reused via `deriveVaultSlot(vault, count, demoMode)` for
each island's caption so a demo instance's islands clamp to the amber tier
the same as every other Vault chrome, #152), and `describeVaultsNotDrawn`
from `lib/vaultParticipants.ts`.

**Coordination paths:** `App.tsx`, `types.ts`, backend graph wire types/handler,
`app/vaultSlotLogic.ts`, `lib/vaultParticipants.ts`, `hooks/useVaultScope.ts`,
`test/fixtures/vaults.ts`, and responsive CSS.

**Validation:** `GraphPage.test.tsx`, `graphSimulation.test.ts`, an App route
smoke test if routing changes, and full frontend checks.

### Statistics

**Kind:** product capability.

**Owned paths:**

- `frontend/src/components/StatsPage.tsx`
- `frontend/src/styles/stats.css`

**Public contract:** `StatsPage` and the
`GET /api/v1/vaults/{vault_id}/stats/detail` payload (#137; the legacy
unscoped `/api/stats` this section previously cited was retired in #101).

**Consumed dependencies:** shared API/error/types/UI and router links.

**Coordination paths:** `App.tsx`, `types.ts`, backend stats wire types/handler,
and responsive CSS.

**Validation:** add focused component coverage for behavioral changes, affected
route tests, and full frontend checks.

### Settings

**Kind:** product capability/adapter.

**Owned paths:**

- `frontend/src/features/settings/SettingsPage.tsx`
- `frontend/src/features/settings/VaultSettingsIndex.tsx`
- `frontend/src/features/settings/VaultSettingsIndex.test.tsx`
- `frontend/src/features/settings/vaultGitBehavior.ts`
- `frontend/src/features/settings/VaultCreation.tsx`
- `frontend/src/features/settings/VaultCreation.test.tsx`
- `frontend/src/features/settings/vaultCreation.ts`
- `frontend/src/features/settings/UnsavedDrafts.tsx`
- `frontend/src/features/settings/UnsavedDrafts.test.tsx`
- `frontend/src/features/settings/relativeTime.ts`
- `frontend/src/features/settings/settings.css`
- `frontend/src/features/settings/SettingsPage.test.tsx`

**Public contract:** the Settings page presents a two-level Vault-management index
from `GET /api/v1/vaults`, including disabled Vaults only in Settings, and each
selected Vault's condition, editable definition fields, identity facts, and
revisioned pause/rebuild/disconnect controls through the existing Vault API.

A git-backed Vault's own page (issue #149, resolving #121) carries one
segmented Git-behaviour control offering the four behaviours legal on a
folder Hatchdoor did not clone (`local`/`existing_git`: No Git, Local
history, Pull-only, Two-way) or the two legal on one it did
(`managed_git`: Pull-only, Two-way) — illegal options are absent, not
greyed. The plaque above it states the folder's source kind as a fixed
identity fact and, once the behaviour requires a remote, gains an
affordance that opens its repository, branch and folder
(`vault_subdirectory`) lines into fields; a Vault's own `repository_path`
disk location is never itself editable here. Every change that would alter
`same_source_identity` (`src/vault_registry.rs`) — crossing the No-Git/Git
boundary, or editing repository/branch/folder — runs one refuse-then-confirm
round trip: a confirmation modal, then a client-orchestrated
disable→PATCH(`confirm_identity_change: true`)→enable sequence. A failed
disable or a failed edit is rolled back by re-enabling and reporting nothing
changed; a failed final enable leaves the Vault paused with a persistent
red-line recovery state (a `hatchdoor:vault-recovery:{vaultId}`
`localStorage` marker, since the registry has no "wanted enabled but
couldn't" flag of its own) shown on both the Vault's own page and its
management entry in the index, each carrying one recovery button that
re-enables with a freshly fetched revision. Sign-in is one no-sign-in/access-
token control with no separate Remove; the token field is always empty and
its state reads `saved`, `none`, or (the instant an identity field changes)
`will be cleared`. The sync schedule is a 1–1440-minute field (client-side
bounded; the registry enforces only a 60s floor) defaulting to 1440,
shown whenever the drafted behaviour is remote-backed — this resolves #148's
outstanding AC4: the legacy `HATCHDOOR_GIT_DEBOUNCE_SECONDS`
local-edit-to-commit debounce has no per-Vault successor (the multi-Vault
pipeline already coalesces writes through `vault_watcher.rs`'s fixed,
non-configurable watcher debounce), so that concept is retired rather than
folded into this field; the schedule field answers a different question —
how often to poll a remote for incoming changes — which is the only new
per-Vault timing control this page adds. A live sync console
(shown whenever the Vault's own `git` status is not `"disabled"`) carries a
`Sync now`/`Try again` button calling `POST .../sync` or `.../retry`, and
renders one of nine failure sentences off `git_error.code` (plus an
unrecognised-code fallback) — the two carrying an affected-file list
(`managed_git_dirty_working_copy`, `managed_git_conflict`) render it from
`git_error.detail`'s `affected_paths` data, not from the message string.
This page owns all of this wording itself; the server sends only codes
(matching this page's existing reindex/Git-init confirmation copy).
`vaultGitBehavior.ts` holds every pure helper above (behaviour derivation,
identity comparison, failure-code copy, the recovery marker) split out of
the component file because a file exporting non-component values breaks
Fast Refresh (`react-refresh/only-export-components`).

Switching `mode` alone (a behaviour swap that stays within
`local`/`existing_git`'s three Git modes, or `managed_git`'s two) never needs
the Vault disabled or `confirm_identity_change`, since `mode` and
`poll_interval_secs` sit outside source identity. This page also presents
server-provided setting metadata at `/settings`, keeps copy and section
layout in the browser, confirms saves that rebuild indexing, generates an MCP
token candidate without persisting it, reveals an MCP secret only when it
grants the authenticated viewer no new capability, PATCHes only the active
section's changed keys to `/api/settings` before replacing its state with the
complete response, confirms local Git initialisation and remote downgrades
when the server requests it, and polls `/api/index-status` plus
`/api/git-status` for dedicated background progress without using the
startup gate.

When `GET /api/v1/vaults` reports `recovery` (the registry file itself is
unreadable, #150), `VaultSettingsIndex.tsx` replaces its whole `Vaults`
group with the same documented error block `App.tsx`'s note-pane shows,
omitting `Add a Vault`; `This server` is a separate group and keeps working.
`legacyMigrationRecovery` is deliberately not surfaced here — the registry
loads fine (empty) in that case, so the group renders its ordinary
zero-Vault state.

`VaultCreation.tsx`'s `VaultCreationDialog` (issue #153) is the one creation
flow both `Add a Vault` entry points open: the settings index's own button
here, and the zero-Vault workspace state's button in `App.tsx`, which has no
room for the flow itself and instead navigates to `/settings` carrying
`{state: {openVaultCreation: true}}`, consumed once by `SettingsPage.tsx` (via
`useLocation`, cleared with `navigate(..., {replace: true})` so a later
back/forward visit does not reopen it) and threaded down as
`VaultSettingsIndex`'s `autoOpenCreation` prop. The dialog collects a name,
one source configuration, and an `exclude_patterns` list (issue #157) via
`vaultGitBehavior.ts`'s shared `parseExcludePatterns` — also now used by the
edit flow's own field instead of a second inline `split(",")` normalizer —
sent in the initial `POST` (omitted when empty, relying on the server's
default, the same convention `credentials` already used) so the first
admitted Index turn observes it rather than waiting on a later edit-flow
`PATCH`. It also reuses `vaultGitBehavior.ts`'s
`behaviorOptions`/`buildSourceForBehavior`/`withIdentityFields` unchanged —
the same two-step composition the edit flow already uses, starting from an
empty `local` or `managed_git` source instead of an existing Vault's — so a
brand-new Vault's Git behaviour is chosen with the identical four-or-two-option
control the edit page presents. `vaultCreation.ts` holds the pieces specific to
creation: `baseSourceForKind`, `validateCreateSource`, and the
`POST /api/v1/vaults` call itself, fetching a fresh `expected_registry_revision`
immediately before submitting (the same pattern `recoverPausedVault` already
uses) rather than trusting a value read whenever the dialog opened. On success
the dialog calls back with the created `VaultSummary`: `VaultSettingsIndex`
appends it to its own list, calls the optional `onVaultCreated` prop (wired to
`App.tsx`'s `discovery.loadVaults`, so the sidebar/scope zone/explorer also
learn about the new Vault without a reload — the settings index's own list is
a separate fetch from that app-wide discovery), and opens the new Vault's own
page the same way clicking it in the index does. A registry-revision conflict
and every structured API failure render as a form-level notice without
clearing entered fields; a credential token is held only in the dialog's own
React state, never logged, never echoed back, and simply omitted from the
request body when no sign-in is chosen. Demo mode removes the button in both
entry points (`VaultSettingsIndex`'s own `demo_mode` read, and `App.tsx`
passing its `demoMode` down as `ZeroVaultState`'s `demoMode` prop) — belt and
suspenders alongside the invariant below, since the zero-Vault state renders
on a route demo visitors can otherwise reach.

`UnsavedDrafts.tsx` (#151) is a second "This server" nav entry, shown only
while `lib/writeDrafts.ts`'s `listHeldDrafts()` returns at least one draft
recovered from before Vault qualification (#137): a pre-#137 note draft was
keyed by slug alone, and the standalone create draft has never carried a
Vault. Each row lets the operator pick a destination Vault (pre-filled only
at exactly one enabled Vault), then Restore or Discard independently — no
batch action, since drafts need not share a Vault. Restoring a note draft
checks the destination Vault for a note at that slug before acting: found,
it seeds that Vault's ordinary `lib/writeDrafts.ts` per-note draft slot and
navigates to `NotePage` with `?restoreEdit=1`, which `NotePage.tsx` (#151)
reads once to open the editor the same way its own Edit button would, then
strips the marker; not found, the row offers a different Vault or restoring
the text as a new note (preserving the standalone create draft's own
`folder`, or an empty folder for a recovered note draft, which carries only
a slug) through `OpenCreateDraft`, a typed callback `UnsavedDrafts.tsx`
exports and `App.tsx` supplies: it seeds the standalone create draft and
calls `hooks/useNoteActions.ts`'s `openCreateDialog` with an explicit target
Vault ID (its second, optional parameter, added for this ticket) rather than
the Vault `resolvePrimaryVaultId` would otherwise infer from the currently
open note. A restored note draft keeps its own `baseContentHash` — the
version it was actually typed against — rather than the destination note's
current hash, so `NotePage.tsx`'s existing stale-draft comparison still
fires correctly. `relativeTime.ts`'s `formatWhen` (accepting either an ISO
string or an epoch-ms number) is the one relative-age ladder both this
section's draft rows and the Git/index status console share. The section is
a migration artefact, not a standing feature: it withdraws for good once the
last held draft is discarded or restored. `NotePage.tsx` separately shows a
dismissible (per view, not persisted) notice above the note body — naming
only that drafts are held and linking to this section, not repeating this
section's own explanation of what was cleared — whenever any held draft
exists; ordinary post-#137 per-note draft recovery (returning to a note and
clicking Edit) is unchanged. The one-time sweep that populates held drafts
(`collectLegacyHeldDrafts`) and the one-time removal of note- or
folder-naming browser state it cannot trust across Vault qualification
(`lib/storage.ts`'s `clearLegacyNoteScopedBrowserState` — Recent notes, the
last note opened, unfolded explorer folders, explorer scroll position; six
Vault-agnostic preferences are left untouched) both run once, synchronously,
in `main.tsx` before the app ever renders, so every component's first read
already reflects them.

Out of this page's scope: giving a Vault a source it did not start with (its
first repository, i.e. a Local Vault becoming `managed_git`, or a bare
first-run Vault) is the separate first-run flow (#122), not a field this page
edits. `docs/design/design-system.html` is not documented against this
ticket's primitives — on this branch it predates even #120's Settings work
and has diverged from `development`'s own (also incomplete) copy; treated as
separate, pre-existing design-system documentation debt rather than in scope
here.

**Consumed dependencies:** authenticated `apiFetch`, the settings HTTP
contract, and (for `UnsavedDrafts.tsx`) `lib/writeDrafts.ts`'s held-draft
functions.

**Coordination paths:** `frontend/src/App.tsx` (route; also supplies
`vaults` and `onOpenCreateDraft` to `SettingsPage`, and seeds the standalone
create draft before opening the dialog), `frontend/src/app/ExplorerPane.tsx`
(normal-deployment navigation), `frontend/src/App.css` (stylesheet
aggregation), `frontend/src/components/NotePage.tsx` (`?restoreEdit=1`
handling and the held-drafts notice), `frontend/src/hooks/useNoteActions.ts`
(`openCreateDialog`'s target-Vault override), `frontend/src/main.tsx` (runs
the one-time legacy sweep and browser-state cleanup before rendering),
`src/server.rs` (SPA/API routes), `src/handlers/settings.rs` (settings,
index-status, and git-status wire producer), and `frontend/src/types.ts`
(`VaultSource`/`VaultGitMode`, mirroring `src/vault_registry.rs`'s
same-named types, and `VaultSummary`'s `source` field, now typed rather than
`unknown`; consumed by this section and by
`frontend/src/app/vaultSlotLogic.ts`, already listed under Vault chrome's
own `types.ts` coordination entry).

**Invariants:** demo mode exposes no Settings navigation or endpoints;
environment-managed and permanently unavailable values are records rather than
disabled form controls; secret values are never rendered from the settings
document; a held draft is deleted only through an explicit Restore or
Discard, never aged out.

**Validation:** `SettingsPage.test.tsx`, `VaultSettingsIndex.test.tsx`,
`VaultCreation.test.tsx`, `UnsavedDrafts.test.tsx`, affected shell tests
(`App.startup-workspace-states.test.tsx` covers the zero-Vault entry point),
frontend typecheck, then full frontend checks.

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
The tokens in `base.css` are governed by
[`docs/design/design-system.html`](../design/design-system.html), which is
authoritative for visual decisions across every feature stylesheet; a component
the system does not yet cover gets its section added by the change that ships
it. `icons.tsx` holds the inlined Material Symbols (Sharp) set; icons size to
`1em` and paint with `currentColor`, so callers control them through font-size
and color. Attribution lives in `THIRD_PARTY_NOTICES.md`. `VaultPrefix` (#140) is
the one marked-path-root primitive every flattened, scope-spanning surface
uses for Vault provenance — hot ink, a middot instead of a folder `/`, and
never eliding; consumers give the adjacent title or path the shrinking room
instead. `StateBlock` (`ui.tsx`) takes an optional `tone="error"` (#141) for
the documented §23 red-heading variant — a genuine failure, never the plain
empty shell — consumed wherever a partial collection read has nothing usable
and wherever an exact read fails outright.

**Coordination rule:** a feature work packet should prefer its owned stylesheet.
Changes to shared selectors, tokens, or responsive rules must name affected
features. `App.css` remains an aggregation/composition stylesheet; feature
styles should migrate only as part of a declared boundary pilot.

**Validation:** affected component/App tests, responsive manual or screenshot
review when layout changes, `python3 docs/design/palette.py` when a `base.css`
accent or token changes, and full frontend checks.

### Small shared browser utilities

**Kind:** shared infrastructure.

**Owned paths:**

- `frontend/src/lib/clipboard.ts`
- `frontend/src/lib/stateCompare.ts`
- `frontend/src/lib/vaultParticipants.ts`

**Consumers:** shell copy actions and rendered code-block controls consume
clipboard behavior. Vault Explorer consumes tree comparison, while Note reading
consumes note and link comparison. `vaultParticipants.ts` (#141) — a
`VaultReadProjection`'s `participants` down to the Vaults that did not answer
fresh, and the shared "X did not answer." sentence — is consumed by Vault
Explorer (`ChangesPanel`) and Search (`SearchDialog`), the two flattened
collection surfaces a `partial` read can span.

**Coordination rule:** keep these utilities behavior-only. Feature-specific
copy labels, workflows, or state ownership stay with their feature.

**Validation:** `clipboard.test.ts` and `stateCompare.test.ts`.

### Frontend test infrastructure

**Kind:** test infrastructure, not production ownership.

**Paths:**

- `frontend/src/test/setup.ts`
- `frontend/src/test/fixtures/vaults.ts`
- all `frontend/src/**/*.test.ts`
- all `frontend/src/**/*.test.tsx`

Tests follow the production boundary they cover. Cross-feature `App.*` tests
belong to composition and must be run when their named integration changes.
`test/fixtures/vaults.ts` (#137) is the shared multi-Vault fixture set every
later slice's tests assert against: one, three, and eight Vaults, and a
builder for each non-healthy per-Vault condition (indexing, stale, sync
failed, sync stopped, conflict, unavailable) plus the collection-read
envelope/participant shapes.

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
