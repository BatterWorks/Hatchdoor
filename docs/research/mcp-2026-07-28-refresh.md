# Refresh: MCP migration baseline against current development

Resolves research ticket [#163](https://github.com/BatterWorks/Hatchdoor/issues/163), part of wayfinder map #41.
Baseline being refreshed: closed ticket [#42](https://github.com/BatterWorks/Hatchdoor/issues/42) (v2.4.0 assessment) and decision package in closed ticket [#43](https://github.com/BatterWorks/Hatchdoor/issues/43).
Investigated against `development` at commit `2e4e035` (branch `research/refresh-mcp-baseline`); every claim cites a file:line or issue URL.

## Verdict

The v2.4.0 assessment's shape still holds — hand-written POST-only adapter, no SDK, tools-only surface — but **the tool catalogue it measured no longer exists**. The v2.5.0 Vault-collection release rebuilt every tool around explicit `vault_id`/`scope` addressing, added 7 vault-management and 3 setup tools (29 → 35 when write mode is on), and changed the advertised capabilities. Roughly half of ticket #43's decision items are unstarted; the parts of the migration package that name "the 29 tools" and their schemas must be revised before implementation tickets are written.

## Deltas vs the v2.4.0 assessment (#42)

### 1. Tool catalogue — materially changed (revision required)

- #42 counted **29 tools** with write mode enabled. Current catalogue is **35**: 3 first-run setup tools (`get_model_setup_status`, `accept_gemma_terms`, `decline_gemma_terms`, always advertised — `src/mcp/tools/mod.rs:193-223`), 11 read tools (`src/mcp/tools/read.rs:616-641`), 7 Vault-collection management tools (`create_vault`, `edit_vault`, `enable_vault`, `disable_vault`, `disconnect_vault`, `sync_vault`, `retry_vault` — `src/mcp/tools/read.rs:641-712`), and 14 note/attachment write tools (`src/mcp/tools/write.rs:681-889`). Composition: `tools_list()` at `src/mcp/tools/mod.rs:262-268`.
- Every vault-dependent tool now takes an immutable `vault_id` or a collection `scope`; there is no default Vault ("no selected, sole, or default Vault" instruction, `src/mcp/config.rs:11`). Migration commits: `4a44da4` ("Migrate MCP tools to explicit vault scope"), `bf0f574`.
- Management writes use optimistic registry-revision control (`expected_registry_revision`) — a concurrency model #42 never saw (`src/mcp/tools/read.rs:645-659`).
- **No `outputSchema` exists anywhere under `src/mcp/`** (repo-wide grep returns nothing). Decision item 7 of #43 ("add `outputSchema` for all 29 tools") is entirely unstarted and must be renumbered/re-scoped to 35 tools.

### 2. Protocol versions — narrowed since assessment

- Supported set is exactly `"2025-11-25", "2025-06-18", "2025-03-26"`; `2024-11-05` was deliberately dropped because this adapter never implemented its HTTP+SSE transport (`src/mcp/config.rs:1-7`). This resolves #42's "incompatible `2024-11-05` claim" finding.
- Negotiation echoes a supported client version else falls back to `2025-11-25` (`config.rs:15-27`, applied at `routes.rs:160-176`).

### 3. RMCP 3.x dual-version behavior — not started

- No `rmcp` dependency in `Cargo.toml`; zero references repo-wide. Decision item 1 of #43 (adopt stable rmcp 3.x as protocol boundary) is unimplemented.
- What `2026-07-28` support would require on today's code (all absent from `src/mcp/routes.rs`): stateless modern requests without initialization (`server/discover` replaces `initialize` for modern clients), per-request `_meta` with matching `MCP-Protocol-Version`/capability HTTP headers, `Mcp-Method`/`Mcp-Name` validation, typed result types, `subscriptions/listen` replacing standalone change streams, cache metadata on list results, and MRTR handling. The header gate currently only validates `MCP-Protocol-Version` against the three legacy strings (`src/mcp/auth.rs:76-89`), so a `2026-07-28` request is rejected today.

### 4. Legacy sessions and notifications — partially improved

- Requests without an `id` (notifications) short-circuit to `202 Accepted` (`src/mcp/routes.rs:110-112`); unknown methods return `-32601` (`routes.rs:124-126`).
- `tools.listChanged` was flipped from `true` (flagged by #42 as dishonest) to **`false`**, with an honest comment: the POST-only transport has no channel to deliver the event, clients reissue `tools/list` (`routes.rs:170-175`, `193`). This removes the gap #42 named, but also means the `mcp_tools_changed` broadcast channel in `AppState` still has no transport consumer — only construction sites in tests/handlers (`grep mcp_tools_changed`: `app_state.rs`, tests). #43 item 5 (wire `subscriptions/listen` to this broadcast) remains fully open.
- GET `/mcp` answers 405 + `ALLOW: POST` (`routes.rs:50-61`): POST-only Streamable HTTP, no SSE stream, no session ids, no keep-alives — all of which #43 items 1/5 would introduce.

### 5. Resource protection / security model — strengthened, rate limits absent

- Static bearer token is mandatory whenever MCP is enabled, even read-only (`src/mcp/config.rs:139-150`), enforced per request before body collection (`auth.rs:14-24`, `routes.rs:36-38`).
- Origin allowlist blocks DNS rebinding; localhost allowances accept arbitrary ports but only loopback hosts (`auth.rs:96-116`, tested `auth.rs:131-145`).
- Ordering in `validate_mcp_request` (`auth.rs:14-92`): enabled check → token-configured check → Origin → constant-time bearer compare → protocol-version header. Matches ADR-09 (`docs/adr/README.md:120-124`).
- v2.5.0 breaking change relevant to agents: the multipart attachment endpoint accepts the MCP bearer token only while MCP *and* MCP write mode are both live-enabled, checked per request (`CHANGELOG.md` v2.5.0 breaking-changes section; module map lines ~1229, ~1503). The `SERVER_INSTRUCTIONS` text now qualifies the token capability accordingly (`config.rs:11`).
- **#43 item 9 (layered limits: 120 calls/min/token, 8 concurrent ordinary / 2 expensive searches, 4 live subscriptions, HTTP 429 + `Retry-After`) has no implementation anywhere under `src/mcp/`** (grep for `Retry-After|rate.limit|429` returns nothing outside unrelated test names). Only response-size bounds exist: 8 MiB JSON-RPC ceiling (`protocol.rs:12`), per-config request-body limits including a 512 MiB in-memory upload ceiling (`config.rs:44-56`, `186-208`).
- Per-Vault mutation lock shared with the HTTP adapter is in place (`src/mcp/tools/mod.rs:120-123`), consistent with ADR-03.

### 6. Response/error semantics — refined since assessment

- Text + `structuredContent` dual responses retained (`protocol.rs:158-172`).
- New since v2.4.0: `JsonRpcFailure::tool_level` flag routes domain failures (e.g. not-found) through `tool_structured_error`, preserving the shared Vault error object so agents branch on `code` (`protocol.rs:20-72`, `174-190`; `tools/mod.rs:255-266`). Write failures surface structured Vault errors (restored in `f6ed6d1`).
- `-32603` internals are logged and masked to "Internal server error" (`routes.rs:130-140`). This matches #43 item 8 closely; little revision needed beyond documenting structured errors as existing behavior the migration must preserve.

### 7. Conformance gates / validation — unchanged, suite still absent

- Repository validation is `cargo test --all` plus frontend suites (`CONTRIBUTING.md:23`); focused suites referenced in the module-map MCP section (`docs/architecture/module-map.md:1527`: `cargo test mcp`, vault write tests).
- No MCP conformance-suite integration anywhere in docs or CI (grep finds none). #43 item 10 (manual conformance run before releases) has no recorded procedure yet — the migration package should specify one.

### 8. ADR / module-map impact

- ADR-02 (one binary, three surfaces), ADR-03 (writes via `vault/write/`), ADR-09 (MCP off by default, own token, Origin allowlist) all remain Accepted and remain valid under the #43 package (`docs/adr/README.md:76-86,120-124`). A new ADR will be needed for adopting RMCP as the protocol boundary and dropping `2025-03-26`/`2025-06-18` (supersedes part of ADR-09's "Streamable-HTTP implemented directly" sentence, line 124).
- Module map's "MCP adapter" section lists exactly the current 8 files and documents the POST-only contract, `listChanged: false`, and validation commands (`docs/architecture/module-map.md:1463-1527`). Migration changes the public contract (`initialize` behavior, headers, discovery) → interface-change checklist applies and the section must be rewritten; file list grows if RMCP adapter modules are added. Run `node scripts/check-module-map.mjs` after any structural change.

## Parts of the migration package needing revision before implementation tickets

1. **Tool count and schemas**: replace every reference to "29 tools" with the current 35-tool catalogue (3 setup + 11 read + 7 management + 14 write); `outputSchema` work is scoped against these, including the registry-revision and identity-change schemas management tools carry.
2. **Scope-addressing assumptions**: any ticket text assuming single-vault tools must be restated against `vault_id`/`scope` addressing, partial-participant envelopes, and optimistic `expected_registry_revision` semantics.
3. **Legacy version set**: the compatibility story drops not just to two versions but from a different starting point — `2024-11-05` is already gone, so the removal work for `2025-03-26`/`2025-06-18` is smaller than assumed.
4. **Notifications baseline**: `listChanged` is already honestly `false`; item 5's work is purely additive (build `subscriptions/listen` on the existing broadcast), not a fix of a wrong capability flag.
5. **Rate limiting**: item 9 is greenfield — no quota, 429, or Retry-After code exists; size-limit plumbing (`MAX_JSONRPC_RESPONSE_BYTES`, request-body limits) is the only precedent to build on.
6. **Security delta**: add the v2.5.0 rule — MCP token works on the HTTP attachment endpoint only while MCP writes are enabled — to the security section so the RMCP boundary doesn't regress it.
7. **Governance**: schedule one new ADR (RMCP adoption + version narrowing) and a module-map MCP-section rewrite ahead of code, per AGENTS.md guardrails.

## Unchanged and still valid

- Destination context holds: Markdown authority, disposable SQLite, all mutations through `src/vault/write/` (ADR-03 evidence paths unchanged), static bearer token + Origin checks (ADR-09), tools-only scope (no prompts/resources/Tasks/etc. in `capabilities`, `routes.rs:168-176`).
- Error-semantics decision (item 8) largely describes current behavior; frame those tickets as "preserve and cover", not "change".

## Residual risks

- Tool count drifts with development; pin tickets to a specific commit or recount at ticket-writing time.
- rmcp 3.x API details were not verified against the crate source here; confirm exact trait/type surface during design of the adapter ADR.
