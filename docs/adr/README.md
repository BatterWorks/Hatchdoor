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
| 10 | [Optional git sync as a debounced background task](#adr-10--optional-git-sync-as-a-debounced-background-task) | Accepted | Never force-checkout over uncommitted manual vault edits |
| 11 | [Soft delete (trash) and archive-by-move](#adr-11--soft-delete-trash-and-archive-by-move) | Accepted | Don't hard-delete note or asset files |
| 12 | [Distroless, rootless, multi-stage container](#adr-12--distroless-rootless-multi-stage-container) | Accepted | Don't assume a shell at runtime; keep the image rootless |
| 13 | [Deliberate minimalism: trait seams only where they pay](#adr-13--deliberate-minimalism-trait-seams-only-where-they-pay) | Accepted | Don't add speculative abstractions, state libraries, or frameworks |
| 14 | [No deployment step requires a browser](#adr-14--no-deployment-step-requires-a-browser) | Accepted | Don't add a setting the UI can set but the environment can't, or a startup gate only a browser can clear |
| 15 | [Search quality is a product feature, not a tunable](#adr-15--search-quality-is-a-product-feature-not-a-tunable) | Accepted | Don't change the retrieval path without eval numbers; don't trade recall or MRR for speed |
| 16 | [EmbeddingGemma by default behind a licence gate; representation locked at 800/50 with context](#adr-16--embeddinggemma-by-default-behind-a-licence-gate-representation-locked-at-80050-with-context) | Accepted | Don't change the embedder, chunk size, or contextual headers without re-running the eval |

> Records 01–13 were reconstructed and adopted on 2026-07-19 from the codebase,
> the CHANGELOG audit fixes (`F-01`…`F-17`), and the semantic-search evaluation.
> Records 14–16 were added on 2026-08-08.

---

## ADR-01 — Markdown is the source of truth; SQLite is a disposable read model

- **Status:** Accepted
- **Context:** Users own an Obsidian-style vault of `.md` files and must never be locked into Hatchdoor's storage.
- **Decision:** The `.md` files under `VAULT_PATH` are authoritative. SQLite is a generated cache built from them and safe to delete; it is rebuilt from the vault on demand. The schema is versioned and the embedder identity is stamped — a mismatch wipes and rebuilds rather than serving stale or mixed-model data.
- **Consequences:** The cache can live outside the vault and be discarded freely; schema or embedding-model changes cost a full reindex, accepted as the price of never corrupting the read model.
- **Evidence:** README "Data and Safety Model"; `app_state.rs` (`build_cache`); `cache/schema.rs` (`SCHEMA_VERSION`, `reset_if_embedder_changed`).

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

- **Status:** Accepted
- **Context:** Users want their vault edits versioned and pushed, without blocking writes or fighting manual git usage.
- **Decision:** Optional (off by default) auto commit-and-push via vendored `git2`, on a debounced background loop with conflict-abort semantics. It refuses to force-checkout over uncommitted manual edits to tracked files, surfacing them as an error. The watcher ignores `.git/` so sync churn doesn't trigger reindexing.
- **Consequences:** Writes stay fast (sync is async); manual edits are never silently discarded; sync is opt-in.
- **Evidence:** `git/{sync,task,message}.rs`; CHANGELOG v2.1.0, F-08.

## ADR-11 — Soft delete (trash) and archive-by-move

- **Status:** Accepted
- **Context:** Agents and the UI can delete or archive notes. Destructive deletes on user data are unacceptable.
- **Decision:** Delete moves notes and their referenced assets into `.hatchdoor-trash` (excluded from indexing). Archive moves notes under `HATCHDOOR_ARCHIVE_PREFIX` (default `90-archive/`), which also drives archived-link styling.
- **Consequences:** Deletes are recoverable; nothing is unlinked from disk by Hatchdoor. The trash folder is skipped when deciding whether a vault is empty.
- **Evidence:** README "Data and Safety Model"; `vault/write/`; `config.rs` (`archive_prefix`); CHANGELOG F-12.

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
