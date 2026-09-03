# Architecture Decision Records

This directory records the significant architecture decisions behind Hatchdoor —
what was decided, why, and what it commits us to. Read it before proposing a
structural change: each record is a **constraint your PR is expected to respect**.
If your change would break one, that's fine — but say so, and propose an amendment
(see below) rather than quietly working around it.

Most decisions are captured as short sections in this file. One decision with a
full evaluation behind it lives in its own file and is linked from the index.

## How to change a decision

These records are **append-only**. Don't rewrite an accepted decision to reflect
a new one — instead:

1. Add a new record (next number) describing the new decision.
2. Set the old record's status to `Superseded by ADR-NNNN` and leave its text intact.

This keeps the *history* of why the code is shaped the way it is, which is the
whole point.

## Template

Copy this for a new record:

```markdown
## ADR-NNNN — Short title

- **Status:** Proposed | Accepted | Superseded by ADR-XXXX
- **Context:** The problem and the forces at play (constraints, requirements).
- **Decision:** What we decided to do.
- **Consequences:** What this makes easier or harder; the tradeoff accepted.
- **Evidence:** Where this shows up in the code/docs.
```

## Index

| # | Decision | Status | PR constraint (what not to break) |
|---|---|---|---|
| 01 | [Markdown is the source of truth; SQLite is a disposable read model](#adr-01--markdown-is-the-source-of-truth-sqlite-is-a-disposable-read-model) | Accepted | Never make the DB authoritative; it must rebuild from `.md` |
| 02 | [One binary serving three surfaces over one shared core](#adr-02--one-binary-serving-three-surfaces-over-one-shared-core) | Accepted | Don't split into services or fork the domain core per surface |
| 03 | [Web and MCP writes share one `vault/write/` layer](#adr-03--web-and-mcp-writes-share-one-vaultwrite-layer) | Accepted | Don't implement writes in a handler or MCP tool directly |
| 04 | [Local, CPU-only embeddings; no external inference API](#adr-04--local-cpu-only-embeddings-no-external-inference-api) | Superseded by ADR-16 | — (see ADR-16) |
| 05 | [Pure semantic retrieval by default; rerank and hybrid stay offline](./semantic-search-strategy.md) | Accepted | Don't add reranking or FTS/vector fusion to the runtime search path |
| 06 | [Embedded SQLite: FTS5 + sqlite-vec, WAL, pooled reads](#adr-06--embedded-sqlite-fts5--sqlite-vec-wal-pooled-reads) | Accepted | Don't add an external DB or an ORM; keep reads non-blocking during reindex |
| 07 | [Fail-fast security posture at startup](#adr-07--fail-fast-security-posture-at-startup) | Accepted | Don't downgrade unsafe configs from refuse-to-start to a warning |
| 08 | [Bearer-token auth with a query-param fallback](#adr-08--bearer-token-auth-with-a-query-param-fallback) | Accepted | Keep token compares constant-time; don't leak tokens to logs |
| 09 | [MCP off by default, own token, Origin allowlist](#adr-09--mcp-off-by-default-own-token-origin-allowlist) | Accepted | Read-only MCP still requires a token; keep the anti-rebinding checks |
| 10 | [Optional git sync as a debounced background task](#adr-10--optional-git-sync-as-a-debounced-background-task) | Superseded by ADR-18 (mechanism only) | Never force-checkout over uncommitted manual vault edits (restated in ADR-18) |
| 11 | [Soft delete (trash) and archive-by-move](#adr-11--soft-delete-trash-and-archive-by-move) | Accepted | Don't hard-delete note or asset files |
| 12 | [Distroless, rootless, multi-stage container](#adr-12--distroless-rootless-multi-stage-container) | Accepted | Don't assume a shell at runtime; keep the image rootless |
| 13 | [Deliberate minimalism: trait seams only where they pay](#adr-13--deliberate-minimalism-trait-seams-only-where-they-pay) | Accepted | Don't add speculative abstractions, state libraries, or frameworks |
| 14 | [No deployment step requires a browser](#adr-14--no-deployment-step-requires-a-browser) | Accepted | Don't add a setting the UI can set but the environment can't, or a startup gate only a browser can clear |
| 15 | [Search quality is a product feature, not a tunable](#adr-15--search-quality-is-a-product-feature-not-a-tunable) | Accepted | Don't change the retrieval path without eval numbers (one narrow exemption: ADR-20); don't trade recall or MRR for speed |
| 16 | [EmbeddingGemma by default behind a licence gate; representation locked at 800/50 with context](#adr-16--embeddinggemma-by-default-behind-a-licence-gate-representation-locked-at-80050-with-context) | Accepted | Don't change the embedder, chunk size, or contextual headers without re-running the eval |
| 17 | [RMCP is the MCP protocol boundary; supported revisions narrow to 2026-07-28 and 2025-11-25](#adr-17--rmcp-is-the-mcp-protocol-boundary-supported-revisions-narrow-to-2026-07-28-and-2025-11-25) | Accepted | Don't hand-implement MCP wire behavior or re-widen the advertised revision set |
| 18 | [Per-Vault Git turns supersede the single-Vault debounced sync task](#adr-18--per-vault-git-turns-supersede-the-single-vault-debounced-sync-task) | Accepted | Don't reintroduce an instance-wide sync task or a second execution lane; a Git turn holds its Vault's mutation lock and never force-checks out over manual edits |
| 19 | [Vault-qualified cores are the only seam an adapter crosses](#adr-19--vault-qualified-cores-are-the-only-seam-an-adapter-crosses) | Accepted | Don't call a handler from an MCP tool or reach past a core from any adapter; orchestration lives in the core, mapping in the adapter |
| 20 | [Deleting provably dead retrieval code needs a proof, not an eval run](#adr-20--deleting-provably-dead-retrieval-code-needs-a-proof-not-an-eval-run) | Accepted | Don't claim this exemption for code that runs; the proof enumerates every producer, and deletion-only means deletion-only |

> Records 01–13 were reconstructed and adopted on 2026-07-19 from the codebase,
> the CHANGELOG audit fixes (`F-01`…`F-17`), and the semantic-search evaluation.
> Records 14–16 were added on 2026-08-08. Record 17 was added on 2026-08-24.
> Records 18–19 were added on 2026-08-28 from the architecture review recorded on #162.
> Record 20 was added on 2026-08-31 from the review of #211.

---

## ADR-01 — Markdown is the source of truth; SQLite is a disposable read model

- **Status:** Accepted
- **Context:** Users own an Obsidian-style vault of `.md` files and must never be locked into Hatchdoor's storage.
- **Decision:** The `.md` files under `VAULT_PATH` are authoritative. SQLite is a generated cache built from them and safe to delete; it is rebuilt from the vault on demand. The schema is versioned and the embedder identity is stamped — a mismatch wipes and rebuilds rather than serving stale or mixed-model data.
- **Consequences:** The cache can live outside the vault and be discarded freely; schema or embedding-model changes cost a full reindex, accepted as the price of never corrupting the read model.
- **Evidence:** README "Data and Safety Model"; `cache/vault_snapshots.rs` (`replace_vault_snapshot`, the per-Vault rebuild that replaced `app_state.rs`'s `build_cache` in #185); `cache/schema.rs` (`SCHEMA_VERSION`, `reset_if_embedder_changed`).

## ADR-02 — One binary serving three surfaces over one shared core

- **Status:** Accepted
- **Context:** Hatchdoor must serve a browser UI, AI agents, and static assets, on modest self-hosted hardware, with a simple deploy story.
- **Decision:** A single `axum` binary exposes three surfaces — a JSON HTTP API, an MCP server at `/mcp`, and the built React SPA as static files — over one shared domain core and one `AppState`.
- **Consequences:** One container, one process, shared state; the cost is a large composition root (`server.rs`) that concentrates routing and startup logic.
- **Evidence:** `server.rs` router construction; `app_state.rs`.

## ADR-03 — Web and MCP writes share one `vault/write/` layer

- **Status:** Accepted
- **Context:** Both the browser and agents can mutate the vault. Divergent write paths would mean divergent safety guarantees — the class of bug that loses user data.
- **Decision:** All mutations go through the atomic primitives in `vault/write/`. The HTTP write API and the MCP write tools are thin adapters over the same functions. Writes use a content-hash (`expected_content_hash`) for optimistic concurrency.
- **Consequences:** One place to audit for data-loss safety; both surfaces get path-safety, link-rewriting, and atomic rename for free. Adapters must not reach past this layer to touch the filesystem.
- **Evidence:** `vault/write/{notes,attachments,fs_ops,rewrites}.rs`; `handlers/write_api.rs`; `mcp/tools/write.rs`.

## ADR-04 — Local, CPU-only embeddings; no external inference API

- **Status:** Superseded by ADR-16
- **Context:** Hatchdoor targets private, self-hosted deployment on modest hardware. A dependency on a paid or remote inference API would break privacy and offline operation.
- **Decision:** Embeddings run locally via fastembed (Nomic Embed Text v1.5, 384-dim). Model weights are prefetched at image-build time so first run is offline.
- **Consequences:** Fully private and offline; embedding is CPU-bound, which directly constrains the search-strategy decision (ADR-05).
- **Evidence:** `embed/fastembed_embedder.rs`; `main.rs --prefetch-embedder`; Dockerfile prefetch stage.

## ADR-06 — Embedded SQLite: FTS5 + sqlite-vec, WAL, pooled reads

- **Status:** Accepted
- **Context:** The read model needs keyword search, vector search, links, and stats, with zero external infrastructure and good concurrent-read behavior during reindex.
- **Decision:** Bundle SQLite (no external database, no ORM). Use FTS5 for keyword search and `sqlite-vec` for vectors. Build in memory; run in WAL mode with a pool of read connections. Reindex commits in one transaction so readers keep serving the prior snapshot.
- **Consequences:** No infra to run; hand-written SQL instead of an ORM; reads stay parallel and non-blocking during a refresh (fixes F-03/F-05).
- **Evidence:** `Cargo.toml` (`rusqlite` bundled, `sqlite-vec`); `cache/schema.rs`; `cache/queries/`; CHANGELOG F-03, F-05.

## ADR-07 — Fail-fast security posture at startup

- **Status:** Accepted
- **Context:** A self-hosted app is easy to misconfigure into exposing a private vault (binding `0.0.0.0` without auth; enabling a public demo alongside write surfaces).
- **Decision:** Refuse to start in unsafe configurations rather than warn: no non-loopback bind without `HATCHDOOR_WEB_BEARER_TOKEN`; demo mode is incompatible with MCP or git sync and refuses to boot together.
- **Consequences:** Misconfiguration fails loudly at startup instead of silently exposing data; operators must set a token before going public.
- **Evidence:** `server.rs` (`check_web_auth_posture`, `check_demo_mode_posture`, `check_demo_mode_registry_posture`); CHANGELOG F-01.

## ADR-08 — Bearer-token auth with a query-param fallback

- **Status:** Accepted
- **Context:** When set, a bearer token must protect `/api/*`, `/vault-assets/*`, and downloads. But images, SSE streams, and download links can't carry an `Authorization` header from the browser.
- **Decision:** Accept the token via `Authorization: Bearer` or, where headers can't be set, an `access_token` query parameter. Compare tokens in constant time; redact them from logs. The PWA prompts for the token on a 401 and stores it locally.
- **Consequences:** Works with header-less browser contexts; the query-param path is a deliberate, documented tradeoff (tokens can appear in URLs/proxy logs).
- **Evidence:** `config.rs` (`web_bearer_token`); `auth.rs` (`constant_time_eq`, redaction); CHANGELOG F-01, F-06.

## ADR-09 — MCP off by default, own token, Origin allowlist

- **Status:** Accepted
- **Context:** MCP exposes the whole vault to agents and is mounted at `/mcp`, outside the web-auth layer. Browser-originated JSON-RPC also invites DNS-rebinding attacks.
- **Decision:** MCP is disabled unless explicitly enabled. Even read-only MCP requires its own bearer token. Requests are checked against an Origin allowlist (localhost variants only) to block DNS rebinding. The Streamable-HTTP protocol is implemented directly.
- **Consequences:** Enabling agent access is a conscious, credentialed step; the vault is never exposed by simply turning MCP on.
- **Evidence:** `mcp/config.rs` (`validate`, `validate_mcp_request`, Origin matching); `mcp/tools/{read,write}.rs`.

## ADR-10 — Optional git sync as a debounced background task

- **Status:** Superseded by ADR-18 for the mechanism (the instance-wide debounced task over `VAULT_PATH`); the safety decisions below (opt-in, never force-checkout over uncommitted manual edits, local mode never contacts a remote, writes do not block on sync) remain in force and are restated there for the per-Vault Git turn.
- **Context:** Users want their vault edits versioned and pushed, without blocking writes or fighting manual git usage.
- **Decision:** Optional (off by default) auto commit-and-push via vendored `git2`, on a debounced background loop with conflict-abort semantics. It refuses to force-checkout over uncommitted manual edits to tracked files, surfacing them as an error. The watcher ignores `.git/` so sync churn doesn't trigger reindexing.
- **Consequences:** Writes stay fast (sync is async); manual edits are never silently discarded; sync is opt-in.
- **Evidence:** `git/{sync,task,message}.rs`; CHANGELOG v2.1.0, F-08.

## ADR-11 — Soft delete (trash) and archive-by-move

- **Status:** Accepted
- **Context:** Agents and the UI can delete or archive notes. Destructive deletes on user data are unacceptable.
- **Decision:** Delete moves notes, and the referenced assets that live inside the note's own folder, into `.hatchdoor-trash` (excluded from indexing). A referenced asset kept elsewhere stays where it is. Archive moves notes under `HATCHDOOR_ARCHIVE_PREFIX` (default `90-archive/`), which also drives archived-link styling.
- **Consequences:** Deletes are recoverable; nothing is unlinked from disk by Hatchdoor. The trash folder is skipped when deciding whether a vault is empty.
- **Evidence:** README "Data and Safety Model"; `vault/write/`; `config.rs` (`archive_prefix`); CHANGELOG F-12. Asset travel was narrowed to the note's own folder in #225 (`vault/write/assets.rs`, `asset_move_plan`).

## ADR-12 — Distroless, rootless, multi-stage container

- **Status:** Accepted
- **Context:** The shipped image should have minimal attack surface and run without root.
- **Decision:** Runtime is `distroless/cc` as `nonroot`. The build is multi-stage (cargo-chef dependency cache → Rust build → separate frontend build → slim runtime). Because the runtime has no shell, the health probe is a `--healthcheck` subcommand built into the binary.
- **Consequences:** Small, rootless, shell-less image; anything needing a shell (like a curl-based healthcheck) had to move into the binary.
- **Evidence:** `Dockerfile`; `main.rs` (`run_healthcheck`); CHANGELOG F-17.

## ADR-13 — Deliberate minimalism: trait seams only where they pay

- **Status:** Accepted
- **Context:** It's tempting to add abstractions "for flexibility" — traits, a state-management library, an ORM, a CSS framework.
- **Decision:** Introduce abstraction only where it earns its keep. The only trait seams over external ML dependencies are `Embedder` and `Reranker`, exactly where test doubles (`StubEmbedder`) and the `embedder-tests` feature gate need them. No Cargo workspace, no ORM, no frontend state library, no CSS framework.
- **Consequences:** A lean, navigable dependency set; new code should follow suit rather than introduce speculative layers.
- **Evidence:** `embed/embedder.rs`, `rerank/reranker.rs`; `Cargo.toml` / `frontend/package.json` dependency sets.

## ADR-14 — No deployment step requires a browser

- **Status:** Accepted
- **Context:** A Hatchdoor deployment must be completable end to end by a script, a compose file, or an agent, with no human at a browser to finish the boot. That pulls against the settings page: once a knob is editable in the UI, the pull is to make the UI its home and drop the environment variable behind it.
- **Decision:** No step required to bring Hatchdoor to a working state may be reachable only through the browser. For operator settings the mechanism is an environment variable, and that variable is authoritative for the process — the durable `settings.json` overlay resolves beneath the environment pins and above the defaults, and the UI renders a pinned value as locked with its source rather than silently discarding the edit. A setting may be added to the UI, but never *only* to the UI. For anything that is not a setting, the mechanism is an API or MCP path instead. Accepting the Gemma licence is the standing example: it is a deliberate act by a person or their delegated agent, not a config default to be copy-pasted out of someone else's compose file, so it deliberately has no environment variable — but `accept_gemma_terms` / `decline_gemma_terms` over MCP let an agent complete it unattended, and the versioned on-disk receipt persists it across restarts.
- **Consequences:** A first boot from `.env` alone — plus, where a licence is involved, one agent call — is always a complete deployment, and a vault can be redeployed from scratch with no manual step. The cost is that every new setting is three things (a key in `live_settings_defaults`, a UI row, and the locked/source display), every new startup gate must ship a machine path alongside its UI, and a setting that cannot be expressed as a string in an env var does not get added.
- **Evidence:** `runtime_config.rs` (`live_settings_defaults`, `SettingSource`, `ResolvedSetting::pinned`); `handlers/settings.rs` (`SETTINGS`, `locked`, `source`); `model_setup.rs` (`accept_gemma`, `acceptance_is_current`); `mcp/tools/`; `.env.example`; README operator configuration contract.

## ADR-15 — Search quality is a product feature, not a tunable

- **Status:** Accepted
- **Context:** Retrieval quality is the reason to run Hatchdoor at all. It is also the easiest thing to erode by accident — a cheaper embedding model, a chunking tweak, a filter added for speed, a smaller candidate count. Each looks locally reasonable, and none of them announce themselves as a regression.
- **Decision:** No change may knowingly degrade retrieval quality. Any change touching the retrieval path — embedding model, chunk size or overlap, contextual document headers, task prefixes, layer selection, candidate counts, pre- or post-filters — is validated against the eval set in `eval/` before merge, on the same metrics the harness already reports (Recall@5/10, MRR, FP-rate@5, correct-heading), with the per-category and per-tier breakdown read, not just the aggregate. Where quality and another axis conflict, quality wins: prefer paying latency, memory, disk, or index time over recall or ranking. A regression ships only with an explicit recorded justification and its measured cost.
- **Consequences:** Retrieval changes cost an eval run and usually a full reindex, so they move slower than other work. Optimisations on the search path must prove neutrality rather than assume it. Swapping the embedder is a deliberate, evaluated decision (ADR-16), never a convenience.
- **Evidence:** `eval/` harness, `eval/queries.jsonl`, `eval/results.md`; `docs/research/embeddings/`; `cache/schema.rs` (embedder-identity stamp forcing a rebuild on model change).

## ADR-16 — EmbeddingGemma by default behind a licence gate; representation locked at 800/50 with context

- **Status:** Accepted (supersedes ADR-04)
- **Context:** ADR-04 fixed the principle — local, CPU-only embeddings, no external inference API — but named Nomic Embed Text v1.5 at 384 dimensions with weights baked into the image. The 2026-07 embedding sweep replaced both halves of that. It ran a 24-cell grid over models, chunk settings, and contextual documents against an eval set grown from 26 queries to ~125, tiered and categorised. Separately, shipping Gemma weights in a public image is not something the licence permits us to do on the user's behalf.
- **Decision:** The principle of ADR-04 stands unchanged: embeddings run locally on CPU via fastembed, with no remote inference. What it resolves to now is EmbeddingGemma 300M Q4 at 768 dimensions — multilingual, with its own retrieval query/document prefixes — as the default, and Nomic Embed Text v1.5 as the fallback for a user who declines the Gemma terms, explicitly labelled English-only and lower quality for multilingual vaults. Public images ship neither model; weights are fetched on first run behind the licence gate (ADR-14). The document representation is locked at 800-token chunks, 50-token overlap, and contextual headers on: context off wins raw recall, but context on wins MRR in 11 of 12 like-for-like comparisons and correct-heading accuracy in all 12, and Hatchdoor routes people into a specific note section, so first-result quality and heading accuracy are worth more than the broader recall. ADR-05 was re-validated against this newer evidence and is unchanged — retrieval stays pure semantic, with reranking and hybrid fusion offline evaluation tools only.
- **Consequences:** First run is no longer offline: it needs a network fetch and a licence decision, which is why that decision has a machine path. The cache carries the embedder identity, so switching models wipes and rebuilds. 450/50 with context on remains the specialist setting for heading navigation and is not the default. Any future move off these values is an ADR-15 change and needs eval numbers.
- **Evidence:** `embed/fastembed_embedder.rs` (`embedding_gemma_300m_q4`, `DocumentFormat::GemmaRetrievalV1`); `model_setup.rs`; `chunk/chunker.rs` (`max_tokens: 800`, `overlap_tokens: 50`); `docs/research/embeddings/embedding-sweep-decisions-2026-07-26.md`; `eval/results.md`; CHANGELOG v2.4.0.

## ADR-17 — RMCP is the MCP protocol boundary; supported revisions narrow to 2026-07-28 and 2025-11-25

- **Status:** Accepted (supersedes the single sentence of ADR-09 that reads "The Streamable-HTTP protocol is implemented directly"; every other ADR-09 decision — MCP off by default, its own bearer token even for read-only access, and the Origin allowlist against DNS rebinding — remains fully in force)
- **Context:** Hatchdoor's `/mcp` surface is a hand-written, POST-only Streamable HTTP adapter under `src/mcp/`, serving exactly three legacy revisions (`2025-03-26`, `2025-06-18`, `2025-11-25`). The MCP specification's `2026-07-28` revision adds stateless discovery-based requests, per-request protocol metadata with required HTTP headers, subscription streams, Multi Round-Trip Request (MRTR) wire types, and typed results with machine-readable output schemas. Hand-implementing all of that would grow a bespoke wire layer that every future protocol revision would have to re-earn inside our own code. The refreshed migration baseline (`docs/research/mcp-2026-07-28-refresh.md`, ticket #163) confirmed the parts we own are healthy and must not change: the explicit `vault_id`/`scope` tool catalogue (35 tools), the per-request security ordering, structured error semantics, and the shared `vault/write` mutation layer. The reconciled migration package (#43, corrected by #164) chose a deep protocol boundary: future revisions should arrive through an SDK upgrade, not through re-writing transport code.
- **Decision:** Two halves, decided together:

  1. **Adopt stable rmcp 3.x as the MCP protocol boundary**, pinned to exactly `rmcp = "=3.1.4"` in Cargo.toml (an exact-version requirement; published on crates.io 2026-08-20; MSRV 1.88; Apache-2.0). RMCP owns JSON-RPC framing, Streamable HTTP serving, lifecycle and version negotiation, discovery, request `_meta`/header validation, subscription streams, MRTR-aware wire types, and version-specific serialization. The existing tool catalogue and dispatcher remain framework-independent behind a typed adapter implementing rmcp's `ServerHandler` seam; the 35 tools are NOT rewritten into rmcp's macro-generated route tables.
  2. **Narrow the advertised protocol revisions to exactly `2026-07-28` and `2025-11-25`.** Modern clients work without any initialization handshake — `server/discover` replaces `initialize` for them, and requests are served statelessly. Legacy clients keep the existing initialize/negotiation flow unchanged. `2025-03-26` and `2025-06-18` are no longer advertised (`2024-11-05` was already absent).

  The trait/type surface above was verified against the actual crate source before acceptance — tag `rmcp-v3.1.4`, commit `4a738b9dd99eaca418b614afa433a0cbdaf8d056`:

  - Protocol version constants incl. `V_2026_07_28` and `V_2025_11_25`: `crates/rmcp/src/model.rs:170–171`; both parse back: `model.rs:215–216`.
  - `ServerHandler` trait (default methods overridable one at a time): `crates/rmcp/src/handler/server.rs:593`; default `discover()` method returning server discovery information: `handler/server.rs:342–343`; `server/discover` method string: `model.rs:1148`.
  - `StreamableHttpService` implements `tower::Service` for any `S: ServerHandler`, so it mounts behind axum without replacing the router: `crates/rmcp/src/transport/streamable_http_server/tower.rs:999+`. Version advertisement is handler-controlled via `supported_protocol_versions()`: `tower.rs:347`, surfaced in `ServerInfo.supported_versions`: `model.rs:1187`.
  - Stateless-by-default for modern clients is structural, not optional: sessions are removed from `2026-07-28` per SEP-2567 and such requests "are always served statelessly regardless of this setting" (`legacy_session_mode` doc comment): `tower.rs:66–72`; a strict per-request metadata gate exists as `stateless_protocol_metadata_required`: `tower.rs:130–155`.
  - MRTR types (SEP-2322 `InputRequiredResult`, version-gated to ≥ `2026-07-28`): `crates/rmcp/src/model/mrtr.rs`.
  - Subscription stream request `subscriptions/listen`: `model.rs:2073`; client opt-in categories incl. `tools_list_changed`: `model.rs:1912–1926`.
  - Typed results: `CallToolResult.structured_content`: `model.rs:3785–3803`; `Tool.output_schema` (+ builder): `model/tool.rs:30`, `:210` — schema generation rides the `schemars` feature.

  **Branching revision:** implementation happens on a single feature branch `feature/mcp-2026-07-28`, cut from current `development` and merged back via PR after verification (ticket #172). This supersedes item 11 of the #43 decision package: the previously planned stacking on `feature/ui-ux-polish` is obsolete because that content has already landed on `development`.
- **Consequences:** Future MCP protocol revisions primarily arrive through rmcp upgrades instead of hand-written wire code; each upgrade becomes a deliberate, reviewed event against this record's pin. The hand-written JSON-RPC/transport code in `src/mcp/protocol.rs` and `src/mcp/routes.rs` shrinks into a thin adapter, so the boundary keeps its file inventory only if the map is updated when files move (module-map rule). Security posture must be re-expressed at the new boundary without weakening: the per-request ordering enabled check → token configured → Origin allowlist → constant-time bearer compare → protocol-version header, plus the v2.5.0 rule that the MCP token works on the attachment endpoint only while MCP *and* MCP write mode are live-enabled; a dedicated regression test proves it survives the swap (#172). Rate limiting (#171), subscriptions/listen with honest `listChanged` (#170), the modern surface (#169), and typed results/outputSchema for all 35 tools (#167) build directly on this seam. Golden wire tests lock both supported revisions across the replacement. Costs accepted: one additional dependency tree (rmcp + tower/hyper ecosystem pieces already present via axum), an MSRV floor of Rust 1.88, and the discipline of keeping domain logic out of the adapter.
- **Evidence:** `Cargo.toml` gains the pinned dependency during #168 (verified absent/greenfield today by #163); verified surface cited above against tag `rmcp-v3.1.4` (commit `4a738b9dd99eaca418b614afa433a0cbdaf8d056`) of <https://github.com/modelcontextprotocol/rust-sdk>; current hand-written surface: `src/mcp/{config.rs,protocol.rs,routes.rs,auth.rs}`; `docs/research/mcp-2026-07-28-refresh.md`; tickets #43, #164, #167–#172.

## ADR-18 — Per-Vault Git turns supersede the single-Vault debounced sync task

- **Status:** Accepted (supersedes ADR-10's mechanism only; every ADR-10 safety decision remains in force and is restated below)
- **Context:** ADR-10 fixed optional Git sync as one debounced background task over the process-wide `VAULT_PATH`, started from the settings page and locking phase by phase against an instance-wide write lock. The Vault collection runtime (v2.5.0) changed the unit of work: each Vault carries its own definition (source kind, Git mode, credentials, poll interval, commit identity) and gets its own Git turn, requested through the shared `VaultWorkCoordinator` by the managed-Git scheduler, a manual sync or retry, or activation, and executed by runtime composition under that Vault's mutation lock. The old task still compiled and was still reachable from the settings Git lifecycle, which gates on a value the collection runtime never sets, so in production it answered 503; the architecture review of 2026-08-28 (#162) found the two mechanisms had diverged on locking granularity, drift handling, and recovery, and that the instance-wide Versioning console and `/api/git-status` reported only the dead task.
- **Decision:** The per-Vault Git turn is the only Git synchronisation mechanism. One turn per Vault, one operation at a time, scheduled through the coordinator with no second execution lane; a turn holds that Vault's mutation lock for its whole blocking duration; the Vault's source kind and Git mode select the operation (acquire-or-reuse then synchronise a managed checkout; synchronise an existing checkout with its one configured remote; commit local history without touching a remote). ADR-10's safety decisions are restated for the turn: sync is optional and off per Vault by default; a turn never force-checks out over uncommitted manual edits, reporting a dirty working copy or a conflict as a structured, non-retryable error with the affected paths; local-history mode never contacts a remote; writes never block on sync except for the one Vault whose turn is running. The instance-wide debounced task, its handle, its write-record hook, and the instance-wide write lock are removed (#185). The boot-time parse of `HATCHDOOR_GIT_*` is deliberately kept, now serving only two purposes: refusing to start on a half-configured Git mode, and feeding the demo-mode posture refusal. No runtime behaviour reads its result. `HATCHDOOR_GIT_AUTHOR_NAME` and `HATCHDOOR_GIT_AUTHOR_EMAIL` remain as the commit-identity fallback for a Vault without its own and are read per turn (#181); the other `HATCHDOOR_GIT_*` keys remain only as first-boot import inputs until #82 closes.
- **Consequences:** One Git code path to audit, and Git behaviour that is configured, reported, and controlled per Vault (status, error detail, `Sync now`, `Try again`). The per-turn lock is coarser than the old per-phase lock: a foreground write to a Vault waits while that Vault's turn runs, a trade accepted in #96 over a finer discipline. The commit-on-write debounce has no successor: the watcher's fixed debounce coalesces writes into the next turn, and the per-Vault poll interval answers how often to look at the remote. The settings page loses its instance-wide Versioning console and `/api/git-status` is retired (#183).
- **Evidence:** `git/managed_task.rs` (`run_managed_git_turn`, `run_existing_git_remote_turn`, `ManagedGitScheduler`), `git/managed_sync.rs` (`synchronize_managed_checkout`), `git/managed_checkout.rs` (`acquire_or_reuse`), `git/sync.rs` (`run_local_history_git_turn`), `vault_executor.rs` (`dispatch_git_turn_with`, `plan_git_turn`, `publish_managed_git_turn_outcome`; extracted from `vault_runtime.rs` in #197), `vault_registry.rs` (`VaultGitMode`, `poll_interval_secs`, `commit_identity`); #162, #181, #183; issues #94–#97, #132 for the per-Vault turn's history. The removal itself landed in #185: `git/task.rs` and `git/status.rs` are deleted outright, `git/sync.rs` keeps only the local commit path (`validate_repo`, `validate_local_repo`, `init_local_repo`, `commit_local`, `run_local_history_git_turn`) with its fetch/integrate/push, unpushed accounting, and merge-marker recovery gone, `AppState` loses `git_sync`, `vault_write_lock`, and `startup_git_config`, `server.rs` loses the sync-task shutdown drain but keeps the boot-time `HATCHDOOR_GIT_*` parse for configuration validation and the demo posture check (`server.rs`, `legacy_git_config` into `check_demo_mode_posture`), and `handlers/settings.rs` loses `patch_settings_with_git_lifecycle` and `production_sync_ops`.

## ADR-19 — Vault-qualified cores are the only seam an adapter crosses

- **Status:** Accepted (refines ADR-02 and ADR-03; supersedes neither)
- **Context:** ADR-02 promises one shared core behind three surfaces and ADR-03 one write module behind thin adapters. The adapters did not stay thin. The HTTP and MCP write adapters each re-implemented the same orchestration around `vault/write` (Vault gate, capability check, per-Vault lock, authoritative index build, entry lookup, noise and marker refusal, archive-prefix resolution, off-thread execution, error typing) at 1,224 and 1,499 lines, and had drifted: MCP ran writes on the async runtime while HTTP offloaded them. Sixteen MCP tools reached the domain by calling axum handler functions with hand-built extractors and decoding the HTTP response body through a byte cap and two serialisation passes, so a tool could only be tested with the transport on both sides. Vault collection management (registry commit, runtime reconcile, revision response, projections, the structured error type) lived in an HTTP handler that MCP imported as a library. `VaultReadCore` and `VaultSearchCore` already showed the intended shape: a Vault-qualified core with a small interface, consumed by both adapters (#162).
- **Decision:** Four Vault-qualified cores are the seams adapters cross: `VaultReadCore` (exact reads, collection reads, contained resources), `VaultSearchCore`, the Vault mutation core (every write primitive with its orchestration; #184, #186), and the Vault collection management module (#187). HTTP handlers and MCP tools are wire-shaping adapters: they parse the transport's input, call exactly one core, and map the core's typed outcome or its structured error `{code, message, vault_id?, retryable}` to a status code or a tool error. An adapter never calls another adapter (no MCP-to-handler proxying; #188), never reaches past a core to `vault/write`, the registry, the runtime, the scheduler, or the filesystem, and holds no policy a core could hold. Cores own the off-thread execution of blocking work. The cores stay plain structs (ADR-13): no trait seam is added to formalise them.
- **Consequences:** Domain behaviour is tested once, at a core's interface, with a filesystem fixture; each adapter keeps one mapping test per route or tool. A new surface or tool costs a mapping, not a re-implementation, and a new write primitive reaches every surface at once. Wire shapes do not change when orchestration moves; relocating a wire type follows the interface-change checklist. The module map gains two boundaries and its adapter sections shrink to wire shaping. Until #184–#188 land, the HTTP handlers remain the de facto library MCP calls, and that is the debt this record retires.
- **Evidence:** `vault_read.rs` (`VaultReadCore`), `search/vault_scoped.rs` (`VaultSearchCore`); #162 and its child tickets #184, #186, #187, #188; `docs/architecture/module-map.md` (Vault-qualified read projections, HTTP adapters, MCP adapter).

## ADR-20 — Deleting provably dead retrieval code needs a proof, not an eval run

- **Status:** Accepted (refines ADR-15; supersedes neither. ADR-05 needs no exemption — see Context)
- **Context:** ADR-15 requires any change touching the retrieval path, "pre- or post-filters" named explicitly, to be validated against the eval set before merge. #210 removed `NoteFilters`, `include_properties`, the property projection they drove, the branch selecting between two semantic retrieval implementations, and `semantic_hits` behind it — code that sits squarely on that path and could not run. Every producer of a `VaultSearchRequest` built the filters empty (the web search route, the `search_notes` adapter, the eval harness), `search_notes` refuses a `filters` argument at the protocol level with a test pinning the refusal, and the branch was guarded by `!request.filters.is_empty()`. The three live call sites of `NoteFilters::matches` were a no-op by construction: each check is an `all()` over an empty iterator and the property arm returned `is_empty() && is_empty()`, so it answered `true` for every note. Read literally, ADR-15 demanded an eval run for a deletion that no eval could speak to: the harness measures what executes, and the claim was that this did not. Running it would have reported the same numbers for a reason unrelated to the change, which is a weaker statement than the proof already in hand, not a stronger one. ADR-05 was cited alongside ADR-15 in that PR but was never actually in tension: its constraint is against *adding* reranking or FTS/vector fusion to the runtime path, and a deletion adds nothing.
- **Decision:** An eval run is not required to delete retrieval-path code when all four hold, and every one of them is recorded in the PR:

  1. **The code is dead, and the proof is by enumeration.** Either (a) it is unreachable — a guard no producer in the crate can satisfy — or (b) it executes but is provably an identity for every input any surface can construct. Both are established by listing every producer of the input and reading the callee's own definition. "I could not find a caller" is not this proof; neither is "this is always a no-op in practice."
  2. **It is a deletion.** Code that survives the change is not modified on the strength of this record. A deletion bundled with a behavioural tweak forfeits the exemption for the whole change.
  3. **A before/after comparison is measured, not argued.** The pre-change and post-change builds are run over the real surfaces against fixtures, covering each retrieval route the deletion touches, and return identical results, order, scores and payload bytes. Any field that legitimately differs is named and explained.
  4. **Doubt resolves to the eval.** If the enumeration is incomplete, the identity argument leans on typical rather than all inputs, or the comparison differs anywhere unexplained, ADR-15 applies unchanged.

  Wire shapes that were only ever produced by the deleted code are pinned by a test asserting the *serialized* output, so a later change cannot quietly turn a constant into something else.
- **Consequences:** Dead weight on the retrieval path can be removed at the cost of an argument rather than a reindex and an eval run, which is what makes removing it likely to happen at all — ADR-15's literal reading gave a standing incentive to leave unreachable code where it sat. The exemption is deliberately hard to claim: it demands an enumeration a reviewer can check line by line, and it collapses to ADR-15 the moment the deleted code turns out to have run. It says nothing about adding, tuning, or reordering anything on the retrieval path, where ADR-15 stands in full. The risk accepted is a wrong reachability proof, and it is mitigated by condition 3: a deletion that changes behaviour shows up as a differing result before it reaches `development`.
- **Evidence:** #210 and its PR #211 (the enumeration of producers, the `debug_assert!(!request.filters.is_empty())` that stated the dead branch's own precondition, and the eight-case keyword/semantic/tag comparison over the MCP endpoint and the HTTP search route returning identical results, order, scores and bytes); `search/mod.rs` and `search/vault_scoped.rs` as they stand after the deletion; the surviving `search_rejects_legacy_metadata_filters` test; `eval/` harness and ADR-15 for everything this record does not exempt.
