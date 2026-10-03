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
  to place in `.env`. Per-Vault variables left in the environment
  (`HATCHDOOR_EXCLUDE`, `HATCHDOOR_GIT_*`) are named in one startup warning;
  they no longer hold the instance in a restricted recovery mode (#427).
- `AppState` carries shared runtime state. Every field has a production
  reader, and each is one of: collection runtime (`vault_registry`, `vaults`,
  `vault_work`, `managed_git`, `startup_sqlite`, `embedder`,
  `runtime_embedder`, `mcp_tools_changed`), startup or posture
  (`model_setup`, `model_setup_started`,
  `web_auth_enabled`, `demo_mode`, `startup`), live configuration
  (`runtime_config`), folder listing (`vault_mount_root`, the configured
  `VAULT_PATH`), instance state (`instance_versions`, the version record
  `run_server` takes once from `instance_state` after
  `vault_migration::prepare_registry`, with whether the install existed read
  before that step can write a registry, so a refused start records nothing,
  and `agent_connections`, the last MCP client log loaded from the same store, written by `mcp/adapter.rs` and read
  by `handlers/settings.rs`), or process lifecycle (`shutdown`).
- `ShutdownSignal` (`AppState::shutdown`) fires once when the process starts
  shutting down. `server.rs` stops accepting on it, and every response that
  would otherwise stay open forever ends on it: the collection events stream
  in `handlers/vaults.rs`, MCP `subscriptions/listen` in `mcp/adapter.rs`, and
  legacy MCP sessions through rmcp's cancellation token in `mcp/routes.rs`.
  Graceful shutdown waits for every open connection, so a new long-lived
  response must end on it too (#353).
- `VaultCollectionRuntime` reconstructs disposable background turns at startup
  and, on process shutdown, stops new work and waits only for active
  background-turn and foreground-mutation safe boundaries.
  A `VaultControlBlock` also owns that Vault's `git::WriteLedger`, the one
  per-Vault handle both the mutation core and its Git turn already hold
  (#249). An in-place definition edit rotates the control block, and the
  ledger moves to the replacement alongside `prior_git`: its records describe
  writes already on disk and still uncommitted, so the rotation must not drop
  them. It also owns the Vault's `git::NoteHistory` (#300), which does not
  move on rotation: a definition edit may point the Vault at another
  repository, so the replacement starts with an empty history cache.
  It also keeps the Vault's built link graph between links reads (#361),
  through `VaultControlBlock::linked_index`. The graph does not move on
  rotation either, so no graph outlives the definition (path, exclude
  patterns) it was scanned under, and a disabled or disconnected Vault's
  revoked block refuses before it reaches the graph. The block subscribes to
  the collection's change channel and drops the graph on any report for its
  Vault, a lagged receiver included. That channel carries both the watcher's
  debounced report of an outside edit and the mutation core's
  `report_write`, sent before a write's response, so a Hatchdoor write shows
  on the next links read and an outside edit once the watcher reports it (up
  to `WATCH_MAX_DEBOUNCE`). A Vault whose watcher is not running, or a
  collection that does not watch, never keeps a graph and builds one per
  links read; replacing a watcher moves an epoch that retires any graph kept
  under the old one.
  It also keeps the `IndexedAssets` (catalogued asset paths and `LayerMap`)
  its latest Index turn scanned (#377), set by the Vault work executor through
  `retain_indexed_assets` and read by `VaultReadCore::asset_on_surface` through
  `indexed_assets`, so a demo's asset check does not walk the Vault per
  request. It does not move on rotation, so it never answers for a different
  path or exclude patterns, and a fresh block has none until its first Index
  turn scans. Unlike the graph it ignores the watcher epoch: a checkout
  cloned again under the same block is answered from the old scan until the
  next Index turn, and a file gone from disk is refused before the check.
  `AppState::vault_registry` and `AppState::vaults` expose the authoritative
  definition store and activated per-Vault control blocks to later shared-core
  adapters. `AppState::vault_work` and
  `AppState::managed_git` expose the same background-work coordinator and
  managed-Git scheduler `run_server()` wires into the one dispatch loop, so an
  HTTP adapter (`handlers/vaults.rs`) can reconcile a registry mutation into
  live runtime effects and request an immediate Git or Index turn through the
  same coordinator lanes. The same loop dispatches each Index turn through the
  Vault-qualified Markdown scan and disposable snapshot publisher; the
  runtime's watcher intents re-enter that coordinator rather than creating a
  separate indexing path. An Index turn publishes in two passes: a Vault's
  structural rows first (`VaultSearchStatus::Browsable`), then the same Vault
  again once its vectors exist (`Ready`), so browsing does not wait on
  embedding. The structure pass is skipped for a Vault that already has a
  searchable generation, which keeps search answering across a rebuild. A
  long Index turn takes turns with other Vaults (ADR-35): after
  `INDEX_TURN_SLICE` of embedding with another Vault's indexing queued, it
  stops at a chunk boundary, keeps its saved progress, and rejoins the back of
  the indexing lane.
  `AppState::runtime_config` supplies the immutable settings snapshot each
  reindex binds before it starts, including `HATCHDOOR_EMBED_LAYERS` for the
  per-Vault disposable candidate cache. `request_collection_reindex` is the
  collection-lane entry point an indexing-setting save uses: it requests one
  Index turn per active Vault through the same coordinator every other turn
  goes through, adding no second execution lane, and skips disabled Vaults,
  which have no active runtime.
  The one dispatch loop binds each turn's instance-wide Git commit identity
  from that turn's own settings snapshot (`git_author_defaults`) rather than
  from a value captured at startup, so a saved `HATCHDOOR_GIT_AUTHOR_NAME` or
  `HATCHDOOR_GIT_AUTHOR_EMAIL` applies to the next Git turn of any Vault
  without its own commit identity, with no restart.
- `AppConfig` is the environment-derived deployment contract and interprets the
  live values from the startup `RuntimeConfig` snapshot. Its process-level
  Vault source is always the local `VAULT_PATH`; Git source identity and Git
  behavior belong only to registry Vault definitions. The removed,
  development-only `HATCHDOOR_VAULT_SOURCE`/`HATCHDOOR_VAULT_GIT_*` family is
  rejected explicitly rather than silently falling back to the local path.
  `HOST` accepts numeric IP literals plus the DNS-free `localhost` alias;
  accepted bracketed or bare IPv6 literals normalize structurally before bind,
  and unsupported hostnames fail with guidance rather than depending on DNS.
  The built-in `--healthcheck` selects a local target in the listener's address
  family, preserving the IPv6 listener path in the shell-free runtime image.
- `StartupTracker` exposes startup/model/indexing readiness. `/ready` answers
  from it. `report_indexing_progress` is how an Index turn reports progress,
  and it never moves a tracker that has already settled `Ready`: a routine
  reindex is one Vault's upkeep, reported on that Vault, not an instance
  readiness change (#326). Each report names its Vault and carries every
  active Vault's settled state, so while first-run indexing covers several
  Vaults, `percent` and `eta_seconds` on the startup status describe the whole
  job, weighted by tokens to embed and never decreasing within one pass; the
  `notes_*`, `chunks_*` and `tokens_*` counters stay the current Vault's. A
  queued Vault is weighted by an approximate note count the executor takes
  off-thread from directory entries alone, without its mutation guard. Model
  setup starting over starts the pass over. All of it is in memory (#373).
- `VaultRuntime` and its serialized snapshot expose only the process startup's
  local source/mode, lifecycle phase, and derived non-Git capabilities. Git
  source, mode, and capabilities are derived per Vault by
  `VaultCollectionRuntime`; the startup-status adapter is not collection
  authority and never serializes a managed-Git source or mode.
- `VaultCollectionRuntime` reconciles only newer registry snapshots into zero,
  one, or many Vault-ID-keyed `VaultControlBlock` values; an older asynchronous
  reconciliation cannot replace or re-admit work after a newer collection is
  live. `reconcile()` activates replacement control blocks (directory stat,
  retained-snapshot read, recursive watcher registration) before it takes the
  collection write lock, and holds that lock only to install the result if
  the registry revision it read is still live; otherwise it revokes the blocks
  it built and starts over (#326). During activation, it derives Ready or Stale search capability from a
  retained participating SQLite snapshot before its reconstructed Index turn
  runs; a missing, nonparticipating, or unreadable snapshot remains
  Unavailable. Each enabled block owns its
  definition and resolved Markdown root, capability-specific activation/local
  content/search/Git/watcher status and errors, mutation and refresh locks, and
  independently cancellable watcher. Its `index_turn` (`VaultIndexTurn`:
  `running` or `waiting`, absent when idle) is the Vault's place in the
  indexing lane, published by `refresh_index_turn`, which reads the lane
  under the Vault's status lock; it is independent of `search`, which keeps
  saying what the Vault can answer while it waits (ADR-35). Status changes and
  `reconcile()` advance a
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
  safe boundaries. The mutation lock also carries a per-Vault count of the
  foreground mutations that have taken it. `acquire_mutation` advances it under
  the lock; `acquire_mutation_for_index_reads` gives a background Index turn the
  same exclusion without advancing it, and returns the generation it observed;
  `blocking_retake_mutation_for_index` retakes the lock from a blocking thread
  and answers whether a mutation intervened, under that one acquisition, so the
  caller decides and acts without a window in between (issue #223, following the
  `request_if_idle` rule of issue #127). The count is never readable outside a
  holder of that lock, which is the only place its value means anything.
  Unchanged Vaults retain their control blocks when another
  definition changes; disabled definitions remain visible with no capabilities
  and no active runtime. A Vault whose definition *did* change gets a
  replacement control block, and that block inherits the retiring one's
  `VaultWriteExclusion` — the mutation lock, its generation counter, and the
  refresh lock — so the exclusion's lifetime is the Vault's, not the block's,
  and an edit can never put two live mutexes on one Vault directory (issue
  #321, [ADR-25](../adr/README.md)). Only a genuinely new or re-enabled Vault
  gets a fresh exclusion. `write_exclusion()` exposes it so a caller holding a
  guard across a reconcile can check, by pointer, that a freshly resolved
  block still serializes against what it holds.
- `ModelSetup` owns local model selection, terms acceptance, download integrity,
  and persistent setup records. Once the embedder is installed, startup queues
  each active Vault through the collection Index coordinator; it does not run a
  second legacy single-Vault cache build. Startup becomes Ready once every
  active collection Vault's Index turn has settled (see
  `collection_indexes_settled` below), and stays Ready through later
  rebuilds and single-Vault failures; only model setup leaves it (#326).
- `spawn_vault_change_watcher` reports Vault-ID-qualified change intent through
  an independently cancellable handle. A qualifying filesystem event opens a
  quiet window that later events restart, bounded by a fixed ceiling
  (`WATCH_MAX_DEBOUNCE`): a sustained write burst reports intent no later than
  that ceiling after its window opened, instead of deferring it until the burst
  stops (#229). `run_server()`
  coalesces those intents through the shared `VaultWorkCoordinator` as Index
  requests. It is the only watcher: the transitional single-Vault adapter is
  gone with the rest of the legacy lane (#185). An event flagged `Rescan`
  (the kernel's queue overflowed and events were lost) always qualifies, since
  an Index turn is already the full rescan it asks for (#324).
- The watcher is not the only producer of that intent. The mutation core
  reports every successful foreground write on the same channel through
  `VaultControlBlock::report_write`, and before that labels the Vault's
  published snapshot stale through `mark_snapshot_behind_write`, still under
  the write's mutation guard. A write is therefore indexed, committed and
  reported stale in the meantime whether or not a watcher exists or saw it
  (#324); the coordinator coalesces the two reports of one change.
- The one dispatch loop in `run_server()` takes each turn the coordinator
  admits and runs it, and then its `publish_outcome`, on a task of its own
  through `vault_executor::VaultWorkExecutor`. It drops the turn only after
  publishing, so the next turn in that lane cannot race the outcome (see the
  Vault work execution boundary below). The loop itself holds no readiness
  policy, no turn logic, no limit of its own on concurrency, and no per-turn
  dependency assembly.
- `reconcile_and_reconstruct` activates or deactivates a scheduler-tracked
  Vault's `ManagedGitScheduler` entry (and, on deactivation, releases any held
  checkout lease) alongside its coordinator admission — `ManagedGit`, and an
  `ExistingGit` Vault in `PullOnly`/`TwoWay` mode (issue #132), both driven by
  the same `VaultSource::managed_git_poll_interval` accessor; an `ExistingGit`
  Vault in `LocalHistory` mode has no remote and is never registered.
  `set_local_content_status` (mirroring
  `set_search_status`/`set_git_status`) republishes authoritative
  local-content availability after a Git turn, since `activation_snapshot`
  only stats `vault_path` once, at `reconcile()` time, before a managed
  checkout exists. When that makes the Vault Active it also starts (or
  replaces) the Vault's watcher, which activation skipped for want of a
  directory (#324). `activation_snapshot`'s Git status defaults to `Pending`
  (an immediate first sync) for a genuinely new Vault or a
  disabled-to-enabled transition only; `reconcile()`'s non-retained-
  definition branch (an in-place edit to an already-active Vault) instead
  carries the retiring control block's actual current Git status and error
  through to the replacement (issue #97's reopening findings 1/2 follow-up)
  — otherwise every edit, not just an identity change, would force `Pending`
  and trigger an unwanted immediate real Git turn, bypassing an armed
  backoff or any other real status.
- Disabled runtime state becomes externally nonparticipating immediately;
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
store-resolved local Markdown roots and does not read credentials. Git-turn
dispatch, which does read them, moved out of this boundary into the Vault work
execution boundary below (#197).

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
`cargo test vault_runtime`, `cargo test vault_executor`,
`cargo test vault_watcher`, followed by the full backend checks.

### Background work coordination

**Kind:** infrastructure/runtime scheduling.

**Owned paths:** `src/vault_work.rs`.

**Public contract:** `VaultWorkCoordinator` is the cloneable request side and
`VaultWorkWorker` is the unique admission side of one instance-wide in-memory
queue with two lanes (ADR-31). Index and Repair turns share the indexing lane,
one turn at a time across every Vault. An Index turn may pause part-way for
another Vault and rejoin that lane (ADR-35): `another_vault_waits_to_index`
is the question it asks, and `requeue_paused_index_turn` moves its Vault's
one indexing position to the back, folding in any rerun requested while it
ran, and refuses a drained Vault so lifecycle still discards paused work.
`index_lane_state` (`IndexLaneState::Running` or `Waiting`) says where a
Vault's indexing stands, and the one observer set with `observe_index_lane`
is called, outside the queue lock, with each Vault whose indexing may have
moved; it re-reads the state rather than being told it, so racing
notifications cannot publish an older answer. Setting it reports every Vault
already queued, since startup reconstruction queues first Index turns before
the executor sets it. Git, Commit and Recovery turns
share the
Git lane: they never wait for an Index turn or for another Vault's Git work, at
most four Vaults (`GIT_LANE_WIDTH`, a constant, not a setting) run Git work at
once, and one Vault runs one Git-lane turn at a time. A Vault beyond the cap
waits for a slot, never for indexing. `VaultWorkWorker::next_turn` returns a
`VaultWorkTurn` that holds its lane slot until it is dropped, so the dispatch
loop runs each turn on its own task and the lanes, not the loop, bound the
overlap; the `#[cfg(test)]` `run_next` takes, runs and drops one turn for tests
that drive turns one at a time. Both lanes share one FIFO of request
positions, so a caller taking one turn at a time sees plain request order.
`VaultWorkKind`, `VaultWorkRequest`, `ScheduleResult`, `VaultWorkOutcome`,
and `VaultWorkError` expose deterministic one-operation turns, request
coalescing, lifecycle rejection, and Vault-qualified returned outcomes. Index
work includes local embedding work; Git, commit, and repair remain distinct
operation kinds. `VaultWorkKind::Commit` is separate from `Git` rather than a
flavour of it (#267) precisely so the two coalesce independently: a purely
local commit costs nothing and can run on every change, while talking to a
remote costs a round trip and stays on the Vault's schedule, and folding them
together would let a due sync swallow a pending commit or the reverse.
`VaultWorkKind::Recovery` (ADR-30) is its own kind for the same reason: an
operator's request to publish a conflicted Vault's recovery branch must
neither swallow nor be swallowed by a sync due at the same moment.
A stopped worker returns `None` rather than waiting for discarded work.
`VaultWorkCoordinator::request_if_idle` is `request` for an automatic,
unattended producer: it admits a turn only when that kind is neither active
nor already pending for the Vault, and never adds the one guaranteed rerun
`request` gives an already-active turn. The check and the enqueue happen
under the one lock that owns the answer, so no second, separately tracked
notion of "is this Vault busy" exists to drift out of agreement with it
(issue #127, replacing the read-then-act `has_work` bridge added for #97's
reopening finding 1). `has_work` remains only as a `#[cfg(test)]`
observation. A user-driven request — a manual sync or retry — still uses
`request` and its guaranteed rerun.

**Consumed dependencies:** durable `VaultId` identity and Tokio notification.
The queue owns no Markdown, SQLite, Git, or lifecycle state.

**Consumers:** collection runtime reconstructs and drains work for lifecycle
transitions; its `drain_vault` and boundary waits cover both lanes without
knowing they exist. `handlers/vaults.rs` reaches the coordinator only indirectly,
through `VaultCollectionRuntime::reconcile_and_reconstruct` after a registry
mutation, and directly through `ManagedGitScheduler::sync_now`/`retry_now` for
manual Git control and `VaultWorkCoordinator::request` for the one-Vault HTTP
refresh control — it never calls `drain_vault` itself. Runtime
composition (`src/server.rs`) runs every admitted turn on its own task through
`vault_executor` and joins them all before it exits; Repair remains separately
owned. `git::ManagedGitScheduler`'s
`tick` is the one production caller of `request_if_idle`. `vault_executor`
is the one caller of `requeue_paused_index_turn` and
`another_vault_waits_to_index`, and sets the index-lane observer.

**Coordination paths:** `src/lib.rs` for the module export; runtime composition,
per-Vault watcher intent, cache refresh, Git lifecycle, and repair producers
when their owning packets integrate the coordinator.

**Invariants:** one Vault occupies at most one FIFO position per lane; one
operation runs per turn; at most one Index or Repair turn runs at once; at most
four Vaults run Git-lane turns at once, and never two for the same Vault;
Index and Repair turns run in FIFO order, and a paused Index turn goes
behind everything already queued, never ahead; duplicate pending work
coalesces per
lane and duplicate active work retains at
most one rerun, except through `request_if_idle`, which an automatic producer
uses to add none; remaining work returns to the tail; a returned failure completes
its turn and remains attributable to one Vault. So does a panic in a turn's
future, in either lane: `VaultWorkTurn::run` catches it and completes the turn
with a non-retryable `TURN_PANICKED` (`vault_work_turn_panicked`) failure, so
one panicking turn cannot end the dispatch loop or leave its Vault's safe
boundary unreachable (#326). A turn completes only when it is dropped, which
also covers a turn abandoned midway; the dispatch loop drops it after
publishing its outcome, so the next turn in that lane never races that
publication, as in the single serial loop before ADR-31.
The queue adds no same-Vault gate between the lanes: the Vault's mutation lock
is the guard (ADR-25, ADR-31 decision 4), and no turn takes two Vaults' locks.
The queue stays disposable and adds no priorities, persistence, third lane,
configurable cap, generic timeout, or forced cancellation. Runtime lifecycle
stops new work, discards queued work in both lanes, and waits for the safe
boundary of whatever the Vault has running in either; restart reconstruction
uses durable definitions and current local-content/Git status.

**Validation:** `cargo test vault_work`, the runtime-composition tests when a
consumer is integrated, and the full backend checks.

### Vault work execution

**Kind:** infrastructure/runtime execution.

**Owned paths:** `src/vault_executor.rs`, `src/vault_executor/tests.rs`.

**Public contract:** `VaultWorkExecutor` is where one admitted turn runs.
`run` executes exactly one `VaultWorkRequest`; `publish_outcome` applies what
the collection concludes from a finished turn. The executor is assembled once
at startup and binds one immutable `ConfigSnapshot` at the start of every
turn, so an admitted operation observes a single configuration view while a
saved setting still reaches the next turn without a restart — that is where
`git_author_defaults` (the instance-wide `HATCHDOOR_GIT_AUTHOR_NAME`/`_EMAIL`
commit identity, overridden per Vault by
`git::config::resolve_commit_identity`) and `HATCHDOOR_EMBED_LAYERS` are read.
`collection_indexes_settled` is the startup readiness rule: startup becomes
Ready once every active Vault's Index turn has settled — searchable (`Ready`
or `Stale`), failed with the failure on that Vault's own status, or with no
local Markdown to index — and an empty collection is never Ready. A single
Vault's failure never marks the instance failed (#326). The same settled
rule, as `indexing_participants`, goes to the startup tracker with every
first-run Index progress report and again after each finished turn, so the
startup reading covers the whole collection; the first report of a pass also
spawns one thread that counts the queued Vaults' `.md` files from directory
entries, under the Vault's exclusions and without its mutation guard (#373).
`publish_outcome` also retries a retryable Index failure (other than `embedder_not_ready`)
through `request_if_idle` after a backoff that starts at
`INDEX_RETRY_BASE_DELAY` and doubles, at most `INDEX_RETRY_LIMIT` times per
run of consecutive failures; a success resets the count. A turn that
panicked (`TURN_PANICKED`) gets its Vault's failed search status published
here, since the turn never reached its own publication. `publish_outcome` logs the outcome first and contains a panic in its own work, because it runs on the shared dispatch loop outside the turn's panic boundary; the Vault control block's status lock tolerates poisoning so a turn that panicked while holding it cannot make every later read or publication of that Vault panic. Per ADR-13/ADR-18 this is a plain module with a
small public surface — no trait and no framework. Which turns overlap is the
coordinator's decision (ADR-31): `publish_outcome` runs on each turn's own
task, so the Index retry backoff and the commit cooldown behave the same
whichever lane the turn ran in.

- `dispatch_vault_index_turn` executes a `VaultWorkKind::Index` turn for one
  active Vault. Through `dispatch_vault_index_turn_with_progress`, `run`
  hands it an `IndexTurnSlicing` (the coordinator and `INDEX_TURN_SLICE`,
  five minutes of embedding, a constant per ADR-14; tests shorten the
  executor's `index_slice`). Once the build has embedded for a slice, it asks
  before each further chunk whether another Vault is waiting to index, and
  if one is, the build stops (`cache::IndexYield`,
  `SnapshotPublication::Paused`). The turn then requeues its Vault behind
  every Vault already waiting, publishes the search status its retained
  generation supports with no error (`retained_search_status`, shared with
  the failure path), and returns `Ok`: a pause is not a failure, and the
  next turn resumes from the saved progress. A turn that has embedded
  nothing yet, or has nothing left to embed, never pauses. It acquires that
  Vault's foreground mutation and refresh
  boundaries, builds an authoritative Markdown index and isolated candidate
  cache off the async runtime, hands that scan's asset catalog and layer map
  to the control block for the demo asset check (#377), publishes a structure-only participating
  snapshot before vector embedding on a first build so browsing does not wait
  for semantic search, atomically publishes only that Vault's complete shared
  snapshot, and publishes Ready, Stale, or Unavailable search state without
  changing another Vault's snapshot or status. A retained snapshot is marked
  stale for the duration of the rebuild, not only after a failure.
  The foreground mutation guard spans the read phase only — the authoritative
  scan, the structure pass, and every per-note content read — so a turn can
  never observe half of a multi-file foreground mutation, and is released at
  the read/embed boundary inside the candidate build. Holding it across the
  embedding pass parked every HTTP and MCP Markdown write behind a turn that
  was no longer reading anything, long enough for the caller's transport to
  give up on a write that had already landed (issue #223). The turn retakes
  the guard to publish and, when a foreground mutation completed while it was
  released, publishes that generation `VaultSnapshotFreshness::Stale` rather
  than `Fresh` and settles the runtime at `VaultSearchStatus::Stale` rather than
  `Ready` — the same pair `retained_snapshot_search_status` derives from that
  row after a restart. It still participates, still holds the search
  capability, and still answers search; the watcher's change intent has already
  armed the catch-up turn that makes it `Ready`. Retaking the guard happens
  while the cache's process-wide model epoch is held, so that acquisition is
  the one place the epoch waits on a per-Vault lock; the wait is bounded by one
  in-flight foreground mutation, and no mutation path takes the epoch, so the
  order cannot cycle. A turn
  requested before first-run model setup has installed the embedder defers
  with `embedder_not_ready` rather than wiping a valid cache.
- `dispatch_git_turn` executes a `VaultWorkKind::Git` turn. One shared
  shell owns everything the three Git-capable source kinds have in common —
  the per-Vault commit identity, the credential read, the mutation-lock hold,
  `spawn_blocking`, panic mapping, and outcome publication — and
  `plan_git_turn` supplies only what differs (issue #128). A `GitTurnPlan`
  names three variations: whether the turn holds
  `VaultControlBlock::acquire_mutation`, the error code a panic is reported
  as, and its `GitTurnWork` — `Leased` (the managed checkout, which cannot be
  built or run without that Vault's lease) or `Unleased` (an operator-owned
  checkout, which never takes one).
  - `ManagedGit`: obtains that Vault's checkout lease from
    `ManagedGitScheduler` (reused across turns for as long as the Vault stays
    active in this process — issue #95), then the mutation lock, runs
    `git::run_managed_git_turn`, and hands the lease back afterward. The lease
    is always acquired before the mutation lock, and nothing else in the
    codebase acquires it, so the two can never be taken in opposite orders.
  - `ExistingGit` in `PullOnly`/`TwoWay`: runs
    `git::run_existing_git_remote_turn` against the checkout that already
    exists at the Vault's `repository_path` — no checkout lease, see the Git
    synchronization boundary below for why `ManagedCheckoutLease` does not
    apply to an already-existing, operator-owned checkout — but under the same
    `acquire_mutation` hold as the managed-Git path, so a foreground Markdown
    write can never race either kind of turn's working-tree phases.
  - `ExistingGit` in `LocalHistory`: delegates to `plan_commit_turn`, because
    for a Vault with no remote the Git turn always was a commit and nothing
    else. Since #267 nothing production requests `VaultWorkKind::Git` for such
    a Vault, because activation, the watcher and manual control all ask for
    `Commit`, so this arm is a defensive alias rather than a live path.
  - `Local`: no Git turn at all; returns without publishing anything.

  Every branch that can commit is handed the Vault's `write_ledger()` so the
  commit it makes is named by the writes it records (#249).

  Both locked paths hold the mutation lock for the whole blocking turn
  (coarser than the legacy single-Vault task's fine-grained per-phase locking
  that released across network-only fetch/push) rather than only across
  working-tree-mutating phases; splitting `synchronize_managed_checkout` into
  independently lockable phases to match that finer discipline was judged a
  materially larger change than issue #96's reopening warranted.
- `dispatch_commit_turn` executes a `VaultWorkKind::Commit` turn: the local
  half of Git, on its own (#267). `plan_commit_turn` resolves the Vault's
  source and mode to `git::run_local_history_git_turn` (Local history, no
  mutation lock), `git::run_existing_git_commit_turn` (`ExistingGit` Two-way,
  mutation lock held because `prepare_two_way_worktree` stages from the whole
  checkout's status), or `git::run_managed_git_commit_turn` (`ManagedGit`
  Two-way, lease *and* mutation lock). Pull-only and `Local` plan nothing and
  return `Ok(())`: the first refuses writes and must leave its operator's own
  drift alone, the second has no Git. The turn itself runs through
  `run_planned_turn`, the same lease/mutation-lock/`spawn_blocking` shell a
  Git turn uses, extracted so neither kind has its own copy.
- `finish_commit_turn` publishes a commit turn's outcome, and is deliberately
  not `finish_git_turn` on three counts. It does not feed
  `ManagedGitScheduler`, because a commit is not a check of the remote and
  must not move the schedule that governs one. It does not request an Index
  turn, because the watcher change that asked for this commit already
  requested one, which is also what keeps a Vault whose Git is broken
  indexing normally. And a failure arms that Vault's `git::CommitCooldown`,
  so a standing failure costs one turn per cooldown window rather than one per
  save; a success clears it. Status publication is otherwise identical to a
  sync turn's: `Ready`, or `Unavailable` with the structured error and its
  affected paths. A commit that succeeds clears only a failure a commit could
  have caused: a standing remote-only failure (`git::managed_task::is_remote_only_failure`:
  a conflict with the remote, a refused push, an exhausted push race, an
  unreachable remote, rejected credentials, unpushed Pull-only commits) stays
  published, file list included, until a sync turn resolves it (#323). Syncs
  run on the poll interval and commits on every save, so the earlier rule,
  a commit clears everything, erased a conflict within one save of it
  appearing.
- `dispatch_recovery_turn` executes a `VaultWorkKind::Recovery` turn
  (ADR-30): it re-checks the Vault's `publish_recovery` capability (a sync may
  have resolved the conflict while the request waited), then
  `plan_recovery_turn` resolves a Two-way source to
  `git::run_existing_git_recovery_turn` or `git::run_managed_recovery_turn`
  (lease for the managed one), both under the mutation lock and through the
  same `run_planned_turn` shell, which is generic over the turn's output for
  this reason. `finish_recovery_turn` publishes only the Vault's
  `recovery_branch` status, keeping an earlier publication's fields on a
  refusal for the same branch: it never touches Git status, never feeds
  `ManagedGitScheduler`, and never requests an Index turn, because a publish
  is not a check of the remote and the conflict stays the Vault's failure.
  `publish_managed_git_turn_outcome` clears `recovery_branch` on a successful
  sync.
- `publish_managed_git_turn_outcome` is the single publication path every Git
  turn exit reaches: Git status always, plus authoritative local-content
  availability on success (`activation_snapshot` only stats `vault_path` once,
  at `reconcile()` time, before a managed checkout exists), plus
  `ManagedGitScheduler::record_outcome` so the next attempt is armed. A Git
  failure never touches local-content status, so a Vault that already has a
  usable checkout stays browsable through a later sync failure. A successful
  turn with usable local Markdown requests that Vault's Index turn through the
  same coordinator.

**Consumed dependencies:** the work coordinator, the collection runtime and its
per-Vault control blocks, the Vault registry, the managed-Git scheduler and
turn functions, the disposable SQLite cache, the embedder, live configuration,
and the startup tracker — every one of them a field of `AppState`, which
`from_state` reads them off, so the composition root assembles nothing.
Managed-Git dispatch is the one place outside the registry that reads
plaintext credentials, through the crate-private `https_credentials`
accessor, for Git authentication only.

`report_index_lane_on_vault_status`, called from `from_state`, sets the
coordinator's index-lane observer so every change to a Vault's place in the
indexing lane reaches its `index_turn` status, whichever producer queued the
work (ADR-35 decision 5).

**Consumers:** `src/server.rs`'s dispatch loop, which runs each admitted turn
and its `publish_outcome` on a task of its own. Nothing else constructs a
`VaultWorkExecutor`; lifecycle tests in `src/vault_runtime/tests.rs` reach the
`#[cfg(test)]` `dispatch_vault_index_turn` seam directly.

**Coordination paths:** `src/lib.rs` exports the module; `src/server.rs` owns
the loop that calls it.

**Invariants:** one turn observes one settings snapshot; every Git turn exit
publishes through one path; the mutation lock is never acquired before the
checkout lease; an Index turn holds the foreground mutation guard across every
read it makes of the Vault and publishes a generation built across a foreground
mutation as stale; a returned failure is the turn's result, not a panic; no turn
starts its own scheduler or decides which other turns may overlap it.

**Validation:** `cargo test vault_executor`, `cargo test vault_work`,
`cargo test vault_runtime`, `cargo test managed_task`, followed by the full
backend checks.

### Live configuration foundation

**Kind:** infrastructure/runtime state.

**Owned paths:** `src/runtime_config.rs`.

**Public contract:** `RuntimeConfig`, `ConfigSnapshot`, `ResolvedSetting`,
`SettingSource`, `Environment`, `SETTINGS_SCHEMA`, `live_settings_defaults`,
`settings_file_path`, `is_truthy`, and the versioned
`settings.json` file format. `RuntimeConfig::snapshot` gives one immutable,
lock-free configuration view to bind at the start of an operation;
`RuntimeConfig::save` serializes writes, persists first, then publishes the
new view. `RuntimeConfig::remove_stored` lets the registry's startup step
purge the retired Git-lane keys, persisting before publishing and leaving
environment pins and unrelated values untouched.
`RuntimeConfig::validate_and_save` runs a caller-supplied decision
against the snapshot current at the moment the write lock is taken and only
persists on success, so validation and persistence serialize behind the same
lock (no separate read-then-write race). `ConfigSnapshot::required` is the one
accessor for "this key's value, or a descriptive error" that `src/config.rs`,
`src/mcp/config.rs`, and `src/git/config.rs` all call rather than each keeping
its own copy; `ConfigSnapshot::pinned_count` and `RuntimeConfig::settings_path`
support the startup pinned-setting log line and local-versioning `.gitignore`
setup respectively. `RuntimeConfig::value_revision` counts how many published
snapshots changed one key's resolved value, so a consumer that derives
something from a value (the transfer-link key from the MCP token) learns of
every change, including a change back to an earlier value.

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

### Filesystem rename-flag capability

**Kind:** infrastructure/filesystem foundation.

**Owned paths:** `src/rename_flags.rs`.

**Public contract:** `RenameFlag`, `FlagSupport`, `support`,
`flag_unavailable`, `rename_flagged_at`, `rename_flagged_paths`, and the
test-only `force_unsupported_for_tests`.
`support` answers whether one `renameat2` flag works on the filesystem holding
a directory, by performing the real operation on dot-prefixed scratch names
there and caching the verdict per filesystem. `flag_unavailable` is the
question a failed flagged rename asks: it combines the errno with that probe,
so misuse of the syscall stays an error while a filesystem that does not
implement the flag earns a fallback.

**Consumed dependencies:** `libc` and the local filesystem only. Nothing here
knows about Vaults, notes, or Git.

**Consumers:** the Vault mutation write layer (`src/vault/write/fs_ops.rs`) for
its conditional-write and move commits, the Vault-qualified mutation core
(`src/vault_mutation.rs`) for the reported write capability, Vault runtime
activation (`src/vault_runtime.rs`) for the one-line-per-Vault report, and the
managed Git checkout install (`src/git/managed_checkout.rs`).

**Coordination paths:** `src/lib.rs` exports the boundary.

**Invariants:**

- The verdict comes from performing the operation, never from a filesystem's
  name or version (ADR-26).
- `Undetermined` is a distinct answer from `Unsupported`, and a directory the
  probe cannot use reports the former: a Vault unwritable for its own reasons
  is never reported as lacking compare-and-swap. Only a definite verdict is
  cached, so a directory that later becomes usable is re-probed, and because
  the cache is keyed per filesystem, a read-only directory on a filesystem
  already known to be capable answers `Supported`.
- A probe leaves nothing behind: its scratch names are dot-prefixed, which the
  Vault already excludes as noise, and it unlinks them on every path.
- A bare `EINVAL` is never on its own proof that a flag is missing, because it
  is equally the errno for misusing the call.

**Validation:** `cargo test rename_flags`, plus `cargo test vault::write` and
`cargo test vault_mutation` for the callers, followed by the full backend
checks.

### Vault collection registry

**Kind:** infrastructure/persistent domain state.

**Owned paths:**

- `src/vault_registry.rs`
- `src/vault_registry/tests.rs`
- `src/vault_migration.rs` (the registry's startup step)

**Public contract:** `DEFAULT_VAULT_REGISTRY_PATH`, the startup step
`vault_migration::{prepare_registry, RegistryStartupError,
LEGACY_MIGRATION_DOC_URL}`,
`REGISTRY_SCHEMA_VERSION`, canonical `VaultId` generation and parsing,
`VaultRegistryStore`, immutable `VaultRegistrySnapshot` values, explicit
`VaultRegistryState::Ready` versus `Recovery`, structured recovery/error
types, redacted `VaultDefinition` projections, tagged `VaultSource` values for
local, existing-Git, and managed-Git Vaults, `VaultGitMode`, credential write
inputs/updates, validated `add`/`edit`/`enable`/`disable`/`disconnect`
operations, store-owned `vault_path` resolution for runtime consumers, the
crate-private `ensure_outside_instance_state` containment check the folder
listing reuses to hide instance state,
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
explicit empty initialization (`initialize_empty`), and the
versioned `/data/state/vaults.json` format. An absent file
is a complete revision-0 zero-Vault state and is not created by reads; only a
definite `NotFound` counts as absent, and any other read or stat failure is a
`Storage` error rather than an empty registry a mutation could commit over
(#325). `add`, `edit`, and `enable` refuse, as `InvalidSource`, a `Local` root
or `ExistingGit` checkout that contains or sits inside the registry's own state
directory or any directory named through `with_reserved_directories` (runtime
composition passes the cache and settings directories); a `ManagedGit`
checkout, whose location under the state directory the store itself chooses,
is exempt (#325). Commits
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

`vault_migration::prepare_registry` is the registry's startup step, all that
remains of the legacy single-Vault import #427 removed (ADR-40). A start that
finds a registry purges the retired Git-lane keys from stored settings
(`HATCHDOOR_GIT_SYNC_ENABLED`, `_HTTPS_TOKEN`, `_HTTPS_USERNAME`, `_REMOTE`,
`_BRANCH`, `_DEBOUNCE_SECONDS`), logging and retrying on the next start on
failure, so a plaintext Git token never survives there (#325), then loads it.
A start that finds none refuses with
`RegistryStartupError::UnconvertedLegacyInstall` when stored settings still
carry any of those retired Git-lane keys, which only a single-Vault release
wrote: that install is from 2.4.x or earlier, and the
message sends it through a 2.5.0 to 2.7.x release and names
`docs/migrations/legacy-single-vault.md`. Otherwise it writes an empty
registry, whatever `VAULT_PATH` holds. It never reads, registers, or writes
the `VAULT_PATH` folder, and the refusal writes nothing at all.
`HATCHDOOR_EXCLUDE` and the author keys never trigger it, because a current
install stores them too.

**Consumers:** runtime composition calls `vault_migration::prepare_registry`
after the security refusals and before recording the start or opening the
cache, and loads the registry through it; that step consumes `load` and
`initialize_empty`, and live configuration's `remove_stored` and snapshot.
Runtime composition then passes the snapshot on, and
`VaultCollectionRuntime` consumes its safe
projections and resolved paths; the Vault work executor's managed-Git Git-turn
dispatch (`dispatch_git_turn`) is the one consumer of the crate-private
`https_credentials` accessor, and also resolves `commit_identity` through
`git::config::resolve_commit_identity` before every Git turn. `handlers/vaults.rs`
is the first HTTP consumer of the `add`/`edit`/`enable`/`disable`/`disconnect`
mutation contracts and of `load` for authenticated discovery, including its
explicit `Recovery` state. `ManagedGitScheduler::activate` (runtime
composition's reconcile loop) and Vault collection management's manual
sync/retry and credential-replacement-retry controls consume
`VaultSource::managed_git_poll_interval`. `AppState::vault_archive_prefix`
(Vault mutation's three archive call sites: `handlers/vault_content.rs`,
`handlers/vault_write.rs`, `mcp/tools/write.rs`) consumes `archive_folder`,
falling back to `AppState::runtime_archive_prefix` when absent. Both the HTTP and MCP surfaces reach these
writes through Vault collection management, which owns the
`create_vault`/`edit_vault` request and credential-patch types they share.
Frontend, cache, and search adapters remain separately owned later packets.

The folder listing consumes `load`, `vault_path` and
`ensure_outside_instance_state` to flag registered Vaults and skip folders
that hold instance state.

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

**Validation:** `cargo test vault_registry`, `cargo test vault_migration`,
`cargo test runtime_config`, `node scripts/check-module-map.mjs`, followed by
the full backend checks.

### Vault durable runtime state

**Kind:** infrastructure/persistent operational state.

**Owned paths:**

- `src/vault_runtime_state.rs`
- `src/vault_runtime_state/tests.rs`

**Public contract:** `RUNTIME_STATE_SCHEMA_VERSION`,
`RUNTIME_STATE_FILE_NAME`, `VaultRuntimeStateStore` (`new`,
`beside_registry`, `last_git_turn`, `record_git_turn`, `forget`),
`GitTurnRecord`, `GitTurnOutcome`, and `format_timestamp` — the RFC 3339 UTC
seconds-precision shape shared with `vault_management`, so a timestamp reads
identically whether it came from this file or from the live countdown. The
versioned `state/vault-runtime.json` format: one Git-turn record per Vault,
keyed by Vault ID, each holding the wall-clock `completed_at`, the `outcome`,
and a `code` and already-redacted `message` written for a failure — carried so
a restarted instance republishes the same sentence the previous process showed
rather than falling back to a generic one. `GitTurnOutcome::Failed` owns those
two details, so no caller above the file boundary can hold a failure without
them; a stored record missing either still loads, with the fallback applied
once, where the file is read.

**Consumers:** Git synchronization (`git::ManagedGitScheduler`, which reads a
Vault's record once per activation and writes one after every
interval-arming turn), the runtime composition root, which resolves the store
beside the registry, and Vault collection management, for `format_timestamp`
alone. No HTTP, MCP, or frontend consumer reads this file directly — a status
read renders the scheduler's in-memory clock instead.

**Invariants:** this file records disposable operational state, never
configuration or credentials, and carries no revision or concurrency contract
of its own — a poll can rewrite it without disturbing the Vault collection or
a client's `expected_registry_revision`. Losing it costs one extra Git turn
per Vault, so a missing, unreadable, or unparseable file reads as "no record"
and never blocks startup. Every read-modify-write (`record_git_turn`,
`forget`) is serialized by one lock that clones of a store share, and the
file is replaced by write-to-temporary-then-rename in its own directory, so
a record and a forget cannot undo each other and no reader sees a partial
file (#326); a file whose `schema_version` exceeds this build's
is read as "no record" and, unlike a corrupt one, is never overwritten, so a
downgrade cannot destroy a newer build's state. Records are written whole
through the parent directory's creation, hold owner-only content by way of the
same state directory, and only interval-arming outcomes are stored — a
transient failure's backoff stays process-local by design. Vault contents
remain authoritative Markdown and SQLite remains disposable (ADR-01); the
store adds no service, framework, or speculative trait (ADR-02/13).

**Validation:** `cargo test vault_runtime_state`,
`node scripts/check-module-map.mjs`, followed by the full backend checks.

### Instance state

**Status:** Added by #424 (ADR-40 decision 6, ADR-42); the last agent
connection added by #426.

**Kind:** infrastructure/persistent operational state.

**Owned paths:** `src/instance_state.rs`.

**Public contract:** `INSTANCE_STATE_SCHEMA_VERSION`,
`INSTANCE_STATE_FILE_NAME`, `UNRECORDED_UPGRADE_FROM`,
`InstanceStateStore` (`new`, `beside_registry`, `path`, `record_start`,
`section`, `write_section`), `VersionRecord` (`current`, `previous`,
`fresh_install`, `after_start`, and a `Default` of the running version with no
history), `base_version`, which reads `2.8.0 (dev abc123)` as `2.8.0`,
`AgentConnection` (`name`, `connected_at` in RFC 3339 UTC),
and `AgentConnectionLog` (`load`, `latest`, `observe`, `save`, and a
`Default` that keeps the record in memory only). The versioned `state/instance.json` format: a
`schema_version` beside named sections, each a JSON value owned by one
feature. `versions` is this module's own: `current`, `previous` and
`fresh_install`, all base versions. `previous` moves only when the base
version changes; with no record, a registry or stored settings on disk means
an upgrade from `UNRECORDED_UPGRADE_FROM` (2.7.0), with no previous version
when the running one is 2.7.0 itself, and neither means a fresh install of the
running version. `last_agent` is this module's too: the last MCP client's
cleaned name and the time of its call. `observe` updates it in memory and
reports a save as due only when the name changed or a minute has passed since
the last save. Other sections belong to the features that write them through
`write_section` (#425).

**Consumers:** the runtime composition root, which reads whether the install
existed before `vault_migration::prepare_registry` can write a registry,
records the start once that step succeeds, and holds the record in
`AppState::instance_versions`, and
through it `src/handlers/whats_new.rs`; the composition root also loads
`AppState::agent_connections` from the same store, which
`src/mcp/adapter.rs` feeds on every tool call and `src/handlers/settings.rs`
reads.

**Consumed dependencies:** `config::version_string` for the default record.

**Invariants:** bookkeeping, never configuration or credentials: a missing or
unreadable file reads as "no record", and `record_start` never fails, logging a
write it could not make and returning the record it computed, so a state
directory that cannot be written never blocks startup. A file whose
`schema_version` exceeds this build's is treated as no record and never
written. Every read-modify-write is serialized by one lock that clones share,
and the file is replaced by write-to-temporary-then-rename in its own
directory. It never touches a Vault and never contacts the network. The last
agent record holds only a client name, cleaned of control and invisible
formatting characters and cut to 100 characters, and a time: never tool names,
arguments, results, addresses or tokens. A save that fails is logged and the
record stays in memory.

**Validation:** `cargo test instance_state`, `cargo test
server::tests::a_start_with`, `node scripts/check-module-map.mjs`, followed by
the full backend checks.

### Update check

**Status:** Added by #425 (ADR-39).

**Kind:** infrastructure/background capability.

**Owned paths:** `src/update_check.rs`.

**Public contract:** `UPDATE_CHECK_SETTING` (`HATCHDOOR_UPDATE_CHECK_ENABLED`,
off by default), `CHECK_INTERVAL` (a day), `TICK_INTERVAL` (a minute),
`FetchLatest` (the seam tests replace: returns the latest release's tag),
`github_latest_release` (the real request), `UpdateChecker` (`new`, `tick`),
`spawn`, `status`, and the wire types `UpdateCheckStatus` (`enabled`,
`checked_at`, `update_available`) and `LatestRelease` (`version`,
`release_url`). The `update_check` section of `state/instance.json`, written
through `InstanceStateStore::write_section`: `checked_at` (the last attempt,
RFC 3339 UTC) and `latest` (the last check's answer, `None` after a failure,
so a failed check shows no banner). `status` reports `update_available` only while the setting is on and
the stored version is newer than the running base version.

**Consumers:** runtime composition spawns it with the live configuration,
the instance state store and `github_latest_release`, never in demo mode;
`src/handlers/settings.rs` serves `status` as the settings response's
`update_check`, read from the store beside the registry; the frontend Update
banner reads that field.

**Consumed dependencies:** Live configuration foundation (the setting, read
per tick), Instance state (`InstanceStateStore`, `base_version`),
`config::version_string`, `ureq` (ADR-39, native TLS as `hf-hub` uses it).

**Invariants:** no request at all while the setting is off; at most one
request a day, plus one at the first tick after the setting is switched on,
and never more than one a minute even when the state file cannot be written;
the request is one `GET` to GitHub's latest-release API for this repository
with the user-agent `Hatchdoor`, no version and nothing about the instance; a
failure is logged at info and tried again a day later; the release link is
built from the parsed version on this repository's release page, never taken
from the response, and a tag that is not three plain numbers is never offered;
nothing is downloaded or installed. Tests never reach the network.

**Validation:** `cargo test update_check`, `cargo test handlers::settings`,
followed by the full backend checks.

### Web authentication

**Kind:** infrastructure/security.

**Owned paths:** `src/auth.rs`.

**Public contract:** `WebToken`, `WebOrLiveMcpToken`, `require_web_token`,
`require_web_or_live_mcp_token`, and `require_web_or_live_mcp_read_token`, plus
the crate-internal `request_is_authorized(request, token)`, the web-token check
those middlewares share, which the public manual routes (`src/handlers/docs.rs`)
use to decide whether a caller may see private pages without gating the route. Both
attachment middlewares bind the MCP token from the current runtime snapshot
instead of retaining a token captured at startup, so disabling MCP at runtime
immediately revokes that credential; web-token admission is independent of MCP
state in both.

The upload middleware accepts the MCP token only while MCP *and* MCP writes are
enabled, so a write-mode disable revokes upload capability. With no web token
configured it admits every request, like the rest of the web API: the MCP
token can only add access to a gated route, never start demanding one, so
setting it up cannot 401 the browser's paste-to-upload (#327). The asset-read
middleware accepts it whenever MCP is enabled, so that `get_attachment`'s
default `download_url` is fetchable by the client that holds it (#176).

Admission on the MCP token is **not** equivalent to admission on the web token,
and two constraints carried by the read middleware are what keep it from
widening that credential's reach. Over `/mcp` an MCP client reading attachment
bytes is bounded by `HATCHDOOR_MCP_MAX_BASE64_BYTES` and by #171's tool quota
and concurrency caps; the asset route's own bound is far larger (64 MiB) and it
sits outside the `/mcp` transport, so an unconstrained `download_url` would be a
way around both. Therefore an MCP-admitted request (a) spends the same quota and
concurrency budget as a tool call, against the *same* `RateLimiter` instance the
transport uses — obtained via `HatchdoorMcpTransport::limiter`, so the two
channels share one budget rather than getting one each — answering `429` with
`Retry-After` when exhausted, and (b) carries the `McpAssetRead` request
extension, which `vault_content.rs`'s asset handler reads to apply
`max_base64_bytes` as the response ceiling before the file is buffered. A
web-token request carries no extension and keeps the route's own bound.

The read middleware keeps `require_web_token`'s `access_token` query fallback
for the *web* token alone, since the browser reaches assets through `<img>` tags
and download navigations; the MCP token is header-only, keeping it out of the
request trace span. It is layered on exactly the condition `vaults_v1` uses — a
web token configured, and not demo mode — so a deployment with no web token
keeps serving assets openly and enabling MCP never demands a credential the
browser has never held.

**Consumers:** `server.rs` and protected HTTP routes.

**Consumed dependencies:** live runtime configuration and `McpConfig` parsing
for per-request attachment authorization.

**Coordination paths:** `src/server.rs`, `src/config.rs`, frontend
`frontend/src/api/api.ts`, and any route whose authentication requirements
change.

`RateLimiter::admit_tool_call` is the one admission sequence (concurrency,
then quota) for a tool call's worth of work outside `/mcp`, shared by the
asset-read guard and transfer-link downloads, and `too_many_requests` is their
shared `429`.
`redact_query_token` redacts a transfer link's `signature` as well as the web
token's `access_token`, so neither reaches the request trace span (ADR-27).
Transfer links are redeemed on their own routes, not through these guards.

**Invariants:** constant-time token comparison, no token logging, and deliberate
query-parameter fallback for browser contexts that cannot set headers (ADR-08).

**Validation:** `cargo test auth` and server/router tests.

### Transfer links

**Kind:** infrastructure/security.

**Owned paths:** `src/transfer_link.rs`.

**Public contract:** `TransferLinks` (one instance in `AppState`), with `key`,
`mint_download`, `mint_upload`, `mint_note_replace`, `verify_download`,
`redeem_upload`, and `redeem_note_upload`; `SigningKey`, `MintedLink`, `Grant`,
`NoteUploadRule`, `LinkRefusal` (and its stable `code`s), `LINK_LIFETIME`, and
`SIGNATURE_PARAM`. A link is an absolute URL on the Vault-scoped
`/transfers/{*path}` route whose query carries `expires`, for uploads
`overwrite` and `nonce`, for a link that replaces a note
`expected_content_hash`, and a BLAKE3 keyed-hash `signature` over the Vault ID,
the relative path, the expiry, and the grant. A replacing note link is its own
grant (`Grant::ReplaceNote`), so an attachment link signs and parses exactly as
it did before ADR-32. `redeem_note_upload` answers `NoteUploadRule::Create` for
a plain non-overwriting upload link and `Replace` with the signed hash for a
replacing one, and refuses a plain overwriting link; `redeem_upload` refuses a
replacing note link. `key` derives the
`SigningKey` one request works under from the in-memory master key, the MCP
token's revision, and the token the request was admitted on, so a tool call
admitted on a token that has since rotated mints links the new token will not
accept. See ADR-27 and ADR-32.

**Consumers:** the MCP `get_attachment` and `create_upload_link` tools mint;
`src/handlers/transfer.rs` verifies and redeems; `src/auth.rs` redacts
`SIGNATURE_PARAM` from the request trace span.

**Consumed dependencies:** `RuntimeConfig::value_revision` for the MCP token's
revision, `vault_read::encode_relative_path` for the per-segment path encoding,
and `auth::constant_time_eq`.

**Coordination paths:** `src/app_state.rs` (the field), `src/lib.rs`,
`src/server.rs` (the `/transfers/{*path}` route and the span's `traced_uri`),
`src/handlers/mod.rs`, `src/mcp/tools/mod.rs` (`transfer_link_signer`), and
`src/config.rs` (`capped_log_filter`, which holds the `rmcp` crate and its two
message-logging modules at info whatever `RUST_LOG` asks for, because a tool
result can carry a link).

**Invariants:** a link names one Vault and one path and one direction and is
refused for anything else; it expires five minutes after minting, checked when
the transfer starts; the signing key is random, held only in memory, and bound
to the MCP token's revision and value, so a restart or any token change
(including back to an old value) strands every link; a link is always
absolute, and minting is refused when there is no address to build it on;
no link credential reaches a log; an upload link is spent by its first
redemption; no link replaces a note without the expected content hash signed
into it at minting (ADR-32); this module never decides whether MCP or write
mode is on, which the redeeming adapter re-reads per request.

**Validation:** `cargo test transfer_link`, `cargo test transfer` in the server
router tests, followed by the full backend checks.

### Bundled manual

**Status:** Added by #421 (ADR-38).

**Kind:** product capability/domain core.

**Owned paths:** `src/docs_bundle.rs`.

**Public contract:** `pages()` (Home first, then path order), `home()`,
`page(name)`, and `search(query)`, which returns at most `SEARCH_RESULTS`
pages, private ones included; `manual()`, the `Manual` behind them, whose
`search(query, include_private)` can leave private pages, and links to them in
excerpts, out, and whose `markdown_linking(page, link)` renders a page with
each wikilink pointed wherever `link` says (or reduced to its text);
`ManualPage` (`name`, `title`, `markdown`, `private`) and `ManualSearchHit` (`page`, `excerpt`).
`private` is `private: true` in the page's frontmatter (ADR-38 decision 6).
`Manual::from_sources` builds a manual from fixture pages for tests. A page's name is its path under
`docs/user-vault` with each folder's ordering number dropped and every segment
put through the note slug rule, so `03 Reference/MCP tools reference.md` is
`reference/mcp-tools-reference`. `page` ignores case, surrounding slashes and a
`#heading` fragment. A page's `markdown` has no frontmatter, and every wikilink
outside code is a Markdown link to a page name, plus a `#heading` anchor in the
note slug rule when the link names a heading. `search` is case-insensitive word
matching with a trailing plural `s` folded and common words such as `how` and
`the` dropped from the query. Pages rank by how many query words their title
holds, then their headings outside code, then how often the words appear
anywhere, code included; a query that matches nothing returns nothing.

**Consumers:** the MCP `read_docs` and `search_docs` tools
(`src/mcp/tools/read.rs`), the public manual routes in
`src/handlers/docs.rs` (which the frontend Help reader reads), and
`src/handlers/whats_new.rs`, which reads the private What's new page's
releases.

**Consumed dependencies:** `vault::slugify` for page names and anchors, and
`cache::parse::{frontmatter_span, parse_fence_marker,
parse_frontmatter_metadata}` to strip frontmatter, read the `private` flag and
leave code blocks alone.

**Coordination paths:** `src/lib.rs`, `Dockerfile` and `.dockerignore` (the
image build must see `docs/user-vault` for `include_str!`), and
`scripts/check-docs-freshness.mjs` (the `bundled-manual` surface, and
`docs/user-vault/` as shipped content).

**Invariants:** every Markdown file under `docs/user-vault` is bundled and
nothing else is (`every_manual_page_is_bundled`, `only_markdown_is_bundled`);
every link on every page resolves to a bundled page and heading
(`every_link_on_every_page_resolves_to_a_bundled_page`); the manual is never a
Vault, a registry entry, a cache row or an embedding, so it never appears in
note search, the tree, stats, the graph or a Vault list; nothing writes to it;
it needs no Vault, index or model, so it answers during model setup.

**Validation:** `cargo test docs_bundle`, `cargo test mcp`, and
`docker build` for the image's view of `docs/user-vault`.

### Folder listing

**Kind:** product capability/adapter (the first filesystem read outside any
Vault).

**Owned paths:** `src/folder_listing.rs`.

**Public contract:** `list_folders(root, relative, registry, limits)` returns a
`FolderListing` (`root`, `root_found`, `path`, `markdown`, `vault`, `folders`,
`skipped_invalid_names`) of the immediate subfolders of one folder under the
Vault mount, each a `FolderEntry` with its name, relative path, recursive
`MarkdownCount { count, at_least }`, the `RegisteredVault` rooted exactly
there, and `has_subfolders`; or a `FolderListingError` (`OutsideRoot`,
`NotFound`, `Unreadable`) with a stable `code`. `ListingLimits` defaults to
`MARKDOWN_COUNT_CAP` notes per folder and `COUNT_TIME_BUDGET` per listing. A
missing root is an empty listing with `root_found: false`, not an error.
`root` is the configured mount made absolute, not resolved through symlinks,
so a picker joins it with a folder's `path` to get the path a Vault is created
from (#430). See ADR-41.

**Consumers:** `src/handlers/folders.rs` (`GET /api/v1/folders`), and through
it Settings' `FolderPicker.tsx` in Add a Vault (#430), which the first-run
checklist (#419) reuses.

**Consumed dependencies:** the Vault collection registry's `load`,
`vault_path` and crate-private `ensure_outside_instance_state`.

**Coordination paths:** `src/lib.rs`, `src/app_state.rs`
(`vault_mount_root`), `src/server.rs` (the route, its web-token gate and demo
refusal, and the field from `AppConfig::vault_source`),
`src/handlers/mod.rs`, `src/vault_registry.rs` (the widened check).

**Invariants:** read-only: it opens no file and writes nothing; it resolves the
configured root once and follows no symlink below it, so it never leaves the
root and cannot loop; a requested path that is absolute or contains `..` is
refused; hidden folders, folders that would overlap instance state, and
non-UTF-8 names (counted) are left out; responses carry folder names and
counts only, never file names or content; it is not exposed over MCP; the
registry's containment rule is reused, not reimplemented.

**Validation:** `cargo test folder_listing`, `cargo test handlers::folders`,
`cargo test server::tests::folders`, followed by the full backend checks.

### HTTP wire types

**Kind:** shared contract.

**Owned paths:** `src/api_types.rs`.

**Public contract:** the shared serialized request and response structures
defined here, including resolve, recent, stats, and graph shapes.
Endpoint-local wire types remain owned by their handlers, notably write types
in `handlers/vault_write.rs` and diagnostics types in
`handlers/diagnostics.rs`. The legacy `RefreshResponse` is retired along with
the scope-less refresh surface; Vault-scoped refresh control remains
separately owned.

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
- `src/vault/link_style.rs`
- `src/vault/links.rs`
- `src/vault/markdown_links.rs`
- `src/vault/paths.rs`
- `src/vault/types.rs`
- `src/vault/tests.rs`

**Public contract:** the intentional re-exports from `src/vault.rs`, notably
`VaultIndex`, note/tree/link types, path normalization helpers, layer and
exclusion types, `is_servable_asset`, and `split_wikilink_asset_body`. Nothing
here writes starter notes into a Vault (ADR-40). `VaultIndex`
additionally carries an asset index (`asset_paths`, `assets_by_name`) filled by
the same walk that collects the Markdown files, and `resolve_asset` reads it:
Obsidian's default link format writes an attachment embed as a bare filename and
resolves it by searching the vault, so a purely note-relative reading broke every
embed in a vault using one top-level attachments folder (#158). `is_servable_asset`
is shared with the read core's contained-resource seam
(`src/vault_read/assets.rs`), so resolution can never name a path the asset
route or the MCP `get_attachment` tool would refuse.
`resolve_note_link` answers a Markdown note link (ADR-28) through the same
path ladder (`resolve_path_ladder` in `paths.rs`, which `resolve_asset` now
calls too) over `note_paths`, the catalogue's notes keyed by path and by
filename. `markdown_links.rs` is the one scanner for Markdown links: the link
graph reads it, and so does the write layer's rename, move and delete
rewriting, through `rewrite_note_links`.
`link_style.rs` answers a Vault's link style (ADR-33) through
`vault_link_style`: `.obsidian/app.json` decides when it exists, read and
never written, and otherwise the Vault's existing links vote. The vote is
`count_link_forms` over the Vault's notes: `links::note_link_forms` counts one
note's wikilinks and `![[...]]` embeds against its Markdown note links and
local `![](...)` images, with the same readers the link graph uses, and a
process-wide memo keeps each note's counts until its size or modification
time changes, so a listing re-reads only changed notes. The scanner records
inline images as `MarkdownLink::Image` for that count only, and no rewriter
touches them.

**Consumed dependencies:** filesystem traversal and parsing; `cache::parse`
currently supplies content hashing to the index and, since #248, the shared
Markdown code-region scanner (`for_non_code_line`) the link reader uses to skip
fenced code blocks and inline code spans.

**Consumers:** cache population, handlers, MCP reads, write coordination,
watching, and application startup.

**Coordination paths:** `src/cache/**`, `src/vault_watcher.rs`,
`src/api_types.rs`, and adapters when a public vault type changes.

**Invariants:**

- Markdown files remain authoritative (ADR-01).
- Excluded/noise paths do not enter the index.
- Layer markers remain visible to classification even under broad exclusions.
- A note remains addressable while its layer is reported to callers.
- A backslash before a wikilink's alias pipe is syntax rather than part of the
  target, so `[[Note\|alias]]` - the form a Markdown table cell forces - names
  the same note as `[[Note|alias]]` in the link graph and in wikilink
  resolution. `src/vault/paths.rs` is the single home for that split, shared
  with the write layer's rewriters, and no escape reaches
  `normalize_link_target`, which would read it as a path separator (#252).
- A note link is a wikilink or a Markdown link whose destination, once any
  `#anchor` is removed, ends `.md` (ADR-28), and both feed the same outgoing
  and backlink records. A Markdown target resolves by path from the linking
  note's folder, never by the wikilink title rule, and the two rules must not
  be merged. Links in code, image syntax and unused reference definitions do
  not count. `frontend/src/components/note-page/markdownLinks.ts` repeats the
  recognition for the renderer and has to change with `markdown_links.rs`.

**Validation:** `cargo test vault` and the full backend checks.

### Vault mutation

**Kind:** product capability/domain core; safety-critical.

**Owned paths:**

- `src/vault/write.rs`
- `src/vault/write/assets.rs`
- `src/vault/write/attachments.rs`
- `src/vault/write/frontmatter.rs`
- `src/vault/write/fs_ops.rs`
- `src/vault/write/notes.rs`
- `src/vault/write/paths.rs`
- `src/vault/write/rewrites.rs`
- `src/vault/write/tags.rs`
- `src/vault/write/types.rs`
- `src/vault/write/tests.rs`

**Public contract:** write functions and result/error types re-exported from
`src/vault.rs`, including note CRUD-by-move, section/edit primitives,
shallow frontmatter merge (`update_note_frontmatter`), attachment
operations, the Vault-wide tag rename (`rename_tag` with `TagRename`,
`TagRenameNote`, `TagRenameError`, and `UnsupportedTagNote`), the Vault-wide
tag delete (`delete_tag` with `TagDelete`, `TagDeleteNote`, `TagDeleteError`,
and `NestedTag`, #258), the path-addressed note checks a note upload makes
before its bytes arrive (`note_target` with `NoteTarget`,
`note_exists_conflict`, and `check_note_content_hash`, #303), allowed
attachment extensions, `WriteOutcome`, `WriteError`, and `UnrewritableNote`,
which `WriteError::LinkRewriteUnsupported` carries (#360).
`frontmatter.rs` is internal to the layer: `edit_frontmatter_block` is a plain
`pub(super)` function, deliberately not a trait or an extension point
(ADR-13), and its second caller is the Vault-wide tag rename and delete in
`tags.rs` (#242, #258).

**Consumed dependencies:** vault index/types, the local filesystem, the
filesystem rename-flag capability (`src/rename_flags.rs`) that decides whether
a commit can use `renameat2`'s flags here, and
`cache::parse` for content hashing, frontmatter span parsing, and the shared
Markdown code-region scanner (`for_non_code_line`, `parse_fence_marker`) that
keeps every rewriter's idea of a code block identical to the indexer's. The
tag rename consumes `cache::parse::inline_tags`, the positional form of the
indexer's own inline-tag recognition, and `extract_tags` to read each rewritten
note back, plus `search::tag_matches` for what a tag and its namespace match. It
also consumes the vault read model's wikilink body splits
(`split_wikilink_note_body`, `split_wikilink_asset_body`), so a rewriter and
the link graph can never disagree about where a target ends (#252).

**Consumers:** the Vault-qualified mutation core (`src/vault_mutation.rs`),
which since #186 is the sole caller of every write primitive. The one
exception is `list_note_attachments`, a read that lives here for its path
handling and is called by the MCP `list_note_attachments` tool through the
core's own `write_operation_error` translation.

**Coordination paths:** `src/vault_mutation.rs`, Git write records, frontend
write API/types, and configuration for archive or upload limits.

**Invariants:**

- All HTTP and MCP mutations use this shared layer (ADR-03).
- Optimistic concurrency uses the expected content hash.
- A conditional write commits by exchanging its temporary sidecar with the
  destination wherever the filesystem can do that, so past that exchange the
  outcome a caller is told depends on whether the undo put the old bytes back.
  Where the filesystem cannot, the commit falls back to check-then-rename and
  the exchange-specific outcomes below do not arise, because nothing is
  committed before the check (ADR-26). The fallback keeps every error code and
  message shape, and loses only atomicity against a writer outside Hatchdoor.
  Which path runs is decided by the rename-flag probe, never by a bare
  `EINVAL`. An undo that succeeds reports the
  original failure; one that cannot run leaves the write committed and
  unverified, and reports `recovery_required` rather than a plain failure, so
  no caller is told a write did not land when it did. A `recovery_required`
  message is the one write failure that reaches an API client unsanitized, so
  it names the note and its sidecar without the directories above them.
- A write that names part of a note edits that part and leaves every other byte
  alone (ADR-22). `update_note_frontmatter` rewrites only the lines its named
  keys own, so key order, one-line versus block lists, indentation, quoting,
  comments, and blank lines survive untouched; a replaced value inherits the
  shape its author used, and a new key is appended with any list on one line.
  A named key that cannot be located and replaced unambiguously refuses the
  whole call by name. The edited block is then reparsed and compared against the
  intended merge before anything reaches disk, which is the backstop for a block
  the key scanner reads differently from a YAML parser, an indented top-level
  mapping being the example, and the one case where an unnamed key can refuse a
  call (#257). `append_note`, `edit_note` and `replace_section` never pass the
  note through the whole-content preparation step: only the caller's text is
  converted, to the note's majority line ending (CRLF against lone LF, LF on a
  tie, the rule `LineEnding::of` shares with the frontend's `detectLineEnding`
  in `frontend/src/lib/sourceMap.ts`, so the two change together), and each converted ending or added separating line break is reported
  in `quality_warnings` (#316). Whole-content writes
  (`create_note`, `update_note`) keep their line-ending and trailing-newline
  normalisation; ADR-22 constrains partial writes only.
- A tag rename is all or nothing (#242). It plans every edit first and writes
  only when `expected_plan_hash` equals the fingerprint of a plan made again
  under the same lock; the fingerprint is a hash of the edits and of the text
  they replace, never server state. A note it cannot edit in place refuses the
  whole plan: a frontmatter `tags` list the shared editor would restyle (the
  editor is first handed the list unchanged and must reproduce the block byte
  for byte), an inline tag split by a code span, an inline tag renamed into a
  name without a namespace, or any note whose rewritten text the indexer would
  not read back as exactly the promised tags. The writes go through
  `MutationJournal` one note at a time, and a failure restores every note
  already written.
- A tag delete (#258) shares the rename's handshake, frontmatter editor,
  read-back backstop and journal, with its own fingerprint. It removes one
  exact tag from frontmatter `tags` values and never edits a note body. It is
  refused as a whole while any note carries a tag nested under the target or
  carries the target inline, because tag search is hierarchical and reads
  bodies, and the delete promises that a search for the tag then finds
  nothing. A list it empties stays as `tags: []`: the key is kept and the
  block never stripped, unlike `update_note_frontmatter` deleting its last key.
- Delete is recoverable trash; archive is move-based (ADR-11).
- A rewritten backlink keeps its shape, and a link that resolved before a
  move still resolves after it: a path-qualified link takes the new full path,
  and any target without a folder path, whether a title, a title whose
  punctuation drifted from the filename, or a slug, stays bare as the note's
  new title (#256). The bare form is used only while the new title names
  exactly one note, and falls back to the full path otherwise (#235). An escaped alias pipe is part of that form: the
  rewrite retargets `[[Old\|alias]]` and hands the escape back, so the table
  cell it protects stays valid Markdown (#252).
- Markdown note links are retargeted in the same pass as wikilinks, so one
  file gets one rewrite (ADR-28). A retargeted path keeps the author's form
  wherever that form still reaches the note after the move, checked against a
  relocated copy of the note-path catalogue: bare stays bare, `/`-anchored
  stays anchored, a path from the Vault root stays so, and a note-relative
  path is recomputed from the linking note's folder; otherwise the
  note-relative path, then the anchored one, is written. Only the path part
  changes, never the link text, anchor or title; a reference link changes
  through its definition line. The moving note's own Markdown links are
  repointed from its destination folder. Delete removes each link to the note
  and keeps its text as plain prose, removing a reference definition's line
  along with its uses. A written path escapes only whitespace, `%`, `#`,
  brackets and parentheses.
- The note being renamed or moved is one more note holding links to the target,
  so its own body follows that same rule (#254). Its rewrite is keyed to the
  note's destination path, because the note has already moved by the time
  rewrites are applied, and it composes with the stationary-asset rewrite that
  targets the same path rather than replacing it. `rewritten_notes` still
  counts only the other notes, so a rename whose one stale link is the note's
  own reports zero. Delete leaves the trashed body's self-link as written.
- A note the link planners cannot read is skipped, never rewritten (#360).
  One that cannot be opened at all, a dangling `.md` symlink being the
  example, holds no link the index knows of, as `build_link_graph` skips it
  too. One that is not valid UTF-8 is planned over its lossy text: skipped
  when nothing in it would change, and otherwise the whole rename, move,
  archive, delete or attachment move is refused before anything is written,
  folders included, naming every such note under `link_rewrite_unsupported`.
  The tag rename and delete skip an unopenable note the same way.
- An asset travels with its note only from inside the note's own folder (#225),
  and an occupied destination refuses the whole write - except where that
  destination is the asset's own file, which is a move to nowhere rather than a
  collision: no move and no rewrite are planned for it, and `moved_assets`
  counts what actually moved (#238).
- Paths remain within the canonical vault root.
- The upload allowlist is ingest policy only. `import_attachment` and the HTTP
  upload route apply it; `move_attachment`, `rename_attachment` and
  `delete_attachment` do not, because they act on bytes the Vault already
  stores (#247). What those three refuse instead is a Markdown target, which
  belongs to the note tools, and anything under `.git`, which is the Vault's
  own repository rather than content. Any non-Markdown file with an extension
  counts as an asset reference for listing and for note-move travel; an
  extension is still required, since that is what separates a file from a
  wikilink to a note, and the existing existence check is what keeps a dotted
  note title out of the plan.
- What the Vault excludes as noise is not an attachment either, and since #247
  `reject_noise_write` is applied to an attachment operation's source as well
  as its destination. The upload allowlist was the only thing keeping these
  tools out of `.obsidian/`; that protection now comes from the policy the
  Vault already states.
- Layer marker and excluded/noise writes remain protected at adapter and domain
  boundaries as applicable.
- Concurrent writes to one Vault are serialized through
  `VaultControlBlock::acquire_mutation`, a genuine per-Vault lock and the only
  vault write lock there is — the instance-wide `AppState::vault_write_lock`
  went with the legacy Git-sync task (#185). Since #186 that lock is taken in
  exactly one place, `src/vault_mutation.rs`, on behalf of both surfaces; the
  known gap #103 opened is closed.

**Validation:** `cargo test vault::write`, `cargo test vault_mutation`, and
the full backend checks.

### Vault-qualified read projections

**Kind:** product capability/domain core.

**Owned paths:** `src/vault_read.rs`, `src/vault_read/assets.rs`, `src/vault_read/query.rs`, `src/vault_read/saved_query.rs`.

**Public contract:** `VaultReadCore`, `BrowseSurface`, `AssetSurface`,
`NoteDownload` (#342), explicit `VaultScope`,
the common `VaultReadProjection` envelope, participant state/error types, and
Vault-qualified exact-note, tree, statistics, graph, and recent-note
projections, plus `VaultReadCore::saved_queries` and its wire types
(`SavedQueriesResponse`, `SavedQueryResult`, `SavedQueryOutcome`,
`SavedQueryTable`, `SavedQueryColumn`, `SavedQueryRow`, `SavedQueryTruncation`,
`SavedQueryTruncationReason`), and `VaultReadCore::saved_query` with its
wire types (`SavedQueryEvaluation`, `SavedQueryRows`). `VaultQualifiedNote`
carries `saved_queries: Vec<SavedQuerySummary>` (#277). `BrowseSurface` names which layer surface a caller may read.
`Everything` is the established behavior and stays the default: a layer demotes
a Note from the default *search* surface only, and an operator still reaches it
by slug, in the explorer, and on the graph. `DefaultOnly` is demo mode's clamp
(#109), selected through `BrowseSurface::for_demo_mode` and applied by both
`VaultReadCore::on_surface` and `search::vault_scoped::VaultSearchCore::on_surface`:
a demo has no operator and no layer toggle, so a demoted Note is withheld from
exact reads, links, resolve, and download (as an ordinary not-found, so
withheld is indistinguishable from absent), and `BrowseSurface::restrict` drops
its rows from a published snapshot before any projection reads it, covering
tree, graph, recent, statistics, query, and a surviving search hit's outbound
links. A
link is dropped when either endpoint is withheld, since a surviving edge would
name the hidden Note. `BrowseSurface::layer_selection` parses the caller's raw
comma-separated layer tokens and clamps a restricted surface's selection to the
default surface, so the `layers=` query is not an escape hatch; it and
`VaultScope::parse` are the one implementation both adapters use, together with
the `clamp_recent_limit`/`clamp_search_limit`/`clamp_search_per_note_cap`
bounds and the shared `note_not_found` failure (#188). `statistics_detail` (#137) is the exact-read counterpart to the
lean collection `statistics` projection: it returns `VaultQualifiedStats`
directly (never wrapped in `VaultReadProjection`, like `exact_note`), scoped
to exactly one Vault via `collection`'s `VaultScope::One` gating, computing
every legacy `VaultStatsResponse` field from the same published snapshot
`statistics`/`trees`/`graphs` read rather than the single-Vault-shaped SQL
cache tables the retired scope-less statistics query read. Its
`activity_by_month` is a window rather than a list of findings (#298): exactly
six `MonthActivity` entries, oldest first, one per calendar month ending at the
current UTC month, zero-filled where no Note was created, and counting no Note
from outside the window. Each entry's `created_count` counts Notes by created
date (#300, ADR-29): a `created` property that reads as a date, else, for a
Git-backed Vault, the commit that first added the Note through the Vault's
`NoteHistory` (`git/note_history.rs`, held on its control block), else
modification time. `created_date_status` reports `estimated` when history
should have dated a Note and could not (shallow, unreadable) and `reading`
while the walk is still running; the recent lists stay on modification
time. `VaultScope`
serializes as the flat scalar
`docs/migrations/vault-scoped-clients.md`'s envelope documents — the Vault
ID's canonical text for `One`, or the literal `"all"` — mirroring exactly what
a caller passes as the `scope` path segment, rather than serde's derived
externally-tagged shape. `resolve_wikilinks` resolves every target in a batch
against one
catalog build, rather than one build per target. `resolve_batch`
generalizes it to note *and* asset targets over that same one build (#158),
taking the embedding note's Vault-relative directory because an asset target
resolves relative to the note that names it; assets are returned as
Vault-relative paths, since an asset has no slug. Resolution does not apply
the browse surface: an embed only resolves for a caller already reading the
note that contains it. Fetching the resolved asset does, through
`asset_on_surface`, so on a demo an embed pointing under a demoted directory
resolves but its fetch is refused (#154). `vault_directory`
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
`asset_on_surface` applies the complete demo-readable policy to a contained
asset's Vault-relative path: it must occur in the Vault's asset catalog
(which has already applied configured and built-in noise exclusions) and
survive `BrowseSurface` layer selection. The catalog is the `IndexedAssets`
the Vault's latest Index turn kept on its control block (#377), so a demo
answers a page of embeds without walking the Vault per image, and a file
added, removed or moved under a layer marker changes the answer when the
turn after it lands; before a Vault's first turn has scanned, a one-off
`authoritative_catalog` build answers instead. A file gone from disk is still
refused by `contained_asset`'s own containment and regular-file checks. A demo therefore cannot bypass
its default-only Note surface by requesting a demoted, noise, or excluded asset
directly; ordinary `Everything` reads retain the legacy contained-asset
behavior. `AssetSurface` is that same decision captured from one index, for a
caller checking many paths: `asset_on_surface` delegates to it, and the note
download carries one (#342) so a demo zip never holds an asset the asset route
refuses.
`query_notes` (#274) selects the Notes whose tags, path, and frontmatter
properties satisfy every stated condition, restoring the capability the
multi-Vault rewrite retired. It selects rather than ranks, and nothing in
`src/vault_read/query.rs` reaches the retrieval path: conditions are tested
against the published snapshot's structural rows, so a Vault whose generation
carries no vectors answers in full and there is no score to order by. The
condition vocabulary is `NoteQueryCondition` — a tag (nested-aware), a
Vault-relative `path_prefix` (segment-aware and case-insensitive), or a
property tested by one `PropertyOperator` — and the `NoteQueryResponse` rows are
Vault-qualified, projected with the properties the caller named, ordered by
path then Vault then slug, and flagged `truncated` when the clamped `limit`
held Notes back. `CompiledQuery::compile` validates the whole query before any
Vault is resolved, so a malformed one is the `invalid_query` refusal at every
scope; the two shared tag primitives it normalises and matches with,
`search::normalize_tag_path` and `search::tag_matches`, live in the shared
search vocabulary so a query and the `#tag` search shorthand cannot disagree
about what a nested tag is. The `limit` is clamped inside `compile` rather than by each adapter, so a
caller cannot reach the core with a zero limit and be told its complete answer
was truncated.
`saved_queries` (#275, ADR-21) evaluates every saved query in one Note: each
fenced `base` block in its authoritative Markdown, optionally named by a
`<!-- hatchdoor-query: name -->` marker separated from it by blank lines only.
`src/vault_read/saved_query.rs` parses a documented subset of the Obsidian
Bases syntax and compiles its filters into the same private
`query::CompiledCondition` tree `query_notes` evaluates, which gained `All`,
`Any` and `Not` combinators and a `Subject` (a property, or the file's name,
basename, path or folder) for exactly that purpose, so there is one condition
engine, not two. A construct outside the subset that could change which rows
appear refuses that saved query with a `SavedQueryRefusal` carrying the
construct and a sentence naming it, rather than being partly applied; a
presentation-only one (`groupBy`, `summaries`, a view type other than `table`)
is set aside into the outcome's `ignored` list and every row is still drawn
(#276). The rows come from the Vault's published
snapshot, inside the usual projection envelope, and are never written anywhere:
a result is recomputed on every call, so a filter against `now()` answers
afresh with no file change, and search, backlinks, statistics and the graph
never see it. The Vault is the Note's own and there is no scope argument.
Rows are ordered by title, then path and slug, and held to
`SavedQueryCeiling::ENFORCED` (notes scanned, rows returned) as well as the
view's own `limit`. The scan ceiling is one budget for the whole Note, spent by
each evaluated query, so repeating a block cannot multiply the work of one read;
a query past it answers `stopped` rather than a partial table, and a Note holds
at most `MAX_SAVED_QUERIES_PER_NOTE`. `SavedQueryOutcome` has separate
`populated`, `empty`, `refused` and `stopped` variants, and only
`SavedQueryOutcome::evaluated` builds the first two, choosing `empty` exactly
when no row qualified, so zero rows never arrive without a state saying why.
Marker problems belong to the Note, not to one query, so they are reported
beside the results in `SavedQueriesResponse::marker_problems`: a marker with no
`base` block after it (`orphaned`, with its file line), a name that is not a
slug (`unusable_name`; that block is unnamed), and a name claimed by several
blocks (`duplicate_name`; each keeps the name it claims, and the report is what
tells an addresser it names none of them). None changes a row. Two strings that both read as a date
or date-time compare as instants in the shared `compare`, which `query_notes`
uses too, so `now()` orders correctly against `2026-09-18 10:00` or a zoned
timestamp; any other pair compares byte-wise as before.
`saved_query` (#277) addresses one saved query in a Note by its marker name and
evaluates only that one, with the whole scan budget, against the Note's own
Vault. The name may be omitted only when the Note holds exactly one. Selection
never falls back to position, so reordering a Note cannot change what a name
answers, and each way a request misses is its own `VaultReadError` code, all
listed in `public_code` and none retryable: `no_saved_queries`,
`saved_query_name_required` (the message lists the names),
`saved_query_not_found`, `saved_query_name_ambiguous` (the `duplicate_name`
collision), and, from the evaluation itself, `saved_query_refused` (naming the
construct) and `saved_query_stopped`. Only `populated` and `empty` reach
`SavedQueryRows`, so a refused or stopped definition cannot arrive as rows.
`VaultQualifiedNote::new`, used by every exact Note read, fills
`saved_queries` with the Note's saved queries by the name each may be addressed
by, `null` for none or an unusable one, parsed from the Markdown and never
evaluated, so both the HTTP note read and `get_note` report them while
`note.content` stays the authoritative file (ADR-21 part 4).
`VaultReadProjection::map` / `try_map` and the private `one_vault` carry an
envelope's freshness onto a single-Vault datum.
`exact_note_frontmatter` and `note_attachments` are the surface-gated
counterparts of the frontmatter and attachment-listing reads the MCP tools used
to answer from a raw index build of their own (#188); both return `Ok(None)`
for a Note this surface withholds, indistinguishable from an absent one.
`VaultNoteFrontmatter` carries `content_hash` (#227), computed by
`cache::parse::content_hash` — the one canonical helper every write receipt and
every `expected_content_hash` comparison uses — over the content the
frontmatter read has already loaded. It is therefore identical to the hash
`exact_note` reports for that Note at that instant and costs no extra
filesystem read, and it is not optional: the hash covers the whole file, so a
Note with no frontmatter block still has one to report.
`vault_capabilities` reports one Vault's own mutation/sync posture under the
same gate, for an adapter describing a Vault rather than reading it.
`contained_asset` is the single home for the contained-resource policy both
surfaces answer on: the Vault gate, path containment against the canonical
root, the servable-extension allow-list, the content-type table, the response
bound, and `asset_on_surface`. Its primitives stay private to
`src/vault_read/assets.rs`; adapters see only `ResolvedAsset`,
`AssetPathError` (which owns each outcome's stable `code` and message, while
the HTTP status stays in `handlers/assets.rs`), `AssetReadError`, and
`encode_relative_path`. `VaultResolveResponse` is the wikilink-resolution
projection, relocated here from `handlers/vault_content.rs` in #188 so both
adapters serialize the same type. `VaultReads` is the owned handle that runs a
read off the async runtime (`OffloadedReadError` separates the Vault's own
structured failure from a blocking task that never completed), so neither
adapter re-implements the clone-and-`spawn_blocking` prologue — MCP had drifted
into running index builds and filesystem reads straight on a tokio worker.
`VaultReadError::public_code`/`into_operation_error` give one translation from
the core's internal spellings to the stable `{code, message, vault_id?,
retryable}` object both surfaces report.
`trees` takes a `TreeScope` alongside the Vault scope (#192): the folder it
starts from, how far below it descends, and whether Notes appear at all, with
`TreeScope::default()` the whole Vault every caller read before. The narrowing
lives here rather than in an adapter, so the HTTP route — which passes the
default and always will, since the explorer draws the entire tree — and
`get_tree` share one implementation. A folder the Vault does not have is the
non-retryable `folder_not_found`, never an empty tree, so a mistyped name and
an empty folder stay distinguishable; under `all` it lands on the participant,
the same route any other non-participation takes, which is why `collection`'s
per-Vault projection is fallible — and `trees` raises it back to a refusal when
no Vault produced a tree, so the blur does not simply move up to the collection.
Zero enabled Vaults stays the empty projection it has always been. `VaultExplorerFolder` reports `note_count`
(the Notes directly inside it) and marks `truncated` when `max_depth` held it
back; `VaultExplorerNote` carries no `vault_id`, because the `VaultTree` around
it already does. Flat projections that mix Vaults in one list —
`VaultRecentNote`, search hits — keep theirs.
`exact_note_for_download` returns a `NoteDownload`: a Note, its containing
directory, and the Vault's `AssetSurface`, from one Vault control-block fetch
and one index build. That is required whenever a caller
needs both, since a concurrent Vault edit reconciles a *replacement* control
block rather than mutating the current one in place, so two independent
`exact_note`/`vault_directory` calls could otherwise observe different Vault
generations. The private `control_and_catalog` seam shares the
control-block-then-catalog-build sequence between `catalog` and
`exact_note_for_download` so the two cannot diverge on identical failure
conditions.
Every exact read except `exact_note_links` builds only the Vault's catalog
(`VaultControlBlock::authoritative_catalog`), which walks paths and reads no
note's content (#361): a note read, a frontmatter read, wikilink resolution
(single and batch), attachment listing, download and the demo asset check
cost one directory walk plus the requested Note's own file (the demo asset
check costs none once the Vault has been indexed, #377). The Note's
content and `content_hash` are read from disk on every call and never
cached. `exact_note_links` alone needs the link graph and takes it from
`VaultControlBlock::linked_index`, which reuses the graph until the Vault
reports a change (see Runtime composition). The browse surface is applied
after the graph is fetched, so one kept graph serves both surfaces.

**Consumed dependencies:** the Vault runtime's authoritative per-Vault catalog
and kept link graph,
the shared cache's published Vault snapshot seam, existing Vault note/link
types, and Runtime Search's two tag primitives (`normalize_tag_path`,
`tag_matches`) for a query's tag condition. That last one is a dependency on
the shared search *vocabulary*, not on retrieval: nothing here calls
`VaultSearchCore`. `statistics_detail` also reads a Git-backed Vault's
`git::NoteHistory` through its control block to date notes (#300).

**Consumers:** `handlers/vault_content.rs` (exact note/link/resolve reads,
`saved_queries`,
`vault_directory`, and the contained-asset route),
`handlers/vault_collection_reads.rs` (the collection-read projections `trees`,
`statistics`, `graphs`, `recently_modified`), and — since #188 — `mcp/tools/read.rs`
for every one of the Vault read tools (`mcp::tools::READ_OPS`). All three are thin adapters with
no read domain logic of their own. The core has no adapter or route ownership.

**Coordination paths:** `src/cache/vault_snapshots.rs` for read-only
Vault-qualified snapshot rows, `src/cache/mod.rs` for the crate-private seam,
`src/vault_runtime.rs` for the authoritative exact-read catalog, the kept
link graph, and the Index turn's retained `IndexedAssets`.

**Invariants:**

- Exact reads inspect the requested Vault's Markdown directory; SQLite remains
  a disposable projection (ADR-01).
- Note content and its hash are never older than disk. Links and backlinks
  are the one exception the maintainer accepted (#361): after an edit made
  outside Hatchdoor they may lag disk until the watcher reports the change,
  at most `WATCH_MAX_DEBOUNCE` after the burst began. A Hatchdoor write never
  lags, and a Vault with no running watcher never serves a kept graph. The
  bound is only as good as the watcher: an edit inotify never sees (another
  host writing to a network mount, a subdirectory past `max_user_watches`, a
  Vault root replaced underneath it) leaves the graph stale until the next
  Hatchdoor write or watcher replacement, the same limit search indexing
  already has.
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
- A narrowed tree read never blurs "no such folder" into "empty folder", and a
  folder the depth limit held back says so rather than reading as a leaf.

**Validation:** `cargo test vault_read`, focused cache snapshot tests, and the
full backend checks.

### Vault-qualified mutation core

**Kind:** product capability/domain core; safety-critical.

**Owned paths:** `src/vault_mutation.rs`, `src/vault_error.rs`.

**Public contract:** `VaultMutationCore`, `VaultMutation`, `ensure_mutable`,
`NoteWriteOutcome`, and the transport-neutral `VaultOperationError`. ADR-19
makes this the only seam a write adapter crosses, so the core owns everything
the HTTP and MCP write adapters used to repeat around a `vault/write`
primitive: resolving the Vault ID to a control block and refusing a missing,
disabled, or runtime-less Vault; the mutation capability check; the per-Vault
mutation lock; building the authoritative index off the async runtime;
resolving the slug to an entry; refusing a write to a path this Vault's own
exclusion patterns would make invisible; resolving the archive prefix from the
Vault's own archive folder or the instance default; running the blocking write
off the async runtime; recording what that write did in the Vault's
`git::WriteLedger`, so the commit that eventually records it can say so
(#249); marking the Vault's published snapshot stale and asking for its Index
and commit turns itself, rather than relying on a watcher (#324); and returning `NoteWriteOutcome` or a structured
`VaultOperationError`. `VaultMutation::with_commit_summary` carries the
caller's one-line description of the change into that record; the private
`RecordedWrite` trait is what lets `run_write` build the record once for all
seventeen primitives instead of at each of them. `NoteWriteOutcome` carries the note's resulting layer,
resolved from the `LayerMap` the write's own pre-write index build already
holds rather than from a post-write rescan (#101). `VaultMutationCore` carries a one-shot form — gate, lock, write — for each of
the seventeen primitives, which is what a standalone caller wants:
`create_note`, `update_note`, `append_to_note`, `edit_note`,
`replace_section`, `update_frontmatter`, `rename_note`, `move_note`,
`move_rename_note`, `archive_note`, `delete_note`, `import_attachment`,
`move_attachment`, `rename_attachment`, `delete_attachment`, `rename_tag`, and
`delete_tag`. `check_attachment_import` is not a mutation: it answers, without
writing, locking, or recording, whether `import_attachment` would refuse a
target before its bytes arrive (the Vault gate, marker and noise refusals, the
path and extension checks, and an existing file that may not be replaced), so
an upload transfer link can be refused when minted (#310). `check_note_upload`
is its counterpart for a note upload (ADR-32, #303): the Vault gate, marker and
noise refusals and `create_note`'s path checks, then with no expected hash a
note already at the path (`write_conflict`), or with one a missing note
(`note_not_found`) or a stale hash (`write_conflict`). `upload_note` is the
redemption: it refuses bytes over the limit or not UTF-8, then runs the
`create_note` write without overwrite, or, under an expected hash, the
`update_note` write (`replace_entry`, shared with `update_note`) on the note it
finds at that path, so both carry the same refusals, normalisation, warnings,
ledger entry and index request as the tools. `is_note_upload_target` is the
one rule for which uploads are notes: a filename ending in `.md`, in any case. Both note-upload paths write the target with that extension in lower case (`note_upload_path`), so `Report.MD` lands at `Report.md`.
`rename_tag` is
the one Vault-wide mutation: without an expected plan hash it plans off the
async runtime and records nothing; with one it
records a single ledger entry for every note it rewrote, so a synced Vault
commits the rename once. `tag_rename_error` maps its three refusals onto their
own codes (`invalid_tag_name`, `tag_shape_unsupported`,
`tag_rename_plan_stale`) and its write failures onto `write_operation_error`.
`delete_tag` (#258) follows the same plan-then-apply path and records one
ledger entry, `delete tag "#<tag>"`; `tag_delete_error` maps its refusals onto
`invalid_tag_name`, `tag_has_nested_tags`, `tag_used_inline`,
`tag_shape_unsupported`, and `tag_delete_plan_stale`.
It also answers `write_capabilities`, which deliberately does *not* gate on mutability:
a Vault that refuses writes has to answer that question rather than fail it.
`WriteCapabilities` carries three answers, not two: `mutate_capable`,
`vault_writable`, and `atomic_compare_and_swap`, which reports whether the
Vault's filesystem can commit a conditional write in one step. That third one
answers for the filesystem rather than for the Vault's permissions, so it is
`None` only where the filesystem could not be asked and is never `Some(false)`
for a Vault that is merely read-only (ADR-26, #345). An I/O write failure and
a `write_recovery_required` failure are both logged server-side before being
mapped, so neither reaches only the client's log. `write_operation_error`
maps a link rewrite the planners refused (#360) onto
`link_rewrite_unsupported`, not retryable, with every affected note named in
the message; the HTTP adapter answers it with `409`. A caller whose critical section spans several
operations on one Vault builds a `VaultMutation` with `VaultMutation::gated`
and takes the lock itself through its `acquire_mutation`: the MCP `batch` tool
holds one Vault's lock for a whole call, and `tokio::sync::Mutex` is not
reentrant, so an operation on a `VaultMutation` never re-takes the lock.
Reusing a control block the adapter already resolved also keeps every
operation in one batch on a single Vault generation. `VaultOperationError` is the `{code, message, vault_id?, retryable}`
envelope every surface already reported; it was the HTTP adapter's
`VaultApiError`, which remains as an alias in `handlers/vaults.rs` (with the
axum-shaped `respond`) — the spelling the sibling `/api/v1/vaults/...`
adapters use, now that #187 has moved the collection routes onto the core's
own name.

**Scope:** #184 proved the shape on `update_note` and `archive_note`; #186
brought the remaining thirteen primitives and write-capability discovery here,
and the adapters' own index-build, entry-lookup, marker- and noise-refusal,
filename-replacement, and write-error helpers disappeared with them. The free
function `write_operation_error` is public because the MCP
`list_note_attachments` read tool calls a `vault/write` function without being
a mutation and must not grow a second copy of that translation.

**Consumed dependencies:** the filesystem rename-flag capability
(`src/rename_flags.rs`), which `write_capabilities` asks for the Vault's
`atomic_compare_and_swap` answer, `vault/write` primitives (unchanged),
`VaultReadCore::control_block` for the Vault gate, `VaultControlBlock`'s
authoritative index and mutation lock, `VaultControlBlock::mark_snapshot_behind_write`
(which labels the Vault's published SQLite snapshot stale through the cache's
`mark_vault_snapshot_behind_write`) and `VaultControlBlock::report_write` (which
sends the Vault ID on the watcher intent channel so the server requests Commit
and Index turns) (#324), `AppState::vault_archive_prefix`, and the live
settings snapshot.

**Consumers:** `handlers/vault_write.rs` (all eight routes) and
`mcp/tools/write.rs` (every write tool standalone, and every one except
`rename_tag` and `delete_tag` inside `batch`).
Each is a wire-shaping adapter: it parses transport input, calls this core
once, and maps the typed outcome or the structured error onto a status code or
a JSON-RPC failure. The core has no route or tool ownership.

**Coordination paths:** `src/lib.rs` (module export),
`src/handlers/vaults.rs` (the `VaultApiError` alias and its axum `respond`).

**Invariants:**

- Writes stay inside `vault/write` (ADR-03); this core orchestrates, never
  implements, a mutation.
- Optimistic concurrency by expected content hash is unchanged, as is the
  `batch` hash chain.
- No trait seam formalises the core; it is a plain struct (ADR-13).
- Blocking work is offloaded here, so every surface offloads it.
- A caller holding the mutation lock is what serializes writes to one Vault;
  the core never re-takes a lock a caller already holds. A caller needing more
  than one Vault's lock at once takes them sorted by Vault ID, before any work
  runs (ADR-25); `batch` is the only such caller.
- A planned text rewrite commits against the content hash read when the plan
  was built (`TextRewrite::original_hash`), never against the journal's own
  re-read, so a concurrent save landing part-way through a multi-note apply is
  a `Conflict` rather than a silent overwrite (#321).
- Wire shapes stay adapter-owned: HTTP sanitizes a `write_failed` message,
  MCP reports it, MCP reports `noise_excluded_write` and `layer_marker_write`
  at the protocol level as invalid parameters while HTTP answers `400`, and
  none of those meanings lives in the core.
- Argument-shaped complaints stay adapter-owned too, because the two
  transports word them differently: an empty required field, a `new_title`
  carrying a path separator, the `replace_section` mode spelling, and the MCP
  base64 envelope are all parsed before the core is called.

**Validation:** `cargo test vault_mutation`, the adapter mapping tests
(`cargo test handlers`, `cargo test mcp`, `cargo test server`), and the full
backend checks.

### Vault collection management

**Kind:** product capability/domain core.

**Owned paths:** `src/vault_management.rs`.

**Public contract:** `VaultCollectionManagement`, the collection wire types
(`VaultSummary`, `VaultDiscoveryResponse`, `VaultMutationResponse`,
`VaultScheduleResponse`, `RegistryRecoveryInfo`), the two definition inputs
(`CreateVaultRequest`, `EditVaultRequest`, with `HttpsCredentialsInput` and
the three-state `HttpsCredentialsPatch`), and `parse_vault_id`. This is the
one place a Vault definition changes, so it owns the sequence every change
runs — commit to the registry, reconcile the live runtime through its
foreground-mutation safe boundary, then answer from a single collection
snapshot so the reported `collection_revision` and the returned Vault's status
can never disagree — plus `list` (with its authenticated and demo
projections), `create`, `edit`, `set_enabled`, `disconnect`, the manual
`sync`/`retry`/`refresh` controls. Since #267 `sync`/`retry` choose the operation from what the Vault
actually has: a remote sync through `ManagedGitScheduler` for a Vault with a
remote, and a `VaultWorkKind::Commit` request (plus a clear of that Vault's
`git::CommitCooldown`, because an operator asking explicitly is exactly the
case suppression must not swallow) for one that keeps history but has no
remote. `capability_unavailable` narrowed with it: it now names only a Vault
with no Git at all, not every Vault with no remote.
`publish_recovery` (ADR-30) admits a `VaultWorkKind::Recovery` request only
while the Vault's runtime reports the `publish_recovery` capability, refusing
with `capability_unavailable` otherwise, and `VaultSummary` carries the
runtime's `recovery_branch` status on an authenticated read and withholds it
from the demo projection. `VaultSummary::index_turn` (ADR-35) copies the
runtime's place in the indexing lane onto both projections, the demo's
included, since it is status rather than deployment detail.

`list()` also fills `link_style` and `link_path_form` (ADR-33) on an
authenticated read, read from each active Vault's directory through
`vault::vault_link_style` on every listing, and leaves them absent for a
Vault it cannot read, in the demo projection and on mutation responses.
Without an Obsidian settings file that read walks the Vault's catalog and
re-reads every note changed since the last listing, so both adapters (`GET /api/v1/vaults`, MCP `list_vaults`) run `list()` on
the blocking pool.

`VaultSummary` carries two optional RFC 3339 UTC timestamps
alongside the status fields — `last_checked_at` and `next_attempt_at`, read
from `git::ManagedGitScheduler::polling_clock` — so a caller can tell a Vault
that polled and found nothing from one that is not polling at all, which the
status fields alone cannot express. `last_checked_at` reports when the last
interval-arming turn *finished*, whether it succeeded or failed: a failed
check is still a check, and `git`/`git_error` already say which it was, so
naming it after a sync would report one for a Vault that has only ever failed
to authenticate. Both are absent for a source with no remote to poll and in
the demo projection, which withholds operator deployment detail;
`last_checked_at` is additionally absent until a turn completes, while
`next_attempt_at` is always present for a tracked Vault, because one that has
never completed a turn is due immediately rather than unscheduled. Failures leave as the transport-neutral `VaultOperationError`
(ADR-19). Creating a Vault never writes into its folder: a `Local` Vault on an
empty directory stays empty (ADR-40). An edit
whose `https_credentials` was `Replace` additionally requests an immediate Git
turn and notifies a definition change, because `VaultDefinition` equality
cannot observe a credential value change (#97's and #98's reopening
findings).

Discovery reports registry recovery as `recovery` when the persisted registry
file itself is unreadable, and lists no Vaults then (#150). The
credential-replacement retry skips a disabled Vault, which would otherwise
gain a Git schedule entry that disconnect never deactivates (#325).

**Scope:** #187 moved this out of `handlers/vaults.rs`, where the seven MCP
management tools reached it by calling handler functions with hand-built axum
extractors and decoding the HTTP response body. No wire shape changed.

**Consumed dependencies:** `VaultRegistryStore::{load, add, edit, enable,
disable, disconnect}`, `VaultCollectionRuntime::{snapshot,
reconcile_and_reconstruct_and_wait_for_mutation_boundary, runtime,
notify_definition_changed, subscribe_revisions}`,
`ManagedGitScheduler::{sync_now, retry_now, polling_clock}`,
`vault_runtime_state::format_timestamp`, `VaultWorkCoordinator::request`,
`vault::{vault_link_style, count_link_forms}`, `VaultControlBlock::{vault_path,
authoritative_catalog}` for the link style, and `AppState`'s composed handles including `demo_mode`.

**Consumers:** `handlers/vaults.rs` (every `/api/v1/vaults` route) and
`mcp/tools/read.rs` (`list_vaults`, `create_vault`, `edit_vault`,
`enable_vault`, `disable_vault`, `disconnect_vault`, `sync_vault`,
`retry_vault`, `publish_recovery_branch`, `refresh_vault`).
`POST /api/v1/vaults/{vault_id}/recovery-branch` and `publish_recovery_branch`
pair onto `publish_recovery` the same way. `POST /api/v1/vaults/{vault_id}/refresh` and
the `refresh_vault` MCP tool (#228) are the same single call onto `refresh`,
the way sync and retry pair across the two surfaces. Each is a wire-shaping
adapter: it parses transport input, calls this core once, and maps the typed
response or the structured error onto a status code or a structured tool
error. No MCP tool calls a handler function or decodes an HTTP response for
Vault management. `mcp/results.rs` aliases the collection wire types as its
management tool result types, so the advertised `outputSchema` is generated
from the same structures the core returns.

**Coordination paths:** `src/lib.rs` (module export).

**Invariants:**

- HTTPS credentials never appear in any projection, error, or status;
  `credential_configured` is the only signal (#133).
- Demo mode lists only enabled Vaults and withholds `source`, exclusion
  patterns, archive folder, commit identity, and runtime error details (#109),
  and reports per-Vault `capabilities` as what an unauthenticated visitor may
  do rather than as derived: `mutate`, `pull`, `push`, `retry`, `commit`, and
  `sync` are false, because the demo guard refuses every route behind them
  (#243, extended by #267's two new capability flags). `browse` and
  `search` stay derived, and the four status fields, `local_content` included,
  keep describing the Vault.
- An instance-side failure is logged with its detail and reported with a
  sanitized message here, so neither surface can leak a filesystem path by
  skipping the scrubbing.
- Every registry mutation reconciles within the same call, so the collection
  revision and the SSE stream never lag a commit.
- Status codes, rejection wording, the demo-mode refusal, and the SSE
  `Event`/keep-alive framing stay adapter-owned; none of those meanings lives
  in the core. The revision channel the stream publishes from is reached
  through `subscribe_revisions` here, so the adapter never reaches past a core
  into the runtime (ADR-19).
- ADR-07, ADR-09, ADR-13, ADR-19.

**Validation:** `cargo test vault_management`, the adapter mapping tests
(`cargo test vaults`, `cargo test mcp`, `cargo test server`), and the full
backend checks.

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
and the methods implemented on `SqliteCache`. The read queries are the
Vault-qualified snapshot lookups, the evaluation binaries' `semantic_search`
and `fts_search_notes`, `read_note_by_slug`, and the link/wikilink queries;
the scope-less stats, explorer-tree, recently-modified, read-by-path,
demoted-layer, note-summary, health-check, graph, and layered/filtered
search variants are retired. The crate-private
`vault_snapshots` seam owns Vault-ID-qualified candidate publication,
stale/participation state, attempt ordering, and Vault-local disposal in the
shared cache. `mark_vault_snapshot_behind_write` is the one stale-marking
call that begins no attempt, so an Index turn already building still
publishes; its caller holds the Vault's mutation guard, which is what lets
that turn's own freshness verdict stay correct (#324). A published `VaultSnapshotRead` is structural: notes, links,
tags, and the layer catalog. It carries no chunks — search reads those and
their vectors through its own SQL rather than through a snapshot, so the
`vault_chunk_vectors` join every collection read used to pay for is gone —
and it carries note bodies only when the read asks for `NoteBodies::Load`.
Frontmatter properties are parsed unless the pinned read asks for
`NoteProperties::Omit`, which search does because it never returns them;
`vault_snapshot_embeds_demoted_layers_on` reads the generation's
`embed_layers` stamp on the same pinned read (#328).
The detailed stats report is the only caller that does, because it counts
words and images; bodies come back from the same pinned read as the note
list so a projection can never pair one generation's rows with another's
text. Publication carries the caller's freshness verdict rather than
assuming `Fresh`, and `MutationGuardHandoff` is how an Index turn hands its
Vault read lock through a build: released at the read/embed boundary, retaken
to answer that verdict under one acquisition with the publication it labels
(issue #223). `parse` is currently public and
also supplies parsing/hash behavior to vault indexing, and its
`frontmatter_span`/`parse_frontmatter_metadata` parsing to the shared write
layer's frontmatter merge, and `frontmatter_span` to the Bundled manual, which
strips each page's frontmatter. It is also the single home of the Markdown
code-region scanner: the crate-private `for_non_code_line`, which walks the
lines Markdown renders as prose, and `parse_fence_marker`, which recognizes a
fence delimiter. The Vault link reader and the asset-reference rewriter consume
`for_non_code_line`; the backlink, section, and asset-reference rewriters
consume `parse_fence_marker` for their own line-rebuilding loops, which must
preserve line endings and so cannot use the visiting form, as does the Bundled
manual's wikilink rewrite. It lives here
because tag extraction, link extraction, and rewriting have to agree on what
counts as code: an indexer that reads a hashtag inside a fenced block as a tag
while a rewrite refuses to touch it makes a Vault-wide tag rename look
half-applied (#248, unblocking #242). The copies merged here were behaviorally
identical, so consolidating them changed nothing; the point is that the next
correction lands in one place instead of three. Inline-tag recognition has
two forms over one walk: `extract_tags` stores what the index calls a tag, and
the crate-private `inline_tags` returns the same tags with the byte range each
occupies in the note, for the Vault-wide tag rename (`vault/write/tags.rs`,
#242) to edit. Both read prose through the same line walker as
`for_non_code_line`, so the rename cannot touch a hashtag the index would not
store, or miss one it would. A tag split by an inline code span is recognised
but has no range, because no single run of text spells it.
`ReadSnapshot` is the crate-private pinned-read seam used where participant
metadata and cache queries must observe one published generation.

**Consumed dependencies:** Vault IDs and index/types, chunking, embeddings, SQLite,
FTS5, and sqlite-vec.

**Consumers:** application state/reindexing, runtime composition's per-Vault
Index dispatch, the Vault-qualified mutation core (which reaches
`mark_vault_snapshot_behind_write` only through runtime composition's
`VaultControlBlock::mark_snapshot_behind_write`, #324), Vault-qualified read projections, the Vault-qualified search
core, handlers, MCP reads, evaluation tooling, and diagnostics.

**Coordination paths:** `src/app_state.rs`, `src/vault_runtime.rs`,
`src/vault_read.rs`, `src/search/**`, `src/vault/index.rs`, `src/chunk/**`,
and embedder identity/dimensions.

**Invariants:**

- SQLite is rebuildable and never authoritative (ADR-01).
- Keep embedded SQLite, FTS5, sqlite-vec, WAL, one writer, and pooled
  query-only reads (ADR-06).
- The reader pool is a ceiling on live SQLite handles, not a load-shedding
  policy. A caller at `MAX_READ_CONNECTIONS` waits up to `READ_LEASE_WAIT` for
  a slot and only then reports the pool as exhausted, so no read holds a slot
  across slow work that does not touch the database. Embedding in particular
  runs before the search core takes its snapshot: holding a slot across the
  embedder's inference lock let four concurrent searches starve every other
  read. Waiters are woken one at a time and not in arrival order.
- Schema or embedder identity mismatch rebuilds rather than mixing data.
- A refresh commits a coherent new read snapshot.
- Shared semantic vectors have one embedder identity and dimension; a mismatch
  wipes the disposable cache before any partial rebuild can participate.
  The cache-wide model epoch covers snapshot and legacy builders, and stamps
  the shared identity atomically with snapshot participation.
- Every shared snapshot row and relationship is Vault-ID-qualified; failed
  replacement retains the prior snapshot as stale, disabling removes only
  participation, and disconnect deletes only that Vault's disposable rows.
- Saved embedding progress (`vault_embedding_progress`, ADR-35) is disposable
  and never searchable. An Index turn saves the vectors it computes there as it
  goes, at least every `SAVE_EMBEDDINGS_EVERY` of embedding and whenever the
  build ends, without holding the Vault's mutation guard. A later turn reuses
  a saved vector only for an identical embedding input under the same
  embedder identity and embed-layers policy, and prunes rows its workload no
  longer needs, unless a note failed to read or prepare (then rows wait for
  the next clean pass or the next searchable publication). Each build
  discards every Vault's rows saved under another embedder identity. A
  searchable publication deletes the Vault's rows in its own
  transaction; a structure-only one keeps them. Saves belong to their
  turn's snapshot attempt and stop once it is superseded. Disconnect begins
  an attempt before it deletes them, so a turn still embedding a removed
  Vault neither saves nor publishes afterwards. Disconnect deletes them,
  disabling keeps them, and `snapshot_vault_ids` enumerates them so
  reconciliation can clean up a Vault removed before its first publication.
  A build given an `IndexYield` stops before its next chunk once it has
  embedded for its slice and another Vault waits; it publishes nothing,
  flushes its saved progress, and reports `SnapshotPublication::Paused`
  without marking the retained generation stale again.
  The table has no foreign key to `vault_snapshots`, whose row each
  publication deletes and re-inserts.
- A population pass drops every cached note row that will not still hold its
  slug when the pass ends - the notes that left the Vault and the notes whose
  slug moved to another path - before it writes any row. A slug is unique and
  migrates between paths whenever a note is added, moved, or renamed beside a
  same-named sibling, so releasing it late fails the whole turn (issue #226).
- Every line a Vault's build logs, the progress heartbeat thread's included,
  carries that Vault's `vault_id` and never its name, path, or remote. The ID
  travels on `BuildHandles` and the heartbeat receives the build's span
  explicitly, because a span does not follow work onto another thread
  (issue #155).

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
- `src/search/layer_selection.rs`
- `src/search/vault_scoped.rs`

**Public contract:** the shared search vocabulary `SearchMode` (what a caller
may request), `SearchResponseMode` (what a response reports ran, adding `tag`;
#354), `LayerSelection`, `LayerInfo`, `OutboundLink`, and the two crate-internal tag
primitives `normalize_tag_path` and `tag_matches` (#274). Those two say what a
tag is and what "nested under it" means, which the Vault-read core's metadata
query needs to answer a tag condition the way the indexer stored the tag.
`vault_scoped::tag_results` keeps its own inline copy of the same predicate,
because rewriting it would touch the retrieval path and cost an eval run
(ADR-15) for no behaviour change; the two are held in step by review rather
than by construction, so an edit to either is an edit to both. The Vault-qualified
shared-core contract is `VaultSearchCore`, `VaultSearchRequest`,
`VaultSearchResponse`, and `VaultSearchResult`; it uses the explicit
`VaultScope` and common projection/participant envelope from the Vault-read
core without owning any HTTP, MCP, or frontend adapter. `VaultSearchCore` is
the only search entry point: the scope-less single-Vault `run`, its retrieve
and assemble helpers, and its request/result/response types are retired.

**Consumed dependencies:** `SqliteCache`, its published Vault snapshot/cache
query seam, `Embedder`, the Vault collection runtime, the explicit Vault-read
scope/envelope, and vault metadata/types.

**Consumers:** `handlers/vault_collection_reads.rs` (the HTTP consumer of
`VaultSearchCore::search`), MCP search tools, offline evaluation runners,
`vault_read/query.rs` and the Vault-wide tag rename in `vault/write/tags.rs`
(`tag_matches` only; neither reaches the retrieval path),
and future Vault-scoped MCP adapters.

**Coordination paths:** `src/handlers/vault_collection_reads.rs`,
`src/mcp/tools/read.rs`, cache query methods, and frontend Search contracts.

**Invariants:**

- Runtime search defaults to pure semantic retrieval; hybrid and reranking stay
  offline (ADR-05).
- Layer selection must never widen the eligible result set.
- There is one retrieval path per mode. #210 removed the unreachable note
  metadata filters, the property projection, and the second semantic path they
  selected; a search result's metadata still serializes its `properties` as an
  empty object. Property search is a new feature carrying its own eval
  evidence, never a restoration of that code.
- Vault-qualified search globally ranks every usable Vault snapshot, caps by
  `(Vault ID, slug)`, and never deduplicates equal content or note names across
  Vaults. Staleness is participant status, not a relevance penalty.
- Semantic and keyword per-note-cap selection share one depth policy: each
  progressively enlarges its KNN or FTS candidate window only as needed,
  stopping at candidate exhaustion or the explicit 200-candidate ceiling, and
  the FTS query carries that window as a SQL `LIMIT` (#328). If that bounded
  window is dominated by capped notes, it returns the best available
  cap-compliant partial set without changing ranking.
- A semantic score is the cosine similarity recovered from sqlite-vec's L2
  distance over unit vectors (`1 - d²/2`), so it is monotonic in similarity
  and nonzero for near matches (#328).
- The `#tag` shorthand gives every matching Vault a turn before any Vault gets
  a second, so a Vault with a match is dropped only when `limit` is below the
  number of matching Vaults. It runs as a tag match whatever mode was
  requested, and its response reports `mode: "tag"` rather than echoing that
  request (#354).
- Participants tell the truth about degraded states: a vector-needing search
  reports `not_searchable` for a vectorless generation whatever its freshness,
  and for a selected demoted layer the generation built without vectors
  (`embed_layers=false` in its snapshot metadata). A named layer is judged
  absent only when no selected participant is unavailable; otherwise the search
  degrades to the partial envelope (#328).
- Search reads snapshots with `NoteProperties::Omit`, so it never parses the
  frontmatter it does not return.
- Participant metadata, note projections, and KNN/FTS hits for one search
  response come from one pinned SQLite generation.
- A semantic query is embedded before that generation is pinned, never while
  holding it. Inference is serialized behind the embedder's own lock, so a
  reader slot held across it is a slot no other request can use. The order
  costs an embedding on a query whose Vaults turn out not to participate, and
  reports an unhealthy embedder ahead of a bad layer name or an unavailable
  single Vault, both of which need the pinned generation to detect.
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
`MatryoshkaEmbedder`, `StubEmbedder`, and contextual-document formatting. The
ONNX embedders are exported unconditionally; `NomicV2Embedder` and
`Qwen3Embedder` are exported only under the `eval` feature, so a default build
of the crate does not carry them.

**Consumed dependencies:** local model runtimes, tokenizers, and Hugging Face
model files; under the `eval` feature also `candle-core` and FastEmbed's
`qwen3` / `nomic-v2-moe` features.

**Consumers:** cache building, runtime Search, startup/model setup, auxiliary
evaluation binaries, and tests.

**Coordination paths:** `src/model_setup.rs`, cache schema/identity handling,
chunking, Docker model prefetch, `Cargo.toml`'s `eval` feature, and evaluation
documentation.

**Invariants:** local inference only (ADR-04); embedder identity must encode
behavior affecting stored vectors; the `Embedder` trait remains the deliberate
test seam rather than proliferating model abstractions (ADR-13); the production
ONNX embedders are unconditional, while `src/embed/candle_embedder.rs` and the
candle inference stack it needs stay behind the non-default `eval` feature and
must never become reachable from a default build.

**Validation:** `cargo test embed`; `just check-full` for the model-loading
tests; cache identity/rebuild tests for identity changes; `cargo clippy
--all-targets --all-features` so the `eval`-gated embedders still compile.

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

**Validation:** `cargo test rerank`, relevant eval runner tests, and `just
check-full` for the model-loading tests.

### Git synchronization

**Kind:** infrastructure/background capability.

**Owned paths:**

- `src/git/mod.rs`
- `src/git/commit_cooldown.rs`
- `src/git/config.rs`
- `src/git/managed_checkout.rs`
- `src/git/managed_sync.rs`
- `src/git/managed_task.rs`
- `src/git/message.rs`
- `src/git/note_history.rs`
- `src/git/sync.rs`

**Public contract:** `GitMode` (`off`/`local`, carried only by the legacy
first-boot import — the instance-wide runtime lane is gone, #185), `GitConfig`,
write-record/message types (`WriteRecord`, `WriteLedger`,
`build_commit_message`), commit outcomes and errors (including
`GitError::ManualRecovery` for repository operations that cannot be proven
Hatchdoor-owned), and the local repository operations `validate_repo`,
`validate_local_repo`, `init_local_repo`, `commit_local`,
`has_uncommitted_changes`, and `run_local_history_git_turn`. Since #249
`run_local_history_git_turn` takes the Vault's `&WriteLedger` and names its
commit from the batch waiting there, restoring that batch when the turn finds
nothing to commit. Only the last
three are on a live path; `validate_repo`, `init_local_repo`, and
`has_uncommitted_changes` lost their production callers with the settings
lifecycle and the boot-time legacy validation in #185 and are retained
deliberately by that ticket's explicit keep list. The crate-private
`parse_mode` and `non_empty_setting` helpers serve the startup parse of the
legacy settings for the demo posture check. `resolve_commit_identity` (issue #130)
resolves one Vault's own configured `VaultCommitIdentity`
(`vault_registry.rs`) if set, else the instance-wide
`HATCHDOOR_GIT_AUTHOR_NAME`/`HATCHDOOR_GIT_AUTHOR_EMAIL` defaults; the Vault
work executor's `dispatch_git_turn` calls it once per turn, before
planning any of the branches below, so every commit this boundary makes
for a Vault — managed-Git, existing-Git remote-sync, or existing-Git
Local-history — honors that Vault's own identity. `commit_local` commits without network
access and discovers an enclosing existing checkout while staging only the
configured Vault subtree. Remote fetch/integrate/push, unpushed accounting,
and interrupted-merge marker recovery are gone with the instance-wide task
(#185); every remote graph operation this boundary still performs lives in
`managed_sync.rs`, which owns its own conflict and containment rules.
`commit_local` refuses a checkout found mid-merge, mid-rebase, mid-cherry-pick
or mid-revert, or with a conflicted index, as `ManualRecovery` (#323): seeding
its commit index from HEAD would otherwise drop the conflict entries and
commit the conflict markers. `classify_local_history_error` reports that as
the non-retryable `existing_git_local_history_manual_recovery_required` and
leaves the operation for the operator to finish. Its client-visible messages
are fixed per code; the full `GitError`, which names host paths, goes only to
the operator log. This boundary has no wire
surface and no instance-wide lifecycle: `GET /api/git-status` was retired in
#183 along with the Settings console it fed, and the settings handler's
preflight → drain → replacement protocol went with the task itself in #185.
`init_local_repo` takes the vault's configured
cache-database and settings-file paths and derives `.gitignore` entries from
them (only when those paths live inside the vault), appending to an existing
`.gitignore` rather than skipping it.

`ManagedCheckoutLease`, `ManagedCheckoutRequest`, `ManagedHttpsCredentials`,
and `acquire_or_reuse` form the shared-core managed-HTTPS acquisition boundary.
It holds a per-Vault process ownership lease, clones only into an
application-owned temporary sibling, validates origin, branch, repository
shape, and canonical Vault containment, writes an application-owned receipt
that retains a once-resolved default branch, and only then installs
atomically, so an installed checkout never lacks its receipt (#322).
Reuse accepts only a receipt-backed matching checkout; unknown, damaged,
mismatched, credential-bearing, or out-of-containment destinations remain
untouched and are rejected. An interrupted acquisition is not unknown: a failed
clone removes its own temporary, and while no `repository` is installed the
next acquisition, under the lease, deletes only the leftover temporaries whose
names this module generates (`repository.acquiring-<id>`, the receipt's
`.acquiring-<id>`; symlinks as links) and clones again (#322). This boundary
neither fetches nor resets, checks out, polls, or pushes.
`NoteHistory` (`note_history.rs`, #300, ADR-29) dates each note in a
Git-backed Vault by the author date of the commit that first added it, following
renames forward across the whole repository, so a note moved into a Vault
subdirectory keeps its date; a delete-then-recreate or a copy is new, and a
shallow clone's graft commit yields `FirstAdd::Unknown` rather than a date. It
walks once per `HEAD` on a background thread, keeps the result in memory, and
extends it by only the new commits when `HEAD` descends from the cached one
and the cached walk was not shallow. The tree at `HEAD` decides which notes
exist, so a merge that keeps a note one branch deleted keeps its date. The
control block starts a walk when an active Git-backed Vault's runtime is
built, and `VaultControlBlock::history_location` names the repository root
and the Vault's prefix inside it.
`NoteHistory::read` answers `Ready`, `Reading` (after waiting up to the
caller's bound) or `Unavailable`. It is read-only against the repository and
never touches the index, the working tree, or the network. The Vault read
projection (`vault_read.rs`'s `statistics_detail`) is its only consumer.

`reuse_existing_checkout` is `acquire_or_reuse` with the acquisition half
removed and `Ok(None)` in its place (#267): a commit turn must open no network
connection, and cloning is one, so it reuses the checkout a Vault already has
and reports "nothing here yet" rather than creating one.

`ManagedSyncConfig`, `ManagedSyncMode`, `ManagedSyncOutcome`,
`ManagedSyncError`, and `synchronize_managed_checkout` form the next shared-core
managed-checkout graph boundary. A caller that holds the checkout lease and
serializes Vault writes supplies the already validated repository and contained
Vault root, plus that Vault's `&WriteLedger`: a Two-way commit takes the
pending batch at the moment it is certain to commit and builds its message
from it (#249), restoring the batch if the commit itself fails, while
Pull-only never commits and leaves the ledger alone. Pull-only refuses and preserves any local work or local-only
history, then only fast-forwards a clean checkout. Two-way commits Vault-subtree
work before every tree-changing graph operation, refuses unrelated repository
work, fast-forwards remote-only advancement, creates a merge commit for clean
divergence, aborts every conflict back to the pre-merge local commit, and
never pushes after conflict. The abort runs to completion whatever else the
merge touched, inside the Vault subtree or outside it, leaving no MERGE_HEAD,
no conflict entries, and no conflict markers; it force-restores only the paths
the merge wrote, so a note an external editor saved mid-turn survives for the
next turn to commit, and falls back to a whole-tree hard reset only when that
targeted restore fails (#323). It otherwise uses safe checkout transitions and
rejects outside-Vault dirt rather than overwriting it. A non-fast-forward push
retries only through one bounded fetch-integrate-push graph replay before
returning a redacted push-race error. A push the remote accepts but refuses to
apply (a protected branch, a refusing hook) is read ref by ref through
libgit2's `push_update_reference` callback and fails the turn as
`ManagedSyncError::PushRejected` (`managed_git_push_rejected`, not retryable)
with the remote's one-line reason, instead of passing as synchronized. `commit_managed_checkout` is the Two-way commit without the graph
(#267): the same `prepare_two_way_worktree` step, and then it stops, with no
fetch, merge or push. It validates through `open_commit_repository`, which
proves the repository shape and Vault containment, and refuses a checkout
that is mid-merge (or any other unfinished Git operation) or whose index
holds conflict entries as `ManagedSyncError::OperationInProgress`
(`managed_git_operation_in_progress`, conflicted paths as detail), leaving it
untouched: committing it would publish conflict markers and drop the merge's
second parent, and this boundary cannot tell an interrupted Hatchdoor merge
from an operator's deliberate one (#323);
`open_validated_repository` is that plus the checked-out branch and the
uniquely selected remote, which only an operation that talks to that remote
needs. Pull-only is refused outright, because such a Vault refuses writes and
must leave a folder its operator dirtied alone.

The uniquely selected managed remote and its push URL must remain the
configured credential-free HTTPS repository identity; unrelated remotes in an
operator-owned `ExistingGit` checkout are outside this boundary and untouched.
Public HTTPS makes no credential callback; supplied credentials are callback
input only and remain redacted. Every fetch, push and clone first calls the
crate-private `bound_network_waits` (`git/mod.rs`), which sets libgit2's
process-wide socket timeouts once: 15 s to connect, 120 s without a byte
moving. A remote that stalls mid-transfer therefore fails the turn as the
retryable `managed_git_remote_unreachable` and releases the Vault's mutation
lock and the one work lane, rather than holding both until restart (#322). This boundary does not acquire, delete,
schedule, poll, persist status, or repair checkouts.

`WriteRecord`, `WriteLedger`, and `build_commit_message` are what makes a
commit say what happened. One Git turn coalesces every Vault write since the
last one, so the record of each write waits in the Vault's ledger in between:
the mutation core appends one `WriteRecord` per successful write, and the two
functions that actually commit — `commit_vault_drift` (Two-way) and
`commit_local` through `run_local_history_git_turn` (Local history) — take the
whole batch and name the commit from it. Title: the first three operations and
the unique file count. Body: one `- ` line per caller-supplied summary. A
commit whose batch is empty, which is what drift from outside Hatchdoor
produces, keeps the generic `hatchdoor: vault update`. The ledger is bounded
(`WriteLedger::CAPACITY`) because a `Local` Vault has no Git turn to take it.

`ManagedGitTurnConfig`, `ManagedGitOutcome`, `run_managed_git_turn`,
`ManagedGitScheduler`, `GitPollingClock`, `spawn_scheduler_tick`,
`DEFAULT_POLL_INTERVAL`, and `DEFAULT_TICK_INTERVAL` form the per-Vault managed-Git scheduling boundary —
the "later consumer" the two paragraphs above anticipated. `run_managed_git_turn`
is the concrete `acquire_or_reuse`-then-`synchronize_managed_checkout` operation
`VaultWorkKind::Git` executes; it classifies every `ManagedCheckoutError`/
`ManagedSyncError` into a redacted `VaultWorkError{code, message, retryable}`,
`ManagedCheckoutError::AtomicInstallFailed` carries the reason the install
failed rather than discarding it, since the first report of that variant
reached an operator as a bare "install failed" over a filesystem that could
not do `RENAME_NOREPLACE` (#345); the install falls back to check-then-rename
there, as the Vault write layer does (ADR-26). The classification keeps
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
newly tracked Vault one poll interval after its last remembered
interval-arming turn — immediately when nothing is remembered, or when that
deadline has already passed. For an already-tracked Vault it updates the
stored interval in place and, when the new interval brings that same
deadline forward, re-arms the pending attempt to it: an operator shortens an
interval *for* the next check, so a Vault sitting on a long armed deadline
must not serve the whole of it out before the edit is visible. Only forward
— a lengthened interval leaves the nearer deadline where it is — and never
over a live backoff: a transient failure's backoff is not on the poll
interval at all, so re-deriving it from the last interval-arming turn would
discard the throttle on a remote that is currently failing. A held checkout
lease is untouched either way; it is not a condition on the re-arm. The
interval is clamped to `MAX_POLL_INTERVAL` on the way in, so no deadline this
module arms can overflow — `poll_interval_secs` has a registry minimum but no
maximum, and `record_outcome` arms under the `entries` lock, where a panic
would poison the scheduler for every Vault in the process.
`sync_now`/`retry_now` take the same `poll_interval`
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

`with_state_store` is the production constructor: it gives the scheduler a
`vault_runtime_state::VaultRuntimeStateStore`, which is what makes a poll
interval survive a restart. Each Vault's record is read once at `activate`
time and refreshed by `record_outcome` after every interval-arming outcome —
a success or a non-retryable failure, never a transient failure, whose
backoff stays process-local because a restart cannot verify the condition it
was throttling and should retry at once. What is remembered is the *last
turn*, never a computed deadline, so an interval edited while Hatchdoor is
down takes effect on the next start rather than serving out the interval that
was in force when the record was written. The wall clock is consulted only to
derive that first deadline; the countdown itself is held as an `Instant`, so a
host clock moving mid-process cannot disturb it, and a stored stamp in the
future is treated as unknown rather than delaying a Vault by the skew. A
store failure is logged and dropped — the turn already happened, and
forgetting it costs one extra turn after the next restart.
`forget_persisted_state` prunes a Vault that has left the collection, called
by the collection lifecycle for a disconnect (never for a disable, which
keeps its schedule). `polling_clock(vault_id)` returns the
`GitPollingClock { last_checked_at, next_attempt_at }` a status read
renders, and `remembered_turn(vault_id)` returns the whole remembered record;
both come from memory, so listing the collection never touches the file.
The collection lifecycle uses `remembered_turn` at activation to republish
the Git status the previous process reached — guarded on the `Pending` a
fresh process publishes, so an in-process edit keeps the live status
`reconcile` preserved through `prior_git`. Without it a restart would report
nothing wrong about a failing Vault until its next scheduled turn, which is a
whole poll interval now that a restart no longer forces one.
`without_durable_state` is the store-less constructor — every Vault due
immediately, every schedule lost with the process — named so that a call site
cannot opt out of remembering without saying so; it exists for tests and for
composition roots with nowhere durable to write.

`DEFAULT_TICK_INTERVAL` is a *sampling* interval, not a schedule: a deadline
can only be observed at its resolution, so a compile-time assertion holds it
to at most half of `BACKOFF_BASE`. At one sample per backoff base every
"30 second" retry would land at 60, collapsing `BACKOFF_BASE` into
`BACKOFF_MAX`; the assertion turns lowering either constant without the other
into a build failure rather than a silently degraded retry.

`run_local_history_git_turn` is an `ExistingGit` + `VaultGitMode::LocalHistory`
Vault's counterpart to `run_managed_git_turn`: given the Vault's already-resolved
path and commit identity, it builds its own placeholder `GitMode::Local`
`GitConfig` and calls `validate_local_repo` then `commit_local`, committing
only the contained Vault subtree of whatever enclosing checkout the Vault sits
in and never contacting a remote. It classifies every `GitError` into a
redacted `VaultWorkError`, mirroring the legacy single-Vault task's transient
split (`Remote`/`Other` retry; validation, conflict, and dirty-tree do not).
Its client-facing message is fixed per code rather than `GitError`'s text,
which carries absolute host paths; the full error goes to the log (#323).
Every Git error message that reaches a Vault's status is path-free, including
`existing_git_branch_unresolved` and `ManagedCheckoutError::AtomicInstallFailed`
reasons.
Unlike managed-Git Vaults, an `ExistingGit` Local-history Vault is never
registered with `ManagedGitScheduler`. Before #267 that left it with exactly
one turn per process, the `Pending`-triggered one at activation, so every
note written afterwards sat uncommitted until a restart. It now receives a
`VaultWorkKind::Commit` turn from the watcher on every change (and that
activation turn is a `Commit` too), which is the whole of its Git behaviour;
it still has no remote and still never polls one.

`run_managed_git_commit_turn` and `run_existing_git_commit_turn` are the
commit-only counterparts of the two remote-sync turns above, and what
`VaultWorkKind::Commit` executes (#267). Both refuse any mode but `TwoWay`
with the non-retryable `vault_commit_mode_does_not_commit`; Local history's
commit turn is `run_local_history_git_turn`, unchanged. The managed one takes
the same `&ManagedCheckoutLease` a sync turn takes but reaches the checkout
through `reuse_existing_checkout`, so a Vault whose first clone has not landed
reports `UpToDate` instead of cloning; it carries no credentials, because
there is nothing to authenticate against. The existing-checkout one takes
neither a `repository_url` nor a `branch` and leaves both blank in its
`ManagedSyncConfig`, because `commit_managed_checkout` reads neither. That is
what lets an `ExistingGit` Vault with no configured branch commit without
resolving one. Neither reaches `ManagedGitScheduler`: a commit is not a check
of the remote and must not move the schedule that governs one.

`publish_recovery_branch` (`managed_sync.rs`, ADR-30) pushes a Two-way
checkout's local head to `recovery_branch_name(branch, vault_id)`,
`hatchdoor-recovery/<branch>/<vault id>`, after committing pending drift the
way a sync does. It goes through `push_refspec`, the one non-forcing push the
configured-branch `push` also uses, and renames its failures:
a non-fast-forward is `RecoveryDiverged` (someone added to the branch) and a
refused ref is `RecoveryRejected` with the sanitized remote reason. It never
names the configured branch as a push destination and never deletes a ref.
`run_managed_recovery_turn` (reusing the existing checkout, with credentials)
and `run_existing_git_recovery_turn` (resolving an unconfigured branch like the
remote turn) wrap it, returning a `RecoveryResult` whose `RecoveryFailure`
carries the branch when it was known. `CONFLICT_CODE` is the one Git status
code a publish is admitted from.

`CommitCooldown`, `DEFAULT_COMMIT_COOLDOWN` (5 minutes),
`COMMIT_COOLDOWN_TICK_INTERVAL`, and `spawn_commit_cooldown_tick`
(`commit_cooldown.rs`) are what stops a standing commit failure becoming one
failed turn per save. Every way a commit can fail is non-retryable and needs a
human, so a failed commit turn `arm`s the Vault's window and the
watcher-forwarding path stops being `admit`ted for its duration; a successful
commit or a manual one `clear`s it. Changes arriving while suppressed are not
dropped. They coalesce into one deferred request the tick issues once the
window elapses, which is what lets a Vault resume committing on its own after
the operator fixes the cause. State is process-local and disposable: nothing
about a suppression window is worth surviving a restart.

`source_commits` and `source_syncs_remote` (`mod.rs`) answer "does this Vault
make local commits" and "does it have a remote to sync with" from a
`VaultSource` alone. Deliberately separate from
`VaultSource::managed_git_poll_interval`, whose meaning ("does this Vault poll
a remote") is unchanged: the watcher uses the first to decide whether a change
is worth a commit turn, `vault_management` uses it to decide which manual
operation to admit, and `collection_capabilities` publishes both as the
`commit`/`sync` Vault capabilities the settings Git console labels its action
from.

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
and `AppState`.
`ManagedGitScheduler`/`run_managed_git_turn` are consumed by the Vault work
executor (`src/vault_executor.rs::dispatch_git_turn`) and by runtime
composition (`reconcile_and_reconstruct`, which activates/deactivates a
managed-Git Vault's schedule alongside its coordinator admission). `dispatch_git_turn_with`
obtains the Vault's checkout lease via
`ManagedGitScheduler::take_or_acquire_checkout_lease` before `spawn_blocking`,
passes it into the injected turn (`run_managed_git_turn` in production), and
hands it back with `keep_checkout_lease` once the turn completes, so the
lease survives across turns without being borrowed across the
`spawn_blocking` boundary — and by
`src/server.rs`, which
owns the one global dispatch loop driving `VaultWorkWorker::next_turn` — the
worker/scheduler-tick construction and dispatch this module map previously
noted as missing. `run_local_history_git_turn` is likewise consumed by `plan_git_turn`'s
`ExistingGit` + `VaultGitMode::LocalHistory` arm, off the async runtime via
`spawn_blocking`, publishing through the same
`publish_managed_git_turn_outcome` a managed-Git turn uses; that Vault is
never registered with `ManagedGitScheduler`, so this arm is its whole
Git-turn responsibility. `run_existing_git_remote_turn` is consumed the same
way by `plan_git_turn`'s `ExistingGit` + `VaultGitMode::PullOnly`/`TwoWay` arm
(issue #96's reopening defect 1): no checkout lease, but the same
`VaultControlBlock::acquire_mutation` hold across `spawn_blocking` that the
`ManagedGit` arm also takes (defect 2), and publication through the same
`publish_managed_git_turn_outcome`.
`VaultWorkKind::Commit` is consumed by the Vault work executor's
`dispatch_commit_turn`/`plan_commit_turn`, which resolve the Vault's source
and mode to one of the three commit operations, run it through the same
lease/mutation-lock/`spawn_blocking` shell (`run_planned_turn`) a Git turn
uses, and publish through `finish_commit_turn`, which unlike
`finish_git_turn` feeds no scheduler and queues no Index turn, because the
watcher change that asked for the commit already asked for the reindex. It is
requested by `src/server.rs`'s watcher forwarding (commit first, index second,
so a commit never waits out a multi-minute rebuild), by
`reconcile_and_reconstruct`'s activation gate for a Git-capable source the
scheduler does not track, by `vault_management`'s manual sync/retry on a Vault
with no remote, and by `spawn_commit_cooldown_tick`.
`VaultWorkKind::Index` is consumed by the Vault work executor's
`dispatch_vault_index_turn`, which publishes only that Vault's disposable
snapshot and reports its per-Vault search outcome; `Repair` remains an explicit
non-retryable "not yet implemented" `VaultWorkError` so a Vault's shared FIFO
position is not blocked ahead of its Git turn.

**Coordination paths:** `src/app_state.rs`, `src/server.rs`,
`src/vault_executor.rs`, `src/vault_runtime.rs`, `src/vault_registry.rs` (crate-private
`https_credentials` accessor), `src/vault_runtime_state.rs` (the durable
per-Vault Git-turn record `ManagedGitScheduler` reads at activation and
writes after each interval-arming turn), `src/vault_management.rs` (which
renders `polling_clock` as a Vault summary's `last_checked_at`/
`next_attempt_at`), `src/handlers/settings.rs`, HTTP/MCP write
adapters, configuration, frontend settings UI, and vault watcher Git
exclusions.

**Invariants:** optional and debounced; a commit turn never opens a network
connection, whatever the Vault's mode; writes do not block on sync, except
while a managed-Git or `ExistingGit` remote-sync turn, or a Two-way commit
turn, is in flight for that Vault, see below; task replacement drains
before another task can start;
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
`cargo test managed_task`, `cargo test vault_runtime_state`, and `cargo test
vault_runtime`.

### HTTP adapters

**Kind:** adapter.

**Owned paths:**

- `src/handlers/mod.rs`
- `src/handlers/api.rs`
- `src/handlers/assets.rs`
- `src/handlers/diagnostics.rs`
- `src/handlers/docs.rs`
- `src/handlers/downloads.rs`
- `src/handlers/folders.rs`
- `src/handlers/settings.rs`
- `src/handlers/spa.rs`
- `src/handlers/transfer.rs`
- `src/handlers/vault_collection_reads.rs`
- `src/handlers/vault_content.rs`
- `src/handlers/vault_write.rs`
- `src/handlers/vaults.rs`
- `src/handlers/whats_new.rs`

**Public contract:** handler functions intentionally re-exported by
`src/handlers/mod.rs`; their route, authentication, status, and serialized HTTP
behavior — and nothing else. The one non-handler seam this module used to export
for the MCP `get_attachment` tool (#176) went with #188: attachment resolution,
the servable-extension allow-list, the content-type table, and the size bound
now belong to the read core (`src/vault_read/assets.rs`), which applies
`VaultReadCore`'s browse-surface gating to them, so both surfaces refuse the
same paths. What `assets.rs` keeps is this route's own wire shaping.
`transfer.rs` redeems transfer links (ADR-27, ADR-32) on
`/api/v1/vaults/{vault_id}/transfers/{*path}`: `GET` downloads through
`vault_scoped_asset_handler` with the `McpAssetRead` ceiling after spending the
transport's tool budget, and `POST` uploads through the mutation core's
`import_attachment` with the upload route's own multipart form, read by the
shared `vault_write::read_upload_form`. A `.md` target
(`is_note_upload_target`) is redeemed with `redeem_note_upload` and written
through the core's `upload_note` instead, answering the note-write shape
(`vault_write::note_write_response`); the part's declared content type is
ignored. Both re-read the live configuration per
request: MCP disabled refuses every link (`mcp_disabled`), write mode off every
upload (`mcp_write_disabled`), and a link that does not verify is `403` with
`transfer_link_invalid`, `transfer_link_expired`, or `transfer_link_spent`. The
routes sit outside the bearer and web-token guards, which they leave unchanged.
`folders.rs` serves `GET /api/v1/folders?path=` over the folder listing on the
blocking pool, behind the web token when one is configured and refused with
`403 demo_read_only` in demo mode; its refusals are `400 folder_outside_root`,
`404 folder_not_found` and `422 folder_unreadable` (ADR-41).
`whats_new.rs` serves `GET /api/v1/whats-new` (ADR-42) behind the web token
when one is configured and refused with `403 demo_read_only` in demo mode,
because it names the running version (ADR-38 decision 6). It answers
`no-store` JSON: `version` (`config::version_string`), `previous_version`,
`fresh_install` (true only while the instance runs the version it was freshly
installed on) and `releases`, the sections of the bundled What's new page
after `previous_version` up to the running version, newest first, each with
`version`, `date` and `highlights` (`text`, `action_needed`, `link` with
`label`, `page` and `heading`). It also owns the page's format,
`parse_releases`: `## v<version> - <YYYY-MM-DD>` sections, newest first, of 3
to 6 one-line items, action-needed items (`**Action needed:**`) first, each
ending in at most one link into the manual. `the_bundled_page_parses` keeps a
malformed page from shipping; `scripts/release-common.mjs` drafts and checks
the same shape at release time. `PAGE` (`whats-new`, re-exported as
`WHATS_NEW_PAGE`) is the page's name, which the MCP instructions cite.
`settings.rs` owns the additive `/api/settings` document: effective
value/provenance/lock/class/kind metadata and partial PATCH saves returning the
full refreshed document, plus the read-only `last_agent` (#426) and
`update_check` (#425, `update_check::status` read from the instance state file
beside the registry) fields. MCP enablement and its bearer token validate together
against one prospective snapshot, so an invalid combination saves nothing and
reports field errors. Its candidate-token and capability-safe secret-reveal
endpoints are `no-store`; the ordinary settings document never exposes secret
values. A save whose consequence needs consent (a reindex, initializing local
history, or downgrading away from remote versioning) is refused with `409` and
a machine-readable `confirmation_required` consequence — the server is the
authority, and sends no prose; the page owns the words and resends with a
`confirm` list. `reindex` is the only consequence: #183 retired `git_init` and
`git_downgrade` with the instance-wide Versioning console that explained them,
and #185 removed the repository work they described, so a `HATCHDOOR_GIT_*`
save now only persists a value. Saves persist before rebuilding. A confirmed indexing-setting save requests one Index turn per
active Vault through the shared work coordinator
(`app_state::request_collection_reindex`), never the legacy instance-wide
rebuild: each Vault reports its own `indexing` condition and keeps serving
reads from its previous snapshot until its new one is published, and a
disabled Vault has no active runtime so it is not queued.
A save that flips
`HATCHDOOR_MCP_WRITE_ENABLED` — the only setting that adds or removes tools
from the advertised catalogue — broadcasts
`AppState::mcp_tools_changed` so subscribed MCP sessions re-list; a
layer-marker change does not, because no tool schema is derived from it.
Every save takes one path: the legacy versioning-task lifecycle branch (stop
the task, preflight the repository, respawn) went with the task itself in
#185. `HATCHDOOR_GIT_SYNC_ENABLED`, `_REMOTE`, `_BRANCH`, `_HTTPS_USERNAME`,
`_HTTPS_TOKEN`, `_DEBOUNCE_SECONDS`, and `HATCHDOOR_EXCLUDE` remain in the
schema although #427 removed the import that consumed them; the registry's
startup step purges the Git-lane ones from stored settings and refuses an
install without a registry that still stores any of them. No per-operation
code reads them; two `server.rs` startup checks
still parse them — `check_demo_mode_posture` and `HATCHDOOR_EXCLUDE`'s pattern
validation — and both only refuse a start.
`HATCHDOOR_GIT_AUTHOR_NAME`/`_EMAIL` remain
the commit-identity fallback the collection lane's Git turns read per turn, so
a change to them still reaches the next turn without a restart.

`vaults.rs` is the HTTP adapter over **Vault collection management**: it owns
the `/api/v1/vaults` routes — discovery, collection management (create/edit/
enable/disable/disconnect), manual Git sync/retry, one-Vault Index refresh, and
the collection-wide SSE event stream — and nothing else. Since #187 each route parses its own path, query,
and body, calls `vault_management::VaultCollectionManagement` once, and maps
the typed response or the structured `VaultOperationError` onto a status code
and a JSON body. The registry commit, the runtime reconciliation, the
authenticated and demo projections, the
credential-replacement Git retry, and the recovery action all live in that
core, shared with the MCP management tools, which no longer proxy these
handlers.

This is the first `/api/v1` surface, and it carries no instance-wide readiness
gate: discovery and creating the first Vault stay reachable at zero enabled
Vaults, and discovery reports an explicit `recovery` object rather than erroring
when the persisted registry itself needs operator recovery. Every response uses
the shared `VaultApiError{code, message, vault_id?, retryable}` shape — the
adapter spelling of the core's `VaultOperationError` — and reuses
`vault_registry::VaultSource`/`VaultGitMode` directly on the wire rather than
duplicating them.

The status mapping is the whole of this adapter's error contract, asserted
directly by `every_management_error_code_keeps_its_historical_status`:
`invalid_vault_id`/`invalid_vault_definition` are
`400`; `vault_not_found` is `404`; the registry-state conflicts
(`duplicate_vault_name`, `vault_path_overlap`, the two identity-change
refusals, `registry_revision_conflict`, `vault_disabled`,
`capability_unavailable`) are `409`; `vault_registry_recovery_required` and
`vault_unavailable` are `503`; and
`internal_error`/`registry_revision_exhausted` are `500`. On top of that the
adapter adds the two statuses the core does not model: `201` for a creation and
`202` for admitted background work. `internal_error` is logged and sanitized by
the core, so nothing here re-reports it.

Discovery and the event stream are pure reads and stay reachable in demo mode
(#109: demo mode publishes every enabled Vault in the instance as a public
read-only collection, unlike `settings.rs`'s operator-controls posture, which
remains absent), where the core answers with its public projection. Collection
management, manual Git sync/retry, and one-Vault Index refresh are
Vault-control operations, so `src/server.rs` wraps each of their routes —
individually, since some share a path with a read (`POST /api/v1/vaults`
alongside `GET`) — in `reject_demo_mutation`, which calls this file's
`demo_read_only_response` to refuse with a shared `403 demo_read_only`
`VaultApiError` before any registry mutation runs, rather than being absent.
That refusal body and the SSE stream's `Event` framing are the two things that
stay here because they are transport with no MCP counterpart; the stream's
underlying revision channel is obtained from the core's `subscribe_revisions`
rather than from the runtime directly.

`vault_content.rs` owns exact Vault-scoped content reads and their contained
resources, mounted in the same `/api/v1/vaults/{vault_id}/...` router group as
`vaults.rs` and sharing its demo-mode/auth posture, `VaultApiError` (including
its `new`/`respond` constructors, widened to `pub(crate)`), and
rejection-mapping helpers (`parse_vault_id`, `json_rejection_response`,
`query_rejection_response`, `internal_error_response`, widened to
`pub(crate)` for this reuse): `GET .../notes/{slug}`, `GET
.../notes/{slug}/links`, `GET .../notes/{slug}/download`, `GET
.../notes/{slug}/saved-queries` (#275: the Note's saved queries evaluated now,
in the projection envelope; its definitions come from the authoritative
Markdown and its rows from the published snapshot, so its participant state
reports the rows' freshness), `GET .../resolve`,
`POST .../resolve-batch` (whose request additionally takes optional
`asset_targets` and `note_path`, answered by an `asset_results` array of
`{target, path}`, `path` null when nothing matched, and optional
`note_link_targets`, Markdown note-link destinations as written, answered by
a `note_link_results` array shaped like `results` and resolved by path from
`note_path`'s folder (ADR-28) — both additive, so a client resolving
wikilinks only sees exactly what it saw before, and the batch cap counts all
three target lists), `GET .../assets/{*path}` (serving both embedded
assets and imported attachments, which share one containment rule; mounted
outside `vaults_v1`'s web-token-only gate, under
`require_web_or_live_mcp_read_token`, so `get_attachment`'s advertised
`download_url` is fetchable with the MCP credential), and `GET
.../stats/detail` (#137's rich per-Vault statistics report). Every route but
the last always inspects the requested Vault's authoritative Markdown
directory through `VaultReadCore`, never the disposable cache; `stats/detail`
is the sole exception, reading the same published snapshot the collection
`{scope}/stats` route reads (word/mtime/size data `VaultReadCore`'s
authoritative-index path does not carry), so it can briefly lag a write the
way collection reads do, unlike every other route here. Exact reads run all blocking
filesystem/index work off the async runtime via the read core's `VaultReads`
handle (one trip per request, not one per batch entry or per path-resolution
step), and are gated per-request by that Vault's own
`vault_not_found`/`vault_disabled`/`vault_unavailable` status rather than any
single-configured-Vault readiness gate. Since #188 asset resolution is the
core's (`VaultReadCore::contained_asset`); what is left in `assets.rs` is this
route's own wire shaping — the success response's headers and the HTTP status
each `AssetPathError` carries — and `downloads.rs` still owns export and
download-response building (`build_note_export`, `download_response`);
`build_note_export` bundles only the assets its caller admits, and the
download route admits by the read core's `AssetSurface`.

`vault_collection_reads.rs` owns one-or-all collection reads and search:
`GET /api/v1/vaults/{scope}/tree`, `.../recent`, `.../stats`, `.../graph`, and
`.../search`, mounted in the same router group and sharing the same
demo-mode/auth posture, `VaultApiError` shape, and `query_rejection_response`/
`vault_read_error_response` (the latter widened to `pub(crate)` in
`vault_content.rs` and extended with `invalid_search_query`/
`invalid_layer_selection` (`400`) and `search_unavailable` (`503`) arms for
reuse here) rather than duplicating them. `{scope}` reuses the path segment
name `vault_id` for router-tree consistency with every sibling route in this
group, parsed by the core's `VaultScope::parse` into either a Vault ID or
`VaultScope::All`; anything else is the structured `invalid_scope` error
(`400`). Since #188 that parser, the `layers` grammar, and the limit/per-note
clamps live in the core so the MCP tools apply exactly the same ones. This file is a thin adapter with no collection-read domain logic of
its own: `tree`/`stats`/`graph` return `vault_read.rs`'s existing
`VaultReadCore::{trees, statistics, graphs}` projections unchanged (grouped
per Vault); `recent` returns `recently_modified` (flattened across Vaults);
`search` returns `search::vault_scoped::VaultSearchCore::search`'s projection
(flattened, one global ranking). `search`'s `layers` query parameter is a
comma-separated token list parsed by `BrowseSurface::layer_selection`
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
the retired `/api/attachment` route), and `GET .../write-capabilities`, whose
payload is `{vault_id, enabled, atomic_compare_and_swap, warnings}`. The
`atomic_compare_and_swap` field and one matching `warnings` sentence say that
this Vault's filesystem saves without the atomic swap (ADR-26, #345); that
sentence is this surface's own wording, like the web-auth warning beside it. Since
#186 every one of those eight routes has ADR-19's shape: it parses its path
and body, calls `VaultMutationCore` once, and maps the typed outcome or the
structured `VaultOperationError` onto a status code — including this surface's
own sanitizing of a `write_failed` message into the generic internal error,
and its own operator-facing `warnings` on the capabilities route, which fold
in the instance's web-auth posture the core knows nothing about. No route here
holds gating, locking, index-build, entry-lookup, marker or noise refusal,
archive-prefix, or write-error-translation logic; each of those steps has
exactly one implementation, in `vault_mutation.rs`, shared with the MCP write
tools. Archive prefix and attachment size limit stay instance-wide settings
(issue #62), read via `AppState::runtime_snapshot`/`runtime_archive_prefix`/
`runtime_mcp_config`. A mutation response omits `git_sync_warning`: the
managed-Git scheduler has no debounced-on-write hook, unlike the retired
instance-wide sync task, so there is nothing per-write to report that Vault
discovery does not already expose. `vaults.rs` owns the
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

The attachment upload is the one route that still reads its own body:
`vault_write.rs` binds one live configuration snapshot *before* consuming any
multipart field and reads each field incrementally against its fail-closed
byte limit, so lowering the limit takes effect on the next request rather than
after the bytes are already buffered; an invalid pinned upload limit never
falls back to a larger default. That streaming discipline has to stay where
the bytes arrive, so the core exposes the import primitive over already-decoded
bytes rather than over a stream. A `WriteError` carrying recovery guidance
reaches the client as `write_recovery_required` rather than collapsing into
the sanitized generic internal error. `vault_content.rs` bounds Vault asset
and generated note-download responses so these convenience endpoints are not
unbounded transfer buffers; an over-limit asset or export receives the shared
`VaultApiError` shape and `413 Payload Too Large`.

`spa.rs` serves the built app: `spa_index_handler` answers the app's own routes
with `200`, and `spa_not_found_handler` is the static directory's fallback
(#302), so an address no route or built file matches still loads the app with a
`404` status and the app renders its not-found state. Paths under
`SPA_RESERVED_PREFIXES` (`/api/`, `/vault-assets/`, `/docs/`, `/llms.txt`, and
anything starting `/health`) keep a bare `404`. That list mirrors the service
worker's `navigateFallbackDenylist` in `frontend/vite.config.ts`, and a test in
`spa.rs` fails if the two drift. `docs.rs` (#422, ADR-38) exports
`docs_router`, the public manual routes `GET /docs/<page>.md`,
`/docs/index.md`, `/docs/deploy.md` (the agent deploy page), `/docs/search?q=`
(JSON `results` of `name`, `title`, `excerpt`; `q` cut to 200 characters) and
`/llms.txt`, mounted outside every auth layer and in demo mode. Wikilinks
become links relative to the address asked for; an unknown page is a
plain-text `404`. A private page (`private: true` in its frontmatter) answers
only a caller holding the web token, `401` otherwise, and is left out of the
index, search and its links' destinations for anyone else and out of
`llms.txt` always; with no web token configured it is never served there.

**Consumed dependencies:** `AppState`, HTTP wire types, vault reads,
`vault/write`, Search, cache queries, Git status, auth (`docs.rs` uses
`auth::request_is_authorized` to tell whether a caller holds the web token),
the Bundled manual (`docs.rs` only), and — for `vaults.rs`
only — the Vault collection registry's mutation/load operations,
`VaultCollectionRuntime::{snapshot, reconcile_and_reconstruct,
subscribe_revisions}`, `VaultWorkCoordinator`, and
`ManagedGitScheduler::{sync_now, retry_now}` and `VaultWorkCoordinator::request`
via `AppState::{vault_work, managed_git}`. `vault_content.rs` is the first HTTP consumer of
Vault-qualified read projections (`vault_read.rs`'s `VaultReadCore`, including
its `vault_directory` accessor); `vault_collection_reads.rs` is the first HTTP
consumer of that core's collection-read projections and of
`search::vault_scoped::VaultSearchCore`. `vault_write.rs` consumes the
Vault-qualified mutation core (`vault_mutation.rs`) and nothing else on the
write side: every `vault/write` primitive, `VaultControlBlock` lock, and
`VaultReadCore` gate it used to reach for directly now reaches it through that
core.

**Consumers:** route construction in `src/server.rs`; the MCP boundary's
`get_attachment` read tool, through the asset seam named in the public contract
above.

**Coordination paths:** `src/server.rs`, `src/api_types.rs`, frontend clients,
and whichever domain a handler adapts.

**Invariants:** handlers stay thin. Write handlers never touch the vault
filesystem directly (ADR-03). Static and vault asset behavior must retain auth
and path containment. `docs.rs` reads the Bundled manual and nothing else: no
Vault, registry, settings or token value reaches its responses
(`public_manual_routes_answer_without_a_token_and_reveal_nothing_of_the_instance`). `vaults.rs` never returns HTTPS credentials, only
`credential_configured` (ADR-01/registry invariant); disconnect deletes no
files, checkouts, Git history, or credentials outside the registry record.

**Validation:** `cargo test handlers`, router tests, and affected domain tests.

### MCP adapter

**Status:** Implemented per [ADR-17](../../adr/README.md) (#168): rmcp 3.x
(pinned `rmcp = "=3.1.4"`) owns the `/mcp` transport and the advertised
revisions are exactly `2026-07-28` and `2025-11-25`; #170 adds honest
`tools.listChanged` on the modern surface. The remaining Wayfinder children
build on this seam: #171 (layered rate limits) and #172 (release evidence).
#177 adds `batch`: one generic tool that executes a caller-supplied ordered
list of note/attachment operations (`create_note` through `delete_attachment`,
plus every read tool except `list_vaults`) in a single call, dispatching each
item through the exact same `read`/`write` tool functions a standalone call
uses. Vault-management ops and unrecognized op names are rejected up front,
before any item executes; over the asymmetric per-batch caps in
`src/mcp/limits.rs` (`BATCH_MAX_READ_ITEMS` = 50, `BATCH_MAX_WRITE_ITEMS` =
20) the whole call is refused the same way. Execution is best-effort and in
order — one item's failure never stops the rest, and there is no rollback —
with `expected_content_hash` chaining between items in the same call that
share a `(vault_id, slug)`: `mcp/tools/batch.rs` tracks each note's resulting
hash as the batch runs and substitutes it for a later item's own
`expected_content_hash`, so a caller can create or edit a note earlier in the
batch and reference it again later without an intermediate read; a note not
otherwise touched in the batch still validates its `expected_content_hash`
normally. #321 makes the locking behind that chain ordered rather than lazy:
`lock_touched_vaults` pre-scans the write items, sorts the distinct Vault IDs,
and acquires every mutation lock in that one canonical order before the first
item runs, which is what stops two concurrent batches naming two Vaults in
opposite orders deadlocking each other permanently. It also resolves each
Vault's control block for the whole call; a reconcile mid-call — before the
lock is granted or after it is taken — re-resolves and continues only against
a live block sharing the exclusion the call holds (which a definition edit's
replacement does, since #321 — see ADR-25), and is otherwise refused with a
structured error rather than written unlocked. No Git-specific handling exists in the tool: it writes Markdown
files exactly as the standalone tools do, and the existing per-Vault Git
turn (`commit_vault_drift`, `src/git/managed_sync.rs`) already commits
whatever is dirty at that turn in one commit — a batch's writes therefore
land in one commit the same way any burst of individual write calls would,
without changing ADR-10's debounced background-sync semantics. Catalogue
grows 38 → 39, purely additive. #228 adds `refresh_vault`, the eighth Vault
management tool: a write-gated mapping onto the collection management core's
`refresh`, which admits one Vault's next Index turn and returns its
`VaultScheduleResponse` (`queued`, or `coalesced` when a turn for that Vault is
already pending). It exists so a client reading a collection read's `partial:
true` with a `stale` participant can act on it, which `sync_vault` and
`retry_vault` cannot: both are Git controls, and since #267 they refuse
`capability_unavailable` only on a Vault with no Git at all (a `Local`
source), admitting a commit turn on a Vault that has no remote but does keep
history. It is rejected inside
`batch` like every other management tool, and is deliberately *not* in
`is_collection_management_tool`: that exemption keeps discovery and Vault
control reachable while model setup is pending, and an Index turn cannot run
without a configured search model. Catalogue grows 39 → 40, purely additive.
#242 adds `rename_tag`, the sixteenth write tool: a mapping onto the mutation
core's `rename_tag`, dispatched under the Vault's mutation lock like every
other write tool, answering a plan or an applied rename in `RenameTagResult`.
It is listed in `WRITE_OPS` so the write gate and the catalogue drift guard
cover it, and in `batch.rs`'s `NOT_BATCHABLE_WRITE_OPS`, which refuses it as a
batch item before anything runs: its all-or-nothing promise cannot hold inside
a best-effort batch. Its refusals reach the caller as structured tool errors
with their own codes. Catalogue grows to 42 across all catalogues, purely
additive. #277 adds `evaluate_saved_query`, the fifteenth read tool and a
`READ_OPS` entry (so `batch` may carry it): a mapping onto
`VaultReadCore::saved_query` answering `EvaluateSavedQueryResult`, the shared
projection envelope around `SavedQueryEvaluation`. Its arguments have no
`scope` by design, so a caller-supplied one is an invalid-params refusal, and
every request that reaches no answer is a structured tool error rather than a
success with zero rows. `get_note` reports the Note's `saved_queries` through
`VaultQualifiedNote` itself, so the adapter adds nothing. Catalogue grows to
43, purely additive. #310 adds `create_upload_link`, the seventeenth write tool
(ADR-27): it asks the mutation core's `check_attachment_import` whether the
target would be refused before any bytes arrive, then mints an upload transfer
link through `AppState::transfer_links`, answering `UploadLinkResult`. It is in
`WRITE_OPS`, so write mode gates it and `batch` may carry it. The adapter now
records the origin each tool call arrived on in `McpConfig::request_origin`,
which transfer links fall back to when `HATCHDOOR_PUBLIC_URL` is unset. Since
#358 (ADR-34) that origin's scheme and host each come from the first element
of `Forwarded`, else `X-Forwarded-Proto`/`X-Forwarded-Host`, else `http` and
`Host`, skipping unusable values; the adapter reads these headers for nothing
else.
`McpConfig::public_url` parses that setting (`parse_public_url`, failing
closed on an invalid pin like the attachment limits) and `link_base` picks
between the two. `tools::transfer_link_signer` is the one place a tool gets its
signing key and base, and refuses with `invalid_params` when there is no base.
Catalogue grows to 44, purely additive. #303 (ADR-32) widens
`create_upload_link` to notes without a new tool: a `.md` target asks the
core's `check_note_upload` instead, and an optional `expected_content_hash`
argument, required with `overwrite` on a `.md` target and refused as invalid
params anywhere else, mints a replacing note link through
`mint_note_replace`. `UploadLinkResult` gains `upload_kind` (`note` or
`attachment`) and the echoed `expected_content_hash`, and its `usage` text
differs by kind. #258 adds `delete_tag`, the
eighteenth write tool, shaped exactly like `rename_tag`: in `WRITE_OPS`, in
`NOT_BATCHABLE_WRITE_OPS`, answering `DeleteTagResult`, with its refusals as
structured tool errors carrying their own codes. Catalogue grows to 45,
purely additive.
ADR-30 adds `publish_recovery_branch`, the ninth Vault management tool: a
write-gated mapping onto the collection management core's `publish_recovery`,
answering `PublishRecoveryBranchResult` (`VaultScheduleResponse`), rejected
inside `batch` like every management tool, and in
`is_collection_management_tool` like `sync_vault`, since publishing needs no
search model. `list_vaults` gains `recovery_branch` and the
`publish_recovery` capability through the shared `VaultSummary`. Catalogue
grows to 46, purely additive. ADR-35 adds `index_turn` to the same summary,
and the `list_vaults` description says what `running` and `waiting` mean;
additive. #421 (ADR-38) adds `read_docs` and `search_docs`, two read-only tools
over the Bundled manual that take no Vault. They are dispatched ahead of the
model-setup gate, answer under read or write
permission alike, and stay out of `READ_OPS` like `list_vaults`, so `batch`
refuses them as items. `read_docs` answers `ReadDocsResult` (the Home page plus
every page's name and title with no argument, one page otherwise) and refuses a
name that matches no page with the structured `docs_page_not_found` error;
`search_docs` answers `SearchDocsResult`. Both instruction variants name them.
Catalogue grows to 48, purely additive. #423 adds `docs` to a standalone tool
call's structured error when its `code` is one the manual explains
(`docs_pointers.rs` holds that code-to-page table): `{page, heading}`, where
`page` is a name `read_docs` accepts. `handle_tools_call` adds it to the
finished result, so `batch` item errors, `VaultOperationError` and HTTP bodies
never carry it, and errors for other codes, the writes-off `-32602` refusal and
the plain-text setup refusals are unchanged. Additive.

**Kind:** adapter/security surface.

**Owned paths:**

- `src/mcp/mod.rs`
- `src/mcp/adapter.rs`
- `src/mcp/auth.rs`
- `src/mcp/config.rs`
- `src/mcp/docs_pointers.rs`
- `src/mcp/protocol.rs`
- `src/mcp/results.rs`
- `src/mcp/routes.rs`
- `src/mcp/subscriptions.rs`
- `src/mcp/limits.rs`
- `src/mcp/tools/mod.rs`
- `src/mcp/tools/read.rs`
- `src/mcp/tools/write.rs`
- `src/mcp/tools/batch.rs`

**Public contract (target):** `/mcp` is Streamable HTTP served through rmcp's
`StreamableHttpService` (GET/SSE + POST + DELETE). Legacy `2025-11-25` traffic
keeps today's POST-only request/response shape and initialize/negotiation flow;
modern clients additionally open GET/SSE streams for server-initiated delivery.
Modern clients are stateless with no initialization handshake: `server/discover`
replaces `initialize`, each request carries per-request `_meta` that must match
the required protocol/capability HTTP headers, and `Mcp-Method`/`Mcp-Name`
validation is enforced. Advertised protocol revisions are exactly `2026-07-28`
and `2025-11-25`; older revisions are not negotiated.
Once #170 lands, the modern surface advertises `tools.listChanged: true`
honestly: modern clients receive tool-list change events via
`subscriptions/listen` backed by the existing `mcp_tools_changed` broadcast,
capped at four live subscriptions per bearer token (`subscriptions.rs` owns
the per-token registry and the validated-token request extension), with
acknowledgment, subscription metadata, rmcp SSE keep-alives, and disconnect
cancellation. The legacy handshake keeps advertising `tools.listChanged:
false`, so legacy clients continue reissuing `tools/list`. Layered resource protection (#171) exempts protocol/discovery/
list handling from the tool quota, limits tool calls to 120/minute/token and
concurrency to eight ordinary / two expensive searches, rejects over-limit
requests with HTTP 429 + `Retry-After`, and is explicitly disableable by
configuration (`HATCHDOOR_MCP_RATE_LIMITS_ENABLED`; `limits.rs` owns the quota
window, the concurrency pools, and the POST classification). A `batch` is
charged for the searches it carries (`limits::charge`, #327): one quota unit
per `search_notes` item (at least one, at most `BATCH_MAX_READ_ITEMS`), and an
expensive-search slot for its whole dispatch when any item searches, which
keeps its sequential searches inside the two-search cap.
Every `batch` item error carries a string `code` (#327): write mode's per-item
refusal is `mcp_writes_disabled`, an unwritable target path keeps its core code
through `JsonRpcFailure::domain_error`, and any other plain-text failure maps
to a code by JSON-RPC class (`invalid_arguments`, `internal_error`, ...) with
the number kept as `jsonrpc_code`. `get_attachment`'s base64 size refusal is
the structured `attachment_too_large_for_base64` tool error. Every tool response is a typed Rust result structure whose type
generates the `outputSchema` advertised in `tools/list` (#167), for the full
43-tool catalogue.
Internal JSON-RPC failures expose the stable `Internal server error` message
while the adapter logs diagnostics. `McpConfig`, server instructions, tool
names/schemas/results, and `HatchdoorMcpTransport` (the rmcp-backed transport
with its authorization/body-limit middleware) remain the boundary's public
surface; `adapter.rs` implements rmcp's `ServerHandler` seam over the
dispatcher, and `routes.rs` mounts it.
`list_vaults` exposes the shared redacted
Vault discovery/status/capability and revision shape. `get_tree` is the one
collection read that names more: since #192 it also takes optional `folder`,
`max_depth` and `include_notes`, so it no longer shares the scope-only schema
builder with `get_stats` and `get_graph`, whose schemas are unchanged. It maps
those three onto the read core's `TreeScope` and nothing else; the narrowing
itself, and the `folder_not_found` refusal, belong to the core. Every collection read
names `scope` (one Vault ID or `all`); every exact read, Markdown mutation, and
existing-Vault control names `vault_id`. Revisioned registry management calls the
Vault collection management core directly (#187) rather than proxying an HTTP
handler, and answers with the same shared collection shapes HTTP returns;
`create_vault` is the only zero-ID exception because the registry atomically
generates its immutable ID. MCP
returns shared domain failures as structured error tool results. Since #255
such a result signals its failure twice: `isError` on the result object, and
`ok: false` inside the structured payload beside the domain error's own `code`,
`message`, `retryable`, and optional `vault_id`. The two signals are
independent, so a client reading only the structured payload can still tell a
refusal from a success. Reading it that way is what the advertised
`outputSchema` invites, since that schema describes the success shape alone.
`src/mcp/protocol.rs`'s
`tool_structured_error` is the only place that marker is set, and the shared
Vault error type is deliberately not the carrier: it also serialises into HTTP
bodies and into `batch` item `error` values, neither of which changes shape.
No scope-less/default/sole-Vault tool remains reachable.
`get_attachment_import_config` names one Vault and answers under every write
posture, reporting the instance-wide write switch and that Vault's own
mutation capability as separate fields rather than refusing the call.
Typed results live in `src/mcp/results.rs`: each tool's success response is
produced from one Rust structure — MCP-owned shapes there, the read core's own
projections aliased for the Vault reads, and Vault collection management's wire
types for the registry controls — and that same structure generates the tool's
advertised `outputSchema`. Since #188 every read tool serializes that
projection exactly once, straight from the core; the decode-and-re-serialize
round trip through a proxied HTTP response body, and its 2 MiB cap, are gone.
`list_note_attachments` is a read tool on the read catalogue, reachable without
MCP write permission and without the mutation capability, as is
`get_frontmatter` — a body-free tags/aliases/properties projection of one note
served from the same authoritative Markdown read, carrying that note's
`content_hash` beside its other identity fields (#227) so a caller can prepare
a hash-protected write at frontmatter cost rather than reading every body.
`get_attachment` is the
outbound counterpart to `import_attachment`'s inbound flow, addressed by the
same `relative_path` `list_note_attachments` reports: `encoding: "url"` (the
default) returns a download transfer link (ADR-27, #310) on the Vault-scoped
`/transfers/{*path}` route, absolute and carrying its own credential, with
`expires_at`; `encoding: "base64"` inlines the bytes instead,
bounded by the same `HATCHDOOR_MCP_MAX_BASE64_BYTES` cap `import_attachment`
enforces on the way in. Resolution goes through
`VaultReadCore::contained_asset` (#188), so the Vault gate, containment, the
extension allow-list, the content type, and the browse surface are the same
ones the `/assets/{*path}` route answers on — a demoted or excluded asset is
refused identically on both surfaces, rather than MCP bypassing the surface
policy as it did while it reached into `handlers/assets.rs` directly. The
link's redemption runs through the same asset handler under the same byte
ceiling and tool budget as an MCP-admitted asset read, so it cannot reach an
attachment `get_attachment` would refuse. `get_attachment_import_config`
recommends the transfer link (`create_upload_link`) first, keeps the
bearer-token multipart route as the `alternative` for clients that hold the
token, and `import_attachment` as the base64 fallback, and says that a `.md`
target on the transfer link imports a note, the only method that takes one.
`update_frontmatter` is a
write tool over `vault/write`'s shallow top-level YAML merge primitive
(`update_note_frontmatter`): explicit null deletes a key, unmentioned keys
survive, nested mappings replace wholesale, and the body outside the leading
frontmatter block stays byte-for-byte unchanged. `create_vault` and
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
response buffer. Blocking work — index builds, snapshot reads, query embedding,
and filesystem reads — runs off the async runtime for every tool, through the
read core's own `VaultReads` offload rather than a per-adapter prologue.

**Consumed dependencies:** `AppState`, the four Vault-qualified cores
(`VaultReadCore`/`VaultReads`, `VaultSearchCore`, the Vault mutation core, and
Vault collection management), Vault registry/runtime, model setup, attachment
limits, the live configuration snapshot bound at each request, the Bundled
manual (`docs_bundle::{pages, home, page, search}`) for the two docs tools,
Instance state's `AgentConnectionLog` (`AppState.agent_connections`), fed the
client's name from rmcp's `RequestContext::client_info()` on every
`tools/call` (#426). No HTTP
adapter is consumed: since #188 no file under `src/mcp/` imports
`crate::handlers`, and ADR-19's MCP-to-handler proxying debt is retired.

**Coordination paths:** `src/server.rs`, domains exposed as tools, and
documentation describing agent behavior.

**Invariants:** MCP is disabled by default, uses its own token, validates
Origins, and keeps read-only access credentialed (ADR-09); the wire transport
itself is rmcp-owned rather than hand-implemented, and the advertised revision
set stays narrowed to `2026-07-28` + `2025-11-25` (ADR-17). Per-request security
ordering is preserved across the swap: enabled check → token-configured check →
Origin allowlist → constant-time bearer compare → protocol-version header.
The MCP bearer token is accepted by the multipart attachment endpoint only while
MCP *and* MCP write mode are both live-enabled, checked per request; token
changes, write enablement, Origins, and attachment limits apply to the next
request, and attachment authorization never retains a rotated MCP token.
Recording the last agent keeps only the client's `title` (or `name` when it
has none) and the time, never fails or slows the call (a due save runs on a
blocking task), and the record is never offered back over MCP (#426).

Since #186 every one of the write tools, eighteen since #258, has ADR-19's shape: it
validates its own arguments and then calls the Vault-qualified mutation core
once, mapping the typed outcome or the structured `VaultOperationError` onto a
tool result or a JSON-RPC failure. Two meanings live only here — a target path
this instance will not write (noise-excluded, or the reserved
`.hatchdoor-layer` marker) stays an `invalid_params` error, and an
instance-side failure an internal one whose detail this surface, unlike HTTP,
reports. `vault/write`, the `acquire_mutation` lock, the index build, the slug
lookup, the archive prefix, optimistic concurrency, and the path protections
(ADR-03) are all reached through that core, so both surfaces share one
implementation of each. Every write also runs off the async runtime, because
the core offloads it for every caller; before #184 this surface ran them
inline while HTTP offloaded them. `scoped_vault` gates with the core's own
`ensure_mutable`, and `acquire_mutation` takes the core's lock; the dispatcher
keeps holding that guard itself (`mod.rs` for one tool call, `batch.rs` for a
whole batch call on one Vault) because a batch's critical section is wider
than any single operation. `batch` is a loop over that same per-item dispatch,
with its `expected_content_hash` chaining and its asymmetric read/write caps
unchanged.

Since #188 the read tools have the same shape, in `tools/read.rs`: parse the
arguments, call `VaultReadCore`, `VaultSearchCore`, or Vault collection
management once through the core's offload, and map the typed projection or the
structured failure onto a tool result. `list_note_attachments`,
`get_attachment`, `get_frontmatter`, and `get_attachment_import_config` moved
out of `tools/write.rs` with that change, and the local index build, slug
lookup, and raw asset resolution they kept there are gone.

What stays in this adapter is what only this transport knows: its own argument
names and empty-field wording, the `new_title` path-separator rule, the
`replace_section` mode spelling, the base64 encoding option on
`get_attachment`, and `import_attachment`'s base64 envelope —
whitespace-tolerant, capped on the *encoded* length before it is decoded, with
the core then applying the authoritative check to the decoded bytes.

**Validation:** `cargo test mcp`, vault write tests for mutation changes,
server router tests, and golden wire tests locking both supported revisions'
request/response shapes. Before releases, the manual conformance-run procedure
(#166) produces mandatory release evidence.

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
arguments, and reproducible comparison behavior. Every cache-querying CLI mode
(`run`, `rerank`, `hybrid`, and `compare`) validates the exact stamped
`Embedder::identity()` before querying; an absent or unequal identity requires
a disposable-cache rebuild. Rerank reports preserve heading paths and publish
correct-heading plus category/tier/language slices alongside post-rerank
quality metrics. `index_microbench` validates the active representation stamp
and labels the representation it measures. Both binaries declare
`required-features = ["eval"]`, so every documented invocation carries
`--features eval`.

**Consumed dependencies:** cache, embeddings, chunking, Search, and Reranking,
plus the `eval`-gated candle stack (`candle-core`, FastEmbed's `qwen3` and
`nomic-v2-moe` features, and the version-matched `tokenizers-fe` alias).

**Coordination paths:** `eval/**`, `Cargo.toml`'s `eval` feature and `[[bin]]`
entries, the contributor guide's verification commands, related findings under
`docs/`, and model or chunking code when experiments become runtime decisions.

**Invariants:** hybrid and rerank experiments remain offline unless ADR-05 is
superseded; the harness stays behind the non-default `eval` feature, so no
default, verification, or production build compiles a crate that only the
harness reaches. Crates a production dependency also needs are unaffected:
`fastembed` requires tokenizers 0.22 unconditionally, so that second tokenizers
version stays in the default tree even though Hatchdoor's own edge to it is now
`eval`-only.

**Validation:** `cargo test eval`, binary argument tests, and the relevant eval
command for behavioral changes. Because a default `cargo test --all` skips both
binaries' test targets entirely, the `cargo test --all --features eval` run in
`just check` is what keeps them from rotting.

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
- `frontend/src/app/AppErrorBoundary.tsx`
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
`main.tsx` wraps the router in `app/AppErrorBoundary.tsx` (#339), so a render
that throws degrades to a message and a reload button instead of a blank
page. The boot path's own storage reads cannot be that throw: WebKit throws
`SecurityError` from the `localStorage` accessor when site data is blocked,
so `lib/storage.ts` exports `safeGetItem`/`safeSetItem`/`safeRemoveItem`
(null or no-op on a throw), which `getStoredNumber`, `getStoredString`,
`App.tsx`'s shell preferences, `hooks/useTheme.ts`,
`startup/useStartupStatus.ts` and `NotePage.tsx`'s two preferences all use.
`App.tsx`'s `<Routes>` ends in a `path="*"` catch-all (#339) that renders a
"Page Not Found" `StateBlock` with a "Go to notes" action, so a stale or
pre-#137 link never leaves the note pane empty.
`main.tsx` also owns when the app may reload itself for a new service worker
(#330). Registration stays `autoUpdate`, but the reload runs through
`onNeedReload`, and both that and every `registration.update()` ask
`lib/reloadGuard.ts` first, so a nightly build cannot activate and reload
across an unsaved edit. The editor takes the hold; nothing else does. The
update check runs on a one-hour interval and when the tab becomes visible
(not also on `focus`, which fires for the same return), and swallows the
rejection `update()` gives while offline (#332). `vite.config.ts` keeps
everything only a Mermaid or PDF.js dynamic import reaches out of the
install-time precache, found from the bundle graph by `findLazyChunks`, and
caches those chunks CacheFirst on first use instead; the manifest's splash
colours are the dark theme's, since the manifest has no light/dark form.
`useVaultScope.ts` owns
the selected Vault scope (state/storage, per #137) and the Vault-less-action
default (`resolvePrimaryVaultId`); the Vault collection itself belongs to the
Vault collection client below (#198). It reads that client to reconcile the
stored scope (#335): once discovery has answered with at least one enabled
Vault and no registry recovery, a scope naming a Vault that is missing,
paused, or `activation: "unavailable"` reads as `all` in the same render, is
written back as `all`, and returns a `ScopeFallbackNotice` that `App.tsx`
shows in the shared notice strip. Every instance (App, Graph, Statistics)
reconciles itself, and the revision stream re-renders them all, so a Vault
paused from Settings, another tab, or an MCP agent is caught on its revision. `app/ExplorerPane.tsx`'s Scope zone (#138) calls
`setScope` on the desktop; `app/AppTopbar.tsx`'s scope row and its bottom
sheet (#145) call it below 920px, where the Scope zone itself does not
render. The breakpoint keeps the two callers mutually exclusive — every other
collection-read and Vault-picking call site only reads the selected scope.
`vaultSlot.tsx`/`vaultSlotLogic.ts` (#139) derive each Vault's trailing
count-or-condition slot and the shared All-Vaults/collapsed-head aggregate
from `VaultSummary`'s status fields alone — no new endpoint. A Vault whose
`index_turn` is `waiting` (ADR-35) shows the still `waiting` word wherever it
would otherwise show indexing, unless it is ready or carries a search error.
`vaultSlotLogic.ts`'s `noteInSyncConflict` is also imported by Note reading's
`NotePage.tsx` (ADR-30) to tell whether the open note is on its Vault's
conflict list; this is a deliberate cross-capability import of one pure
function rather than a duplicated copy of the Git code it checks.
`lib/storage.ts`'s `isEditableTarget` is imported by `NotePage.tsx` on the
same terms (#331), so document-level undo recognises editable targets with the
shell's own keyboard-shortcut test.
Note counts reach the slot from the
collection client, which reads them at `"all"` scope independently of the
browsing scope and refreshes them on the collection revision. The topbar's `Tree Stale` badge is deleted (#139) with
nothing replacing it; `Offline` is the only condition left there, because it
is about the workspace and not about any one Vault.
`app/vaultAccordion.ts` (#142) is `app/ExplorerPane.tsx`'s per-Vault
accordion under `all`: pure derivation for the landing default (the open
note's own Vault, else the last persisted, else nothing), the unavailable-
Vault unfold gate, the `LAST_UNFOLDED_VAULT_KEY` persistence pair, and the
per-Vault namespacing of the shared `expandedFolders` record the accordion's
folder-open memory needs. `vaultFolderUpdate` lifts a Vault tree's folder
update to that whole record, reading the Vault's slice from the record it is
applied to rather than from a render's snapshot (#305). Unfolding a Vault never calls `setScope`, same
invariant as the Scope zone's own narrow-scope call being the only one.

Narrowing the scope to one Vault also moves the reader. `App.tsx`'s
`handleScopeChange` wraps `setScope` at both call sites and, when the reader
is on a note route, navigates to the note that Vault was last left on, or to
`"/"` when that Vault has none remembered. `lib/storage.ts` holds that memory
under `LAST_NOTE_BY_VAULT_KEY` as `vaultId -> slug`, written alongside
`LAST_NOTE_KEY` whenever the open note changes and pruned to the browsing
list whenever discovery settles, for the reason `clearStoredLastNote` exists:
a Vault that is gone or paused only resolves to "Vault definition was not
found". An empty browsing list never triggers that prune, since a broken
registry produces one too and it is not evidence that anything departed.
`LAST_NOTE_KEY` stays the single landing note the `"/"` redirect and the
accordion's landing default read. Four cases move nobody: widening back to
`all`, picking the Vault whose note is already open, an unchanged pick, and a
pick made anywhere but a note route (Settings, Graph, Statistics, the empty
landing), where the scope is a filter rather than a request to go and read
something; the note route is matched with the router's own `useMatch`, not a
second spelling of the path. A switch that lands on `"/"` clears
`LAST_NOTE_KEY` as it goes: nothing is open any more, so the landing redirect
finds nothing to put back, now or after a reload. What it does not do is
check that a remembered note still exists — a note deleted since is a
not-found page that heals as soon as any note in that Vault is opened, the
same bargain the landing redirect already makes.

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
the collection client's `demoMode` — a demo instance's own
description reads "This demo has no Vaults loaded." in that state (#152),
never the ordinary "Add a Vault…" sentence with nothing left to act on it)
versus a broken start:
The collection client
also exposes `recovery` (the persisted registry file is unreadable), rendered
as the documented error block with a `Try again` action (a plain re-fetch).

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
scope row/sheet and its single-Vault condition row (`AppTopbar.tsx`,
#334), the explorer accordion and single-Vault `Notes` head, and each graph
island caption (`GraphPage.tsx`, below) — `vaultSlotLogic.ts`'s
`deriveVaultSlot`/`deriveVaultAggregate`/`describeScopeSlot` take an optional
trailing `demoMode` parameter (default `false`) that clamps every condition
to the amber tier and swaps the Vault's own runtime message for the
instruction-free fallback sentence: nobody browsing a public demo is the one
who would act on an operator diagnostic, and the red tier's bordered ground
is reserved for something the app is not already handling. `VaultSlot`/
`VaultAggregateSlot` (`vaultSlot.tsx`) take the same optional `demoMode` prop
and thread it straight through, as does `App.tsx`'s own `describeScopeSlot`
call for the shell's scope live region. `deriveVaultSlot` reads the
server's Git codes: `managed_git_conflict` is `conflict` and
`managed_git_dirty_working_copy` is `sync stopped`, both error tier; any
other Git failure is `sync failed`, warn tier. No slot word blocks a save
(#372): both conditions halt only commit and sync, so a note in such a Vault
stays editable and saves land on disk.
`noteInSyncConflict` (`vaultSlotLogic.ts`, ADR-30) answers whether an open
note is on the Vault's current `managed_git_conflict` file list, restoring the
`.md` extension and the Vault's repository subfolder that note reads drop;
`NotePage.tsx`'s `SyncConflictNotice` renders a non-blocking notice from it.

**Coordination rule:** feature work may touch `App.tsx` only when the work
packet names the route, callback, shortcut, or state integration. A large prop
surface is a coordination seam, not permission to move feature behavior into
the shell.

**Validation:** the applicable `App.*.test.tsx` (including
`App.demo-mode.test.tsx`, #152, and `App.demo-startup-boundary.test.tsx`,
#339), `app/AppErrorBoundary.test.tsx`, `app/ExplorerPane.test.tsx`,
`app/AppTopbar.test.tsx`, `app/vaultSlot.test.tsx`, `useVaultScope.test.ts`,
`App.scope-reconcile.test.tsx` (#335),
`vaults/vaultCollection.test.ts`, `useTheme.test.tsx`, storage tests, then full
frontend checks. Service-worker and PWA changes (`main.tsx`, `vite.config.ts`)
also need `pwaPrecache.test.ts` (lazy-chunk precache exclusion and runtime
caching) and the PWA contracts in `clientAuditContracts.test.ts`, plus a
production build to confirm the precache manifest. Layout changes to
the explorer pane need a browser as well as the suite: its zone structure
depends on real cascade behavior that jsdom does not reproduce.

### Vault collection client

**Kind:** feature/shared client.

**Owned paths:**

- `frontend/src/vaults/index.ts`
- `frontend/src/vaults/vaultCollectionStore.ts`
- `frontend/src/vaults/useVaultCollection.ts`
- `frontend/src/vaults/vaultProjection.ts`

**Public contract:** `frontend/src/vaults/index.ts` is the only import path.
It exposes the collection snapshot (`readState`, the derived read state
`loading`/`error`/`empty`/`partial`/`ready`; `vaults`, the enabled browsing
list; `allVaults`, the registry list Vault management renders; `demoMode`,
`loading`, `error`, `recovery`, `registryRevision`,
`revision`, `noteCounts`, `noteCountsPartial`), `refresh`,
`fetchRegistryRevision`, and the demo-aware slot projection (`slotFor`,
`describeScope`).

`readState` (#333) is what a surface branches on to tell a failed read from an
empty collection. `error` means discovery failed and none has ever succeeded;
`empty` means discovery answered with no enabled Vaults (a broken registry is
also `empty`, and `recovery` says which); `partial`
means the list is known but `noteCountsPartial` is set, because the stats read
failed, answered `partial`, or left out an enabled Vault; `ready` means
everything answered. A refresh that fails after a successful discovery keeps
the last known list and sets the `error` field, but `readState` stays `empty`,
`partial` or `ready`. The `"/"` landing route, the Settings Vault index and the
Statistics page render `error` as "Vaults Unavailable" with a Try again that
calls `refresh`. `noteCounts` merges each stats answer into the counts already
held, so a Vault missing from a partial answer keeps its last known count.

**Contract and responsibility:** one module owns everything about the Vault
collection that more than one surface reads (#198): the Vault list, the
per-Vault note counts from `GET /api/v1/vaults/all/stats`, the demo-mode
projection of `app/vaultSlotLogic.ts`'s slot vocabulary, and the
`/api/v1/vaults/events` `vault-collection-revision` stream that invalidates
all three. The store is a module-level singleton behind `useSyncExternalStore`,
not a provider: the first subscriber starts the one collection read and opens
the one SSE subscription the whole app shares, and the last to unmount tears
both down, so no two surfaces can disagree about the same Vault and a Vault
mutation refreshes every surface without any of them refetching. The two
inputs each call site used to decide for itself — which count source applies,
and whether demo mode applies — are decided here; a surface with its own count
for a Vault (the graph's island node count) passes it to `slotFor` as an
override. Per-surface presentation stays in the surfaces: the presentational
slot components (`app/vaultSlot.tsx`, and the aggregate the accordion and note
page render) still take `demoMode` and a note count as props. They apply a
decision rather than making one — the client is where it is made, and the shell
hands it down — so the slot vocabulary stays renderable in isolation and its
own suites keep testing it that way.

`revision` is the collection revision the published state reflects, and it is
`null` — not `0` — until a discovery lands. The two were one sentinel until a
freshly restarted server, genuinely at revision 0, was found to spend its
first real change being read as "nothing known yet". The baseline is seeded
from the discovery response's own `collection_revision`; once known, only the
event stream moves it, which is what keeps a revision counting from zero again
after a restart a change consumers follow rather than one a later discovery
undoes. Seeding matters because the stream reports the server's current
revision the moment it connects rather than a delta, so against a `0` start
that first event always read as an invalidation and every consumer keyed on it
reloaded.

A refresh that finds nothing new keeps the previous value's identity, and a
patch that changes nothing publishes nothing. Without that, a note write — which
bumps the collection revision — would hand every consumer a fresh-but-identical
Vault array and relayout the graph. `VaultSettingsDetail` seeds its editable
drafts once per Vault but adopts every genuinely new record for display, so it
cannot describe a Vault the Settings index disagrees with.

**Consumers:** `App.tsx`, `hooks/useVaultTree.ts`, `hooks/useVaultScope.ts`
(#335, reconciles the persisted browsing scope against the live collection),
`components/graph/GraphPage.tsx`, `components/StatsPage.tsx`,
`features/settings/VaultSettingsIndex.tsx`, and
`features/settings/vaultGitBehavior.ts`.

**Invariants:** disabled Vaults never appear in `vaults` and never participate
in `"all"`; counts are always read at `"all"` scope regardless of the browsing
scope; exactly one `/api/v1/vaults/events` subscription exists per app;
nothing outside this directory fetches `GET /api/v1/vaults` or
`GET /api/v1/vaults/all/stats`. A `readState` of `error` is never an empty
collection: no surface renders it as the zero-Vault state, and no consumer
takes its empty lists as evidence that a stored Vault or last note has left
the collection. A Vault with no `noteCounts` entry has an unknown count, which
every surface renders as the unknown marker, never as `0`. `VaultSettingsDetail`'s
`expected_registry_revision` is deliberately not one of these: it is a
mutation-sequencing token advanced by each step's own response, seeded from
the client and then owned locally.

**Validation:** `vaults/vaultCollection.test.ts`, then the consumer suites
(`App.*.test.tsx`, `components/graph/GraphPage.test.tsx`,
`components/StatsPage.test.tsx`,
`features/settings/VaultSettingsIndex.test.tsx`), then full frontend checks.

### Frontend API, authentication, and shared wire contracts

**Kind:** infrastructure/shared contract.

**Owned paths:**

- `frontend/src/api/api.ts`
- `frontend/src/api/apiError.ts`
- `frontend/src/components/TokenPrompt.tsx`

**Shared path:** `frontend/src/types.ts`.

**Contract and responsibility:** authenticated/time-bounded fetch, unauthorized
notification, tokenized asset/download/SSE URLs, error extraction, login prompt
(whose "Where do I find my token?" and "Help" links open the Help reader, #417),
and cross-capability TypeScript representations of backend payloads. A feature
may own its wire types when all consumers go through that feature's public
entry point, as Search now does.

**Consumers:** almost every data-backed frontend capability.

**Coordination rule:** `types.ts` is not owned by whichever feature needs one
new field. Contract changes must list the backend serializer and all frontend
consumers. New feature-local types should remain local unless genuinely shared.

**Invariants:** preserve bearer/header behavior and the deliberate query-token
fallback (ADR-08). Never log or render tokens. When the browser refuses to
store the web token, `setToken` keeps it in page memory, prefers it over any
older stored token, and returns `false`; a caller must then apply it without a
page reload, which would forget it (Unlock remounts the app in place). The
unauthorized notification fires only for a 401 answering a request sent with
the token still current, so answers in flight from before an Unlock cannot
lock the new session (#339).

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
(accept/decline Gemma, retry). The poll runs once a second while it succeeds;
after a failed or unreachable poll it backs off (`startupPollDelay`: 2s, 4s,
8s, capped at 30s) and returns to 1s on the first success (#333). Production `App.tsx` resolves Vault discovery
before enabling this polling, so broken-registry and zero-Vault workspaces
never poll or gate; it passes the resulting discovery plus startup
`status`/`retryModelSetup` to its internal `VaultWorkspace` composition and
the gate inputs to `StartupGate` (#150: the gate
shrinks to exactly the `terms_required`/first-`downloading` model step —
`hasSteppedPastGate` latches true the first time any other state is
observed and never re-arms, so a later retry-triggered `downloading` never
reopens the full-screen gate). `terms_required` is the exception to the
latch (#339): nothing else in the app can accept or decline Gemma, so it
gates again after the latch. A demo instance never gates on `terms_required`,
latched or not, since its server 404s the choice (`StartupGate` takes the
collection's `demoMode`); the search dialog's demo sentence covers it. The gate
also holds its decision until discovery has resolved and the first startup
answer has landed (#339), rendering a bare `.startup-shell` meanwhile, so the
workspace is never mounted only to be unmounted a fetch later when that
answer gates; a failed poll (`connectionIssue`) releases the hold, and a
zero-Vault or broken-registry workspace, which never polls, is never held.
Every other state — `scanning`, `indexing`, `ready`, `failed`, a post-latch
`downloading`, and anything registry- or zero-Vault-related the gate never
observed in the first place — renders the ordinary workspace, which reads
the same `status` for its own surfaces: `app/ExplorerPane.tsx`'s Scope zone
slot (`StartupProgress`, which `App.tsx`'s `deriveStartupProgress` also
derives for a post-latch `downloading`, #339) and
`features/search/SearchDialog.tsx`'s work-in-flight/downloading/terms/failed
blocks. The latch itself is read and written through `lib/storage.ts`'s
guarded helpers, so blocked site data leaves it unset rather than throwing.

**Consumed dependencies:** shared API client, theme hook, and the Help
reader's `ContextualHelpLink` and `CONTEXTUAL_HELP` (the model choice's
"How does this work?" link, #423).

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
- `frontend/src/lib/vaultTrees.ts`
- `frontend/src/styles/layout-explorer.css`

**Public contract:** `useVaultTree`, explorer tree/list components, derived
folder paths, and flattened note candidates. Since #192 the tree route sends
notes without a `vault_id` — the tree they hang from carries it — so
`lib/vaultTrees.ts` stamps each note with its Vault as the response is parsed,
before `useVaultTree` merges or flattens the trees and the grouping is gone.
Each flattened candidate (`NoteCandidate`) also carries its Vault-relative
`relativePath`, built from the folder chain and the note's title, which is its
file name; the editor writes a Markdown link's path from it (ADR-33).
`WireVaultTree` is the payload shape and `VaultTree` the attributed one every
component below the hook consumes. The sidebar is three zones — a
fixed rail, a scrolling nav, a fixed footer — and `.explorer-nav` is the scroll
container the shell restores scroll position against, not the pane itself. On
desktop with more than one enabled Vault, the shell-owned Scope zone (#138,
`app/ExplorerPane.tsx`) pins a fourth zone above the rail; it shares this
file's CSS but is not part of this capability's owned React contract.
`ChangesPanel` lists notes changed on disk, newest first across every Vault in scope with no per-Vault share (#341). `useVaultTree` asks `/recent` for the API's ceiling of 25 while the panel shows 15, because the server returns no total: the rows past 15 are what make its `and N more` line and its head count true. It deliberately carries no unread
count, because distinguishing external changes from the user's own edits needs
backend data that does not exist yet. Changed on disk carries the shared
`VaultPrefix` provenance marker (#140) on each row when scope is `all` and
more than one Vault is enabled. Recently viewed is a viewing history rather
than a collection read, so it spans Vaults at every scope: it carries the
prefix whenever more than one Vault is enabled, and keys its rows by Vault
plus slug (#334). A single-Vault instance renders neither prefix. `useVaultTree` also exposes `modifiedNotesPartial` and
`modifiedNotesMissingVaults` from the `/recent` read's own envelope (#141);
`ChangesPanel` never banners a partial read — a trailing warn-ink line below
the last row names only the missing Vaults, and `StateBlock tone="error"`
replaces the empty state outright when nothing is usable. A `/recent` read
that failed outright is `modifiedNotesError`, which `ChangesPanel` renders as
a `Could Not Load` error block with Retry rather than as "Nothing has changed
on disk yet" (#334). Every tree and recent read carries the current scope's
`AbortSignal`; a scope change aborts it, and a read answers only while it is
the newest of its kind for the scope it was started under, so a slow `all`
answer can never land after the narrowed one that replaced it (#334).
`useVaultTree` reads once per collection revision: it records the
`collection_revision` its loaded tree came back with, and a revision event
matching it is not a reload. A read still open is awaited before that
comparison, because the discovery revision lands while the very first tree
read is in flight and there would otherwise be nothing to compare against. A
collapsed folder renders none of its children — the browser hides a closed
`<details>`' content anyway, so mounting a row per note bought DOM and render
time and nothing else; the cost is that find-in-page no longer reaches a note
inside a collapsed folder. A folder's children also mount whenever its own
`<details>` last reported itself open, so it can never show open and empty
(#305). `FolderTree`'s and `ExplorerPane`'s `onExpandedFoldersChange` carry an
`ExpandedFoldersUpdate`, a function of the previous record rather than a whole
next record, and the accordion applies its per-Vault namespacing inside it
through `app/vaultAccordion.ts`'s `vaultFolderUpdate`:
React dispatches `toggle` through every ancestor `<details>`, so several
folders can write in one batch, and each write has to see the others. A
`FolderNode` ignores a toggle whose target is not its own element.
Showing the open note's folders is temporary and never written to the record (#365). A `FolderNode` saves a toggle only when the element disagrees with its own `open` prop, which means the reader clicked it. `FolderTree` remembers the folders the reader closed above the open note for that note alone, so a note's folders close again when the reader moves on and reopen for a later note inside them. The tree read's own `partial` (`treePartial`)
and the Vaults it left out (`treeMissingVaults`) reach `app/ExplorerPane.tsx`
(#334): a trailing `.explorer-tree-partial` warn-ink line under the tree
names them, an unfolded accordion Vault the read left out says it did not
answer instead of showing an empty section, and a settled read with no tree
at all shows a `Nothing Found` error block with Retry rather than a blank
pane. At exactly one enabled Vault the flat tree's `Notes` head carries that
Vault's `VaultSlot`, since no Scope zone, accordion or mobile scope row
renders there. While that slot reports a condition the head takes
`.is-pinned` and sticks to the top or bottom edge of `.explorer-nav`, so a
long tree or long Changed on disk and Recently viewed lists never scroll it
out of view; on mobile, where the head sits in the closed drawer,
`app/AppTopbar.tsx` shows the same condition in a non-pickable
`.topbar-mobile-meta` row (#334). A healthy single Vault gets neither.
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
fetch. Its `isNoteRoutePath` states the same route grammar
without decoding it, for the note-page renderer deciding whether an href in a
note body is a route the router should take.

**Consumed dependencies:** shared API/error utilities, shared wire types,
shared UI components (`components/ui.tsx`'s `VaultPrefix` and `StateBlock`),
`lib/vaultParticipants.ts`, and router links.

**Coordination paths:** `App.tsx`, `app/ExplorerPane.tsx`, `types.ts`,
`lib/stateCompare.ts`, `lib/vaultParticipants.ts`, responsive CSS, and backend
tree/recent/event endpoints.

**Validation:** folder/note-candidate/state comparison tests and affected App
navigation tests; `components/Explorer.test.tsx` covers nested folder
open/close, the active note's ancestors, and the open-but-empty invariant
(#305), plus the note's folders shown without being saved and the reader's close above the open note lasting until the note changes (#365); `app/ExplorerPane.test.tsx` covers the tree and list
components in composition, including the single-active-highlight invariant and the accordion's namespaced record staying empty for a shown note while a reader's close is saved under the Vault's key (#365).
`hooks/useVaultTree.test.ts` covers the `/recent` read's partiality at three
and eight Vaults, the tree read's partiality, a failed `/recent` read, a
superseded `all` read answering after the narrowed one (#334), and the 25-row `/recent` read (#341); `app/ExplorerPane.test.tsx` covers the panel's fifteen rows, `and N more` line and head count (#341).

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
`useSearch` takes no scope: the fetch is always `vaults/all/search`, at the
50-row ceiling `clamp_search_limit` allows, whatever the browsing scope is.
`SearchDialog` takes `vaults`/`scope` and shows the shared `VaultPrefix`
provenance marker (#140) on a result's path line whenever the visible rows
can span Vaults — the dialog's own filter on `all`, at more than one Vault —
the same multi-Vault condition Vault Explorer's lists use, read off the
filter rather than the browsing scope now that the two can differ. The path
itself elides head-first (`.result-path-text`) so the never-eliding prefix
always reads.
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
fresh on every open. It opens on `scope` and a tag tap overrides that, so
narrowing the sidebar decides what the reader is shown first without
deciding what was asked. Both seeds are filtered through the enabled Vaults
and fall back to `all`, because `useVaultScope` reconciles the stored browsing
scope only once discovery has answered and a tag tap can name any Vault: a
Vault disabled since it was last browsed would otherwise open the dialog
filtered to a row that does not exist. The seed runs once, so the chosen filter is also read through
the live collection on every render: a Vault that leaves the collection while
the dialog is open drops the filter back to `all` rather than leaving the
control reading "All results" while it hides every row (#334). The panel's
Tab trap counts only controls that are actually rendered, because the phone
field strip stays in the DOM under `display: none` on desktop. A Vault that was asked and did not answer keeps its seeded
selection but suppresses the "No results in X" line — the row's own `no
answer` and #141's partial sentence say what happened, and claiming the
Vault has no matches would be the exact lie #141 exists to prevent. Two shapes, one meaning: a `.search-facet-rail`
column beside the results on desktop (absent only at one enabled Vault),
and a `.search-field-strip` `Scope`-beside-`Mode` pair (§18's field grammar)
that replaces the desktop Mode checkbox below 920px — both rendered
unconditionally and toggled by the same CSS breakpoint `responsive.css`
already uses, so no `isMobile` prop crosses the boundary. Filtering is a
client-side `Array.filter` over the already-fetched results; no re-fetch, no
re-ranking. `buildFacetRows` has three row states, not two: a count, the
inert `no answer` condition for a Vault that was asked and did not answer,
and an empty slot for every Vault before any search has run, which keeps the
rail a selector from the moment the dialog opens rather than a column of
`0`s that means nothing yet.

`SearchDialog` also takes `startupStatus`/`onRetryModelSetup` (#150), the
shrunk startup gate's own data (`startup/useStartupStatus.ts`): while
`scanning`/`indexing`, the result area shows a work-in-flight block
carrying the same percentage the Scope zone shows, with the query input
left enabled and the topbar's search entry point never greyed; on a failed
model download it shows the reason with a "Retry setup" action instead of
the ordinary empty/error states. A post-latch `downloading` (the re-download
"Retry setup" starts) and `terms_required` get their own blocks too (#339),
so neither falls through to "No matching notes." With `demoMode` (#339) the
failed and terms states read "Search is unavailable on this demo right
now." with no action: never the server's operator diagnostic, never a retry
the server 404s in demo mode. All of these replace the normal
loading/error/empty rendering only — the facet rail and results list underneath are unaffected
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

### Help reader

**Status:** Added by #417 (ADR-38 decision 3).

**Kind:** product capability.

**Owned paths:**

- `frontend/src/features/help/index.ts`
- `frontend/src/features/help/HelpProvider.tsx`
- `frontend/src/features/help/HelpPanel.tsx`
- `frontend/src/features/help/useHelp.ts`
- `frontend/src/features/help/helpPages.ts`
- `frontend/src/features/help/contextualLinks.ts`
- `frontend/src/features/help/ContextualHelpLink.tsx`
- `frontend/src/features/help/help.css`

Feature tests:

- `frontend/src/features/help/HelpPanel.test.tsx`
- `frontend/src/features/help/helpPages.test.ts`
- `frontend/src/features/help/contextualLinks.test.ts`

**Public contract:** `frontend/src/features/help/index.ts` is the only public
TS/TSX entry point. `HelpProvider` (props `demoMode`, `signedOut` while the
token prompt is up, which lifts the panel above it, and
`onOpenSetupChecklist`, #419, which adds a "Setup checklist" card to Home
that closes Help and calls it) owns whether Help is open,
the page it shows and the pages behind Back, and mounts the panel. `useHelp()`
returns `{ openHelp(page?, heading?), closeHelp, isOpen }`; outside a provider
it does nothing. `page` is a manual page name such as
`guides/how-to-set-up-a-git-backed-vault` (no page opens Home, `index`), and
`heading` is a heading anchor in the note slug rule; the panel scrolls to it.
`HELP_PAGES` names the pages other features open Help at. Since #423,
`CONTEXTUAL_HELP` is the one table of where each "How does this work?" link
opens Help (`{page, heading?}` per screen or condition), with
`vaultConditionHelp(vault, paused)` and `gitConsoleHelp(vault)` choosing the
entry for a Vault's condition line and Git console, and `ContextualHelpLink`
(prop `to`) is the link itself, a `.help-link` button. `contextualLinks.test.ts`
reads `docs/user-vault` from disk and fails when a page or heading in the table
is missing. Help CSS is integrated through the `App.css` stylesheet aggregation
seam.

**Behaviour:** Help is an overlay beside the work (the #417 resolution): fixed
to the right under the topbar, the screen underneath keeps its width, full
width covers the work area, and below 920px it is full screen. Escape closes it
unless a dialog above it owns the key, and focus returns to where it was. Pages
come from `/docs/<page>.md` and search from `/docs/search`, through plain
`fetch` with the web token attached when one is stored, never `apiFetch`: a
401 there means a private page, not a lost session, so it must not raise the
token prompt. Links resolve against the page's own `/docs/` address and stay
inside Help; other links open in a new tab. Pages render through
`createManualMarkdownComponents`, so the manual looks like a note.

**Consumed dependencies:** the public manual routes (`src/handlers/docs.rs`),
`types.ts`'s `VaultSummary` (whose `activation`, `git`, `search` and `*_error`
codes `vaultConditionHelp` and `gitConsoleHelp` read, including ADR-30's
`managed_git_conflict`), the shared `test/fixtures/vaults.ts` builders (tests
only), `api/api.ts`'s `getToken`, `createManualMarkdownComponents` and
`lib/noteHeadings.ts` from Note reading and rendering, and the icons in
`components/icons.tsx`. It also borrows other modules' CSS classes, so a change
there reaches Help: the Search dialog's result rows (`.search-results`,
`.search-group`, `.search-result--primary`, `.result-title`,
`.result-path-text`, `.result-snippet`), Note reading's `.note-body` prose
styles, and Shared UI's `.state-block` and `.icon-button`.

**Coordination paths:** `App.tsx` (mounts `HelpProvider` in `AppSession`, wires
the topbar, and passes `onOpenSetupChecklist` while signed in and not in demo
mode), `App.css`, `app/AppTopbar.tsx` (the `?` button on wide screens and
the first `…` menu item on phones), and `components/TokenPrompt.tsx` (its two
Help links). The contextual links (#423) sit in `App.tsx` (No Vaults Yet, Vaults
Unavailable and the registry recovery screens), `startup/StartupGate.tsx` (the
model choice), `features/settings/SettingsPage.tsx` (each section head, and the
MCP writes row or its "Managed outside this page" entry) and
`features/settings/VaultSettingsIndex.tsx` (a Vault's condition line, its Git
console, and the index's recovery blocks), and
`features/settings/FolderPicker.tsx` (`folderOutsideMount`, #430), each
passing a `CONTEXTUAL_HELP` entry and nothing else.

**Invariants:** Help never fetches or shows Vault content and calls no
`/api/` route; it never needs the web token; a `base` block renders as its
source. Heading ids inside Help carry a `help-` prefix so they never collide
with the note open underneath.

**Validation:** `npx vitest run src/features/help src/app/AppTopbar.test.tsx
src/App.startup-auth.test.tsx src/App.startup-workspace-states.test.tsx
src/startup src/features/settings`, then full frontend checks.

### What's new pop-up

**Status:** Added by #418 (ADR-42).

**Kind:** product capability.

**Owned paths:**

- `frontend/src/features/whats-new/index.ts`
- `frontend/src/features/whats-new/WhatsNew.tsx`
- `frontend/src/features/whats-new/whatsNew.ts`
- `frontend/src/features/whats-new/whats-new.css`

Feature tests:

- `frontend/src/features/whats-new/WhatsNew.test.tsx`
- `frontend/src/features/whats-new/whatsNew.test.ts`
- `frontend/src/App.whats-new.test.tsx`

**Public contract:** `frontend/src/features/whats-new/index.ts` exports only
`WhatsNew`, a component with no props. Mounted once, it reads
`GET /api/v1/whats-new` and shows nothing or one centred dialog (the #418
resolution). The browser remembers the last version it dismissed What's new
for under the `localStorage` key `hatchdoor_whats_new_seen`, as a base version
(`2.8.0`, never the ` (dev …)` suffix). CSS is integrated through the `App.css`
stylesheet aggregation seam.

**Behaviour:** the releases shown are the server's list, newer than the stored
version when there is one, newest first. Action-needed items from every
listed release are pinned in one box at the top, each tagged with its version;
each release then lists its other items. A highlight's link and "Full
changelog" open Help through `useHelp()`; Help sits above the dialog, which
moves left of it and stays unseen. "Got it" or Escape marks the running version
seen; with Help open, Escape closes Help first. Nothing shows on a fresh
install, when nothing is new, when the request fails, or when `localStorage`
throws, since a dismissal that cannot be remembered would bring it back on
every load. Highlight text renders as inline Markdown only.

**Consumed dependencies:** the What's new endpoint (`src/handlers/whats_new.rs`)
through `api/api.ts`'s `apiFetch`, `useHelp()` from the Help reader, and
Shared UI's `UiButton`. It borrows the shell's `.modal-backdrop` and
`.modal-panel` and Help's `.help-eyebrow` and `.help-link`, and sits beside
Help by matching `.help-panel`'s width and staying under its layer, so a
change to any of those reaches it.

**Coordination paths:** `App.tsx` (mounts it inside the startup gate's
children in `AppSession`, signed in and never in demo mode) and `App.css`.

**Invariants:** never shown in demo mode or on a fresh install; a storage
failure never throws and never shows the dialog.

**Validation:** `npx vitest run src/features/whats-new
src/App.whats-new.test.tsx`, then full frontend checks.

### Update banner

**Status:** Added by #425 (ADR-39).

**Kind:** product capability.

**Owned paths:**

- `frontend/src/features/update-banner/index.ts`
- `frontend/src/features/update-banner/UpdateBanner.tsx`
- `frontend/src/features/update-banner/updateBanner.ts`

Feature tests:

- `frontend/src/features/update-banner/UpdateBanner.test.tsx`

**Public contract:** `frontend/src/features/update-banner/index.ts` exports
only `UpdateBanner`, a component with no props. Mounted once, it reads the
`update_check` field of `GET /api/settings` and shows nothing or one line in
the shell's notice strip: "Hatchdoor <version> is available", a "What's new"
link to the release page in a new tab and a "How to upgrade" link that opens
Help at `CONTEXTUAL_HELP.upgrade`. The browser remembers the last version it
dismissed under the `localStorage` key `hatchdoor_update_dismissed`; a later
version shows again. It borrows the shell's `.write-notice` styles and Help's
`.help-link`, so it needs no stylesheet of its own.

**Behaviour:** nothing shows when the check is off, found nothing newer, or
the request fails. Blocked storage still dismisses for the visit and forgets
it at the next load.

**Consumed dependencies:** the settings endpoint through `api/api.ts`'s
`apiFetch`, `lib/storage.ts`'s safe accessors, and `useHelp()` and
`CONTEXTUAL_HELP` from the Help reader.

**Coordination paths:** `App.tsx` (mounts it above the notice strip in
`VaultWorkspace` while `settingsEnabled`, so never in demo mode and never
before discovery has said whether this is a demo).

**Invariants:** never shown in demo mode; the link it opens in a new tab is
always the server-built GitHub release page.

**Validation:** `npx vitest run src/features/update-banner`, then full
frontend checks.

### First-run checklist

**Status:** Added by #419 (the #416 resolution, section C).

**Kind:** product capability.

**Owned paths:**

- `frontend/src/features/first-run/index.ts`
- `frontend/src/features/first-run/FirstRunChecklist.tsx`
- `frontend/src/features/first-run/firstRun.ts`
- `frontend/src/features/first-run/first-run.css`

Feature tests:

- `frontend/src/features/first-run/FirstRunChecklist.test.tsx`
- `frontend/src/features/first-run/firstRun.test.ts`
- `frontend/src/App.first-run.test.tsx`

**Public contract:** `frontend/src/features/first-run/index.ts` is the only
public entry point. `FirstRunChecklist` (props `vaults`, `onVaultCreated`,
`onAddGitVault`, `onOpenSearch`) is the checklist page. `shouldShowFirstRun`
decides whether it replaces the note pane's empty screen, `fetchFreshInstall`
reads `fresh_install` from `GET /api/v1/whats-new`, `useFirstRunState` returns
what this browser remembers, `reopenFirstRun` is Help's entry, and
`recordSearchResults(query, count)` is how a search that found something ticks
the last step. The browser remembers a dismissal under the `localStorage` key
`hatchdoor_first_run_dismissed` and the search that proved indexing under
`hatchdoor_first_run_search`. CSS is integrated through the `App.css`
stylesheet aggregation seam.

**Behaviour:** the page shows on a fresh install until closed, survives a
reload, and otherwise only after Help's "Setup checklist" entry reopens it for
the visit. Its four steps tick themselves from real state: a Vault exists
(step 1, adding one through Settings' `FolderPicker` and `createVault`, or
"Use a Git repository instead", which opens Add a Vault), the search model
(always done, chosen on the startup screen), an agent has connected while MCP
is on (step 3, polling `GET /api/settings` every 5 seconds while MCP is on and
no agent has connected), and a search in this browser found something (step
4). "Connect an agent" generates a password through
`POST /api/settings/mcp-token/generate` and sends one `PATCH /api/settings`
turning `HATCHDOOR_MCP_ENABLED` on and `HATCHDOOR_MCP_WRITE_ENABLED` off,
leaving any key the configuration file holds untouched, and refuses outright
when the configuration file holds MCP off or writes on; the password stays in
the component and is shown once, inside a ready-made config for Claude Code,
Codex, OpenClaw, Hermes or a generic client, addressed at
`HATCHDOOR_PUBLIC_URL` or the page's own origin plus `/mcp`. "Make a new
password" replaces it the same way. An optional row toggles
`HATCHDOOR_UPDATE_CHECK_ENABLED`, off by default. Storage failures never throw:
a dismissal then holds for the visit only.

**Consumed dependencies:** `api/api.ts`'s `apiFetch`, the settings HTTP
contract (the same requests Settings sends, unchanged), the What's new
endpoint's `fresh_install` (#424), the settings response's `last_agent`
(#426), Settings' public entry `features/settings/index.ts` (`FolderPicker`,
`createVault`, `baseSourceForKind`, `formatWhen`, `patchSettings` and
`generateMcpTokenCandidate`), the Vault collection client's
`fetchRegistryRevision`, `lib/storage.ts`'s safe accessors,
and the Help reader's `ContextualHelpLink` and `CONTEXTUAL_HELP`. It borrows
Settings' `.settings-btn`, `.settings-row`, `.settings-segmented`,
`.settings-notice`, `.settings-toggle` and `.folder-picker-*` styles and Help's
`.help-link`, so a change there reaches it.

**Coordination paths:** `App.tsx` (renders it on the `/` route in place of
the zero-Vault and empty states while `shouldShowFirstRun` holds and
`settingsEnabled`, reads `fresh_install`, records a search that found
something, and passes `onOpenSetupChecklist` to `HelpProvider`), `App.css`,
and the Help reader's `HelpProvider.tsx`/`HelpPanel.tsx` (the entry).

**Invariants:** never shown in demo mode and never shown on its own to an
upgraded install; the one-click connect never turns MCP writes on; the
password is never stored in the browser; a storage failure never breaks the
app.

**Validation:** `npx vitest run src/features/first-run
src/App.first-run.test.tsx src/features/settings src/features/help
src/App.startup-workspace-states.test.tsx`, then full frontend checks.

### Note reading and rendering

**Kind:** product capability.

**Owned paths:**

- `frontend/src/components/NotePage.tsx`
- `frontend/src/components/note-page/NotePreview.tsx`
- `frontend/src/components/note-page/PdfPreview.tsx`
- `frontend/src/components/note-page/RendererComponents.tsx`
- `frontend/src/components/note-page/SavedQueryBlock.tsx`
- `frontend/src/components/note-page/dom.ts`
- `frontend/src/components/note-page/markdownLinks.ts`
- `frontend/src/components/note-page/paragraphs.ts`
- `frontend/src/components/note-page/renderers.tsx`
- `frontend/src/components/note-page/savedQueries.ts`
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
target, because the same filename in notes at two depths can be two files; it
also sends Markdown note links (ADR-28, found by `markdownLinks.ts`, which
skips code, images and wikilinks) as `note_link_targets` and rewrites each
destination to the same note route, archived route or `/__missing__/` form a
wikilink gets, so `renderers.tsx` needs no branch for them. Local Markdown
images (`findMarkdownImages`) go out as `asset_targets` too, decoded, and a
resolved one is pointed at the asset route, so a root-anchored or bare-name
`![](path)`, the forms ADR-33's inserts can write, renders; an unresolved
image keeps its destination. `resolveAssetTargets` is the uncached form the
editor asks when choosing a `shortest` attachment path — heading/search-hit navigation, Markdown transformations,
note navigation/rendering behavior, the editable-block component map produced by
`createNoteMarkdownComponents`, the Vault-free `createManualMarkdownComponents`
(#417: the same callouts, code, tables and headings, a `base` block shown as
its source, no Vault endpoint, and links left to the caller's `renderLink`),
consumed by the Help reader, the paragraph marker `CalloutOrQuote` uses to
recognise its own first child, and the soft-break splitter that reconstructs one
source line per rendered line for the two unit types addressed per line.
A note link in a rendered body is a router navigation, not a browser one:
`createNoteMarkdownComponents` emits `Link` for any href on the note route
(`isNoteRoutePath` in `lib/notePath.ts` owns that grammar, shared with the
explorer's active-path highlighting) and for the archived-note branch, so
following one repaints the note pane alone instead of remounting the app and
rebuilding every Vault tree. Every other href keeps a bare anchor on purpose:
asset and PDF URLs under `/api`, in-page fragments, and external links, where
handing the click to the browser is what the click means. Following a note
link therefore no longer lets the browser resolve a `#heading` fragment, so
`NotePage` makes that jump itself, once per history entry, gated on the body
having settled onto the note the URL names and on the heading being on screen.
That last check runs on every commit rather than on a dependency list: the
order in which the note's fetch, its wikilink resolution and its render land
differs between a cold visit and a warm one, and a subset of them named as
deps makes the jump stop happening whenever the order shifts.

A fenced `base` block is a saved query (#275, ADR-21). `createNoteMarkdownComponents`
renders it as `SavedQueryBlock` (`note-page/SavedQueryBlock.tsx`), which draws
the table the server computed, inside the Table section's `.table-wrap`, with
the first `file.name` or `file.basename` cell (else the first cell) linking to
the row's note. `NotePage` fetches `GET .../notes/{slug}/saved-queries` through
`useSavedQueries` (`note-page/savedQueries.ts`) only while the editor is closed
(nothing renders the results while it is open) and only when the note holds a
`base` fence, again whenever its content hash changes or the collection revision
moves past the one its loaded results were evaluated at, and hands the results down through `SavedQueryProvider`. Each block finds its
result by its position among the note's `base` fences, cross-checked against
its source text, so two identical blocks keep their own outcomes. Refused,
stopped, empty, loading and failed states each render a distinct line inside
the frame, and ignored presentation instructions and the block's own marker
problems render as notes under it. Each column heading re-sorts that table
alone, in component state only: nothing is written or sent, and a reload
forgets it (#276). The note read is untouched: it still returns only the Markdown. The
editor preview has no provider and shows the definition as code, because it
renders unsaved text and only the file on disk is evaluated.
`remarkHideQueryMarkers` drops a `<!-- hatchdoor-query: name -->` marker that
names the `base` block after it from the syntax tree on the note page and in
the preview, and turns one with no block after it into a
`hatchdoor-orphaned-marker` element, which `OrphanedMarkerNotice` fills with the
server's `orphaned` notice where the marker sits. `useSavedQueries` also fetches
for a note holding only a marker, so that notice can arrive. Every other piece of raw
HTML renders exactly as before, and no other node moves, so line-addressed
inline editing is unaffected.

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
"Not Found". No Vault condition blocks a save before it is attempted (#372,
which removed #141's slot-driven write block): `SaveState` and the
`.write-notice` strip show `Not saving` only for a save the server actually
refused, from autosave's own status. `NotePage`'s `onTagSelect` prop is
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
any browser profile a demo instance happens to be served from. The
`lib/writeDrafts.ts` draft now covers the inline write surface too (#330),
not source mode alone: one debounced writer takes `handleInlineChange`,
`handleInProgressChange` (text living only inside an open block) and source
mode's `draftContent`, captures which note a scheduled write belongs to so a
pending one cannot follow the page onto the next note, and forces the write out
synchronously on `pagehide`, on `visibilitychange` to hidden, and on unmount —
the window a closing tab or a service-worker auto-reload falls into. Because
the inline editor has no open/close moment to read a draft at, recovery happens
when the note lands: a draft naming the hash now on disk is the interrupted
write, so it goes back into the body and is handed to autosave to finish once
inline editing is actually enabled (not on the commit the note arrives on,
where wikilink resolution has not settled and autosave would swallow it); one
naming an older hash is not replayed, and a notice points at source mode, which
already knows how to show a stale draft against the current version. A refused
draft write raises its own `write-notice`. The same issue closes the revision
effect's blind spot: `inlineDirty` is cleared only by a save landing, so
once autosave has stopped on a refused save, the effect's
"probably our own write, wait for quiet" skip never ended and the page ignored
every later revision for the session. When no write of ours can be in flight
the bump is someone else's, so it sets `noteChangedOnDisk` — flagged, with its
own reading-view notice, rather than refetched, because refetching is what
would replace the unsaved text. `NotePage`'s
`Vault` property row (`NoteProperties`'s `vaultName`, above) is a name only
— it carries no condition slot, so #152's demo-mode amber clamp has nothing
to touch there. `handleSave`'s catch and `handleBodyDrop`'s attachment-upload
catch both take the optional `onDemoRefusal` prop (#152, Note editing and
vault actions), checked first — `handleSave` falls back to its existing
`ConflictError`/generic-error branches on a miss, `handleBodyDrop` to its
existing generic `onWriteNotice` fallback.

**Consumed dependencies:** API/auth helpers, router state, Markdown/rendering
libraries, shared types/UI, note editing (including its held-draft recovery
model, #151), `app/vaultSlotLogic.ts`'s `noteInSyncConflict` (Application
shell and navigation, ADR-30), and `lib/storage.ts`'s `isEditableTarget` (Application
shell and navigation, #331), which `NotePage`'s document-level undo listener
uses to leave Ctrl/Cmd+Z and Y typed into inputs, textareas and
contenteditables outside the open block to the browser.

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
a guessed line. The note body never waits for the links read (#361): the note
page fetches the note and its links at once, drops the skeleton when the note
lands, and fills the links panel when its read settles, so a failed links read
hides only the panel.

**Validation:** note-page unit tests, `NotePage.test.tsx` (saving through every
Vault condition, read escalation), `NotePage.body-links.test.tsx` (in-body link routing and the
fragment jump), Markdown/heading/search/state tests,
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
- `frontend/src/lib/caretPoint.ts`
- `frontend/src/lib/editHistory.ts`
- `frontend/src/lib/imageUpload.ts`
- `frontend/src/lib/linePrefix.ts`
- `frontend/src/lib/sourceMap.ts`
- `frontend/src/lib/reloadGuard.ts`
- `frontend/src/lib/writeDrafts.ts`
- `frontend/src/lib/writePaths.ts`
- `frontend/src/components/note-page/BlockGap.tsx`
- `frontend/src/components/note-page/BlockInput.tsx`
- `frontend/src/components/note-page/EditableBlock.tsx`
- `frontend/src/components/note-page/InlineEditorProvider.tsx`
- `frontend/src/components/note-page/blockEditorSetup.ts`
- `frontend/src/components/note-page/editorFont.ts`
- `frontend/src/components/note-page/SaveState.tsx`
- `frontend/src/components/note-page/attachmentDrop.ts`
- `frontend/src/components/note-page/autocomplete.ts`
- `frontend/src/components/note-page/conflictDiff.ts`
- `frontend/src/components/note-page/frontmatter.ts`
- `frontend/src/components/note-page/inlineEditorContext.ts`
- `frontend/src/components/note-page/linkStyle.ts`

**Public contract:** write capability discovery and operations, editor/action
components, note-action/write-mode hooks, local draft behavior, client path
validation, upload normalization, frontmatter editing, conflict display,
note-link autocomplete and attachment inserts written in the Vault's link
style (`linkStyle.ts`, ADR-33: `[[Title]]`/`![[path]]` in a wikilink Vault,
`[Title](path.md)`/`![](path)` in a Markdown one, with paths encoded as the
rename rewriter encodes them), inline block editing (the editor
provider/context, the
per-block wrapper, the CodeMirror block input and its markdown syntax
highlighting, click-to-write in the space between blocks, structural block
operations, document-level undo, which ignores Ctrl/Cmd+Z and Y aimed at an
editable target outside `.block-input` (#331), autosave scheduling and save
state), line
mapping between rendered nodes and file lines, and attachment acceptance and
insertion. `lib/writeDrafts.ts`'s `HeldDraft`/`listHeldDrafts`/
`discardHeldDraft`/`collectLegacyHeldDrafts` (#151) are the recovery model
for drafts that predate Vault qualification, consumed by Settings'
`UnsavedDrafts.tsx`; ordinary per-note and create drafts
(`saveNoteDraft`/`loadNoteDraft`/`clearNoteDraft`/`saveCreateDraft`/
`loadCreateDraft`/`clearCreateDraft`/`pruneNoteDrafts`) keep their shape, with
one change: `saveNoteDraft` returns whether the write actually landed (#330).
`NotePage`'s debounced editor draft writer — the one behind the promise the UI
makes while the user types — raises a notice on a `false`; the reload-latest,
conflict-resolution and held-draft-restore call sites still discard it, so a
blocked store stays silent on those paths and surfacing it there is unfinished.
`collectLegacyHeldDrafts` reads every key before it writes any, the same
two-phase shape `pruneNoteDrafts` uses, because writing into a storage area
mid-enumeration can shift entries behind the `key(i)` cursor and skip drafts.
`api/writeApi.ts`'s `updateNote` takes an optional `{ keepalive }` (#330) for
the unload send, and `hooks/useNoteAutosave.ts` takes an optional `flushSave`
the `pagehide`/`visibilitychange` flush hands to `write` in place of the
ordinary sender: a fetch started while the document is being torn down is
cancelled with it. It is one send, not a side channel — the hook books its
outcome like any other save, so a tab that was only hidden comes back with a
current hash rather than conflicting on the next keystroke. That flush now
takes `pendingRef ?? queuedRef`, so an edit parked behind an in-flight save
leaves with the page too. `lib/reloadGuard.ts` is the seam that keeps the
service worker from reloading over all of this: `NotePage` holds it while an
edit is unsaved, a block is open, the source editor is open (#332), or a save
is in flight, and `main.tsx`
(coordination path) asks it before pulling an update and before acting on one
that has already activated.
`hooks/useNoteActions.ts`'s `openCreateDialog` takes an optional second
`targetVaultId` parameter (#151) so a caller outside the currently open note
— draft recovery — can pin which Vault a note is created in, overriding
`resolvePrimaryVaultId`'s inference for that one dialog session.

`lib/linePrefix.ts`'s `linePrefix` (#286) reads a line's whole invisible
leading run - its indentation, then any list marker, task box, heading hashes,
or quote arrows behind it - rather than only a marker and the indent ahead of
one. Indentation counts with no marker required, so a wrapped list item's
continuation line (addressed alone under D25a) reports the indent that has no
rendered counterpart. `caretMap.ts` consumes it directly; its former private
`invisiblePrefix`, which widened the answer for the caret only, is gone, and the
two no longer disagree on an indented heading or quote. `note-page/editorFont.ts`'s
`resolveFont` is the other half of making that hang land: `getComputedStyle().font`
serializes empty whenever a longhand cannot fold back into the shorthand, which
the heading fonts do through `font-variation-settings`, so the longhands are
composed instead. `BlockInput.tsx` hangs nothing for a `code block` unit, whose
leading spaces are partly rendered.

`hooks/useWriteMode.ts` fails closed in demo mode on the server's word
(#152): `GET .../write-capabilities` carries the same `demo_guard` layer every
mutation route does (`src/server.rs`'s route registration, grouped with
mutations "since it is write-capability discovery, not content browsing;
gated the same as the mutations it describes"), so the request 403s with
`demo_read_only` in demo mode and this hook's catch resolves `writeEnabled`
to `false`. It also re-derives rather than reading once per Vault (#339),
since a backend can restart into demo mode under an open tab: it takes the
collection's `demoMode` (true resolves `writeEnabled` to `false` in the same
render, no request needed) and `revision` (every collection revision re-asks
`write-capabilities`), and returns `recheck`, which `handleDemoRefusal`
calls. A first read for a Vault fails closed on any error; a re-read that
fails for any reason other than a demo refusal keeps the answer already held,
so a dropped connection mid-edit does not tear the editor down.
`writeApi.ts` exports `DEMO_READ_ONLY_CODE`/`isDemoReadOnlyError`, reading
the `code` every write error now carries (`parseError` returns `{message,
code}` rather than a bare string) so a demo refusal can be told apart from
every other write failure. `App.tsx`'s `handleDemoRefusal` is the
defense-in-depth backstop for a write that reaches the server anyway: one
app-authored sentence into the shared `.write-notice` strip (never the
server's own message, and never the generic inline failure state a note
action's dialog or the editor would otherwise show), plus a fresh
`loadVaults()` call and a `write-capabilities` recheck — "the app re-asks the
server what it is permitted to do" — and no retry affordance. It is threaded into `useNoteActions.ts`'s
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
`caretMap`, `caretPoint`, `editHistory`, `linePrefix`, `editorFont`,
`useNoteAutosave`,
`attachmentDrop`, `inlineEditing`, and `properties` tests;
`useNoteActions.test.tsx` (#152); plus `App.write-mode.test.tsx`,
`App.demo-mode.test.tsx` (#152), and full frontend checks.

### Graph

**Kind:** product capability; suitable bounded dry-run candidate.

**Owned paths:**

- `frontend/src/components/graph/GraphPage.tsx`
- `frontend/src/components/graph/graphSimulation.ts`
- `frontend/src/styles/graph.css`

**Public contract:** `GraphPage`, graph simulation helpers (including the
island layout primitives `computeIslandCenters`, `buildIslandGraphs`, and
`createIslandSimulation` — #143; the screen-space hit test, wheel-delta
normaliser, viewport cull, budgeted label placement, island caption sizing
and count line, and island-field fit — #337), and the
`/api/v1/vaults/{scope}/graph` payload. Under `all` with more than one participating Vault, every Vault's
component is laid out on its own and placed as a labelled, dash-enclosed
island on one shared canvas (one zoom, one pan); at zero or one participating
component — including a single-enabled-Vault instance under `all` — the page
is byte-identical to the narrowed single-Vault graph (#118's resolution).

**Consumed dependencies:** shared API/error/types/UI, router navigation,
`d3-force`, `useVaultDiscovery` (Vault-management order, `demoMode`, and
per-Vault condition, reused via `deriveVaultSlot(vault, count, demoMode)` for
each island's caption so a demo instance's islands clamp to the amber tier
the same as every other Vault chrome, #152), the collection client's
revision (the graph re-reads on every move and folds the answer into the live
simulation without resetting the view, #336), and `describeVaultsNotDrawn` /
`joinWithAnd` from `lib/vaultParticipants.ts` (the graph's own "still being
indexed" empty-field sentence stays in `GraphPage.tsx`).

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
unscoped `/api/stats` this section previously cited was retired in #101). The
"Notes created" chart draws `activity_by_month`'s `created_count` in the order
supplied and averages over the six-month window rather than over the entries
received (#298). It shows a one-line notice when `created_date_status` is
`estimated` or `reading`, and asks again for a Vault still `reading` until it
is not (#300).

**Consumed dependencies:** shared API/error/types/UI and router links.

**Coordination paths:** `App.tsx`, `types.ts`, backend stats wire types/handler,
and responsive CSS.

**Validation:** add focused component coverage for behavioral changes, affected
route tests, and full frontend checks.

### Settings

**Kind:** product capability/adapter.

**Owned paths:**

- `frontend/src/features/settings/index.ts`
- `frontend/src/features/settings/settingsApi.ts`
- `frontend/src/features/settings/SettingsPage.tsx`
- `frontend/src/features/settings/VaultSettingsIndex.tsx`
- `frontend/src/features/settings/VaultSettingsIndex.test.tsx`
- `frontend/src/features/settings/vaultGitBehavior.ts`
- `frontend/src/features/settings/VaultCreation.tsx`
- `frontend/src/features/settings/VaultCreation.test.tsx`
- `frontend/src/features/settings/vaultCreation.ts`
- `frontend/src/features/settings/FolderPicker.tsx`
- `frontend/src/features/settings/FolderPicker.test.tsx`
- `frontend/src/features/settings/UnsavedDrafts.tsx`
- `frontend/src/features/settings/UnsavedDrafts.test.tsx`
- `frontend/src/features/settings/relativeTime.ts`
- `frontend/src/features/settings/SettingsModal.tsx`
- `frontend/src/features/settings/settings.css`
- `frontend/src/features/settings/SettingsPage.test.tsx`

**Public contract:** `frontend/src/features/settings/index.ts` is what other
features may import (#419): `FolderPicker`, `formatWhen`, `createVault`,
`baseSourceForKind`, and `settingsApi.ts`'s `patchSettings` and
`generateMcpTokenCandidate`, the one definition of the `PATCH /api/settings`
and `POST /api/settings/mcp-token/generate` requests that the Settings page and
the First-run checklist both send. The shell still imports `SettingsPage`
directly. The Settings page presents a two-level Vault-management index
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
outstanding AC4: the local-edit-to-commit trigger is not a configurable
debounce and does not belong to this field, which answers a different
question, how often to poll a remote for incoming changes. #267 gave that
trigger its successor without a setting: `vault_watcher.rs`'s fixed
non-configurable debounce now asks for a commit turn as well as an index
turn, so a Vault commits shortly after the writing stops. A live Git console
(shown whenever the Vault's own `git` status is not `"disabled"`) carries a
`Sync now`/`Commit now`/`Try again` button calling `POST .../sync` or
`.../retry`. Which of the first two it offers, and whether the healthy
sentence names a remote at all, comes from the Vault's `capabilities.sync`
flag rather than from its Git mode string. That flag is definition-derived,
so a failing Vault keeps its own label (#267). It
renders one of nine failure sentences off `git_error.code` (plus an
unrecognised-code fallback) — the two carrying an affected-file list
(`managed_git_dirty_working_copy`, `managed_git_conflict`) render it from
`git_error.detail`'s `affected_paths` data, not from the message string.
While the Vault's `capabilities.publish_recovery` is true, the console also
renders `RecoveryBranchPanel` (ADR-30): the recovery branch name
(`recoveryBranchName`) with a copy control, a host link for an HTTPS remote
(`recoveryBranchUrl`), the last published commit, and
`describeRecoveryFailure`'s sentence for a refused publish, with a button
calling `POST .../recovery-branch`.
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
complete response, and shows no instance-wide status console: #183 retired
the **Search index** and **Versioning** consoles, their two-second polling of
`/api/index-status` and `/api/git-status`, and the local-Git-initialisation
and remote-downgrade confirmations that went with them. Each Vault's own
settings page is where that information lives now.

When `GET /api/v1/vaults` reports `recovery` (the registry file itself is
unreadable, #150), `VaultSettingsIndex.tsx` replaces its whole `Vaults`
group with the same documented error block `App.tsx`'s note-pane shows,
omitting `Add a Vault`; `This server` is a separate group and keeps working.

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

`SettingsModal.tsx` (#338) is the one shell all three Settings modals render
through (Vault creation, the identity-change confirmation, the reindex
confirmation): it moves focus into the dialog on open, keeps Tab inside it,
closes on Escape (held while the dialog's own Cancel is disabled), and
returns focus to the opener. It follows `NoteActionsDialog.tsx`'s focus-in,
Tab-wrap and Escape behaviour and adds what that dialog does not: focus
return to the opener, pulling stray focus back inside, and focusing the
dialog itself when it holds no focusable control. While Help is open beside
it (#430, from the folder picker's link), its backdrop gives up Help's width,
Escape closes Help before the dialog, and Tab is not held inside, the What's
new dialog's rules. A Vault's own page (#338) reads the registry revision
fresh at the click for Pause, Resume and Disconnect, which carry no form
fields; a Save and the identity round trip's pause step are checked against
the revision the form was based on, and a `registry_revision_conflict`
re-reads it and asks for the Save again in plain words (an alert, never the
server's diagnostic string) rather than re-sending the stale number. A Git
behaviour switch returns whatever it takes off the screen (typed token,
sign-in choice, schedule, and for No Git the repository fields) to its saved
value, and a behaviour without a remote always sends `https_credentials`
`remove`. Each instance section's Save and Discard touch only that
section's drafts. The unsaved-drafts destination picker disables, and names
the reason for, every Vault whose `capabilities.mutate` is false or that is
paused or unavailable.

Out of this page's scope: giving a Vault a source it did not start with (its
first repository, i.e. a Local Vault becoming `managed_git`, or a bare
first-run Vault) is the separate first-run flow (#122), not a field this page
edits. `docs/design/design-system.html` is not documented against this
ticket's primitives — on this branch it predates even #120's Settings work
and has diverged from `development`'s own (also incomplete) copy; treated as
separate, pre-existing design-system documentation debt rather than in scope
here.

**Consumed dependencies:** authenticated `apiFetch`, the settings HTTP
contract, the folder listing `GET /api/v1/folders` (for `FolderPicker.tsx`,
#430), (for `UnsavedDrafts.tsx`) `lib/writeDrafts.ts`'s held-draft
functions, and the Help reader's `useHelp` (for `SettingsModal.tsx`), `ContextualHelpLink`, `CONTEXTUAL_HELP`,
`vaultConditionHelp` and `gitConsoleHelp` (#423).

**Coordination paths:** `frontend/src/App.tsx` (route; also supplies
`vaults` and `onOpenCreateDraft` to `SettingsPage`, and seeds the standalone
create draft before opening the dialog), `frontend/src/app/ExplorerPane.tsx`
(normal-deployment navigation), `frontend/src/App.css` (stylesheet
aggregation), `frontend/src/components/NotePage.tsx` (`?restoreEdit=1`
handling and the held-drafts notice), `frontend/src/hooks/useNoteActions.ts`
(`openCreateDialog`'s target-Vault override), `frontend/src/main.tsx` (runs
the one-time legacy sweep and browser-state cleanup before rendering),
`src/server.rs` (SPA/API routes), `src/handlers/settings.rs` (settings wire
producer), and `frontend/src/types.ts`
(`VaultSource`/`VaultGitMode`, mirroring `src/vault_registry.rs`'s
same-named types, `VaultSummary`'s `source` field, now typed rather than
`unknown`, and the `FolderListing` types mirroring `src/folder_listing.rs`
for the folder picker; consumed by this section and by
`frontend/src/app/vaultSlotLogic.ts`, already listed under Vault chrome's
own `types.ts` coordination entry).

The **Agent access (MCP)** section shows the settings response's read-only
`last_agent` (`LastAgentConnection` in `frontend/src/types.ts`) as "<name>
connected <relative time>" through `formatWhen`, or "No agent has connected
yet" (#426). The **Updates** section holds the
`HATCHDOOR_UPDATE_CHECK_ENABLED` switch, whose help sentence states exactly
what the daily request sends (#425); the banner itself is the Update banner's.

Add a Vault's local-folder choice (#430) defaults to `FolderPicker.tsx`, a
flat list of the Vault mount with drill-in and a breadcrumb, which reports the
picked folder's absolute path (`root` joined with its relative `path`); it
takes `value` and `onPick` only, so it renders outside the dialog too; the
First-run checklist's step 1 is its other consumer (#419), with
`vaultCreation.ts`'s `createVault` and `baseSourceForKind`.
**Type a path instead** swaps in the unchanged Folder path field; both edit
the same path draft, and the create request is the same `POST /api/v1/vaults`.
Its "My folder isn't here" and empty-mount Help links read
`CONTEXTUAL_HELP.folderOutsideMount`.

**Invariants:** demo mode exposes no Settings navigation or endpoints;
environment-managed and permanently unavailable values are records rather than
disabled form controls; secret values are never rendered from the settings
document; a held draft is deleted only through an explicit Restore or
Discard, never aged out.

**Validation:** `SettingsPage.test.tsx`, `VaultSettingsIndex.test.tsx`,
`VaultCreation.test.tsx`, `FolderPicker.test.tsx`, `UnsavedDrafts.test.tsx`, affected shell tests
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
and wherever an exact read fails outright. Its optional `help` node (#423)
renders on its own line under the description, for the start states' "How
does this work?" links.

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
Explorer (`ChangesPanel`, and `useVaultTree` for the tree read's missing
Vaults), Search (`SearchDialog`), and the Application shell's
`app/ExplorerPane.tsx` (the tree's trailing line, the accordion's per-Vault
"did not answer" line and the empty-tree error block, #334), and Graph
(`GraphPage`: `describeVaultsNotDrawn` for Vaults that drew no island, #143,
and `joinWithAnd` for its empty-field sentence, #336). The tree is
the third surface, after the Changed on disk list and search results, whose
`partial` read names the Vaults that did not answer.

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

Test files run with `isolate: false` (#351): each worker keeps one jsdom and
one module cache across files. `setup.ts` runs before every file and puts
back what a fresh environment would give it: it resets the module registry,
drops stylesheets earlier files injected, clears `<html>` and `<body>`
attributes, the body's children and `localStorage`, and unmounts Testing
Library renders after every test. A test that overrides anything else on
`window`, `document`, `navigator`, a prototype or a global must restore it in
its own `afterEach`. Validate a change here with
`npx vitest run --sequence.shuffle` five times in a row.

## Auxiliary repository paths

These paths are outside the runtime module catalog and require separate work
packet scope:

- `Dockerfile` and `docker-compose.yml`: packaging/deployment. The Dockerfile's
  default target produces the rootless runtime image; `verification` runs the
  default-feature locked Rust suite. BuildKit Cargo cache mounts and optional
  Cargo build controls are documented in `docs/development/container-builds.md`.
  Consumers are local Docker builders and external CI; no provider-specific
  configuration belongs in this contract. `docker-compose.yml` sets
  `stop_grace_period` above the Git connect plus transfer bounds in
  `git/mod.rs`, so shutdown can wait out a wedged sync turn instead of being
  killed by Docker's default 10 s grace (#322); raise it if those bounds grow.
  Validate cold/warm verification,
  source/dependency invalidation, and the final image's platform/healthcheck.
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

```bash
just check
```

It runs formatting, clippy for the default and the all-features build, the
backend tests with `--features eval`, and the frontend format, lint, typecheck,
test and build steps. `CONTRIBUTING.md` lists the exact commands. Run
`npm ci` in `frontend/` first on a fresh checkout.

`just check-full` adds the backend tests in the default configuration and with
`--all-features`, which loads real model weights. Run it for changes to
Embeddings, Reranking, model identities, or inference dependencies.

Use focused tests during development. Run the full gates before merging a
boundary or interface change.
