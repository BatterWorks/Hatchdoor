# MCP conformance run — 2026-08-25 (issue #172, MCP 2026-07-28 migration)

> [!note]
> **Superseded** by [`./2026-08-28-mcp-capability-expansion.md`](./2026-08-28-mcp-capability-expansion.md).
> This run verified the boundary at 35 tools; issue #174 then added four more,
> and the later run covers the 39-tool catalogue the next release will ship.
> Kept as the record of the ADR-17 migration itself.

Clean release evidence for the ADR-17 boundary swap (`feature/mcp-2026-07-28`
integrated into `development`). Procedure: [`../mcp-conformance-run.md`](../mcp-conformance-run.md).

- Runner: `@modelcontextprotocol/conformance` **0.2.0-alpha.11** via `npx -y`
  (first runner supporting `--requirements 2026-07-28`; `--version` on the
  pinned 0.1.16 was recorded during baseline work earlier the same day).
- Server: local dev build (`just dev-start`), `http://127.0.0.1:42824/mcp`,
  reached through a header-injecting proxy on `127.0.0.1:42999` per the
  documented procedure.
- Baseline: [`../conformance-baseline.yml`](../conformance-baseline.yml),
  reviewed and amended this run:
  - pruned stale `server-sse-multiple-streams` (RMCP boundary now serves streams) and
    `completion-complete` (suite satisfied by the -32601 answer since 0.1.16);
  - added reviewed 2026-07-28 entries (capability-gated caching/resources
    detail checks, SEP-2575 stateless scenario, suite-owned fixture/diagnostic
    tool families: json-schema-2020-12, custom-header, input-required, tasks).

## Command lines

```bash
npx -y @modelcontextprotocol/conformance@0.2.0-alpha.11 server \
  --url http://127.0.0.1:42999/mcp \
  --expected-failures docs/maintenance/conformance-baseline.yml \
  --requirements 2025-11-25 --output-dir <dir>
npx -y @modelcontextprotocol/conformance@0.2.0-alpha.11 server \
  --url http://127.0.0.1:42999/mcp \
  --expected-failures docs/maintenance/conformance-baseline.yml \
  --requirements 2026-07-28 --output-dir <dir>
```

Run once per write posture (`HATCHDOOR_MCP_WRITE_ENABLED=true|false`,
server restarted between postures).

## Results

| Posture      | Revision    | Result                                        | Results dir                          |
| ------------ | ----------- | --------------------------------------------- | ------------------------------------ |
| Write-enabled| 2025-11-25  | exit 0 — "Baseline check passed"              | `./conformance-final-write-1125b/`   |
| Write-enabled| 2026-07-28  | exit 0 — "Baseline check passed"              | `./conformance-final-write-0728g/`   |
| Read-only    | 2025-11-25  | exit 0 — "Baseline check passed"              | `./conformance-final-ro-1125/`       |
| Read-only    | 2026-07-28  | exit 0 — "Baseline check passed"              | `./conformance-final-ro-0728/`       |

All `wire-schema-valid` checks passed in every run; no stale-baseline exit.

## Fix that came out of this run

`sep-2575-http-server-error-jsonrpc-id` initially failed: the retired-version
header rejection answered `"id": null`. Fixed in `src/mcp/auth.rs` +
`src/mcp/routes.rs` — the check now runs after body buffering and echoes the
request id (regression asserted by `retired_protocol_version_header_is_rejected_cleanly`).
