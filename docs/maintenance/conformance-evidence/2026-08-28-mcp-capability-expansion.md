# MCP conformance run — 2026-08-28 (issue #174, MCP capability expansion)

Clean release evidence for the 39-tool catalogue on `feature/mcp-2026-07-28`
(PR #179). Supersedes [`./2026-08-25-mcp-migration.md`](./2026-08-25-mcp-migration.md),
which verified the same boundary at 35 tools before `get_frontmatter`,
`update_frontmatter`, `get_attachment`, and `batch` were added; neither run has
shipped yet, and this is the one that covers what the next release will contain.
Procedure: [`../mcp-conformance-run.md`](../mcp-conformance-run.md).

- Runner: `@modelcontextprotocol/conformance` **0.2.0-alpha.11** via `npx -y`
  (`--version` confirmed at the start of this run), the same version the
  superseded run used.
- Server: local dev build (`just dev-start`), `http://127.0.0.1:42824/mcp`,
  reached through the documented header-injecting proxy on `127.0.0.1:42999`.
  A stale proxy left listening on that port from the 2026-08-25 run was stopped
  first, so every request here went through a proxy whose target and token were
  verified against this build.
- Baseline: [`../conformance-baseline.yml`](../conformance-baseline.yml),
  reviewed for this run and **unchanged** — no entry went stale (the framework
  exits non-zero when a baselined scenario passes, and all four runs exited 0),
  and the four new tools needed no new entry.

## Catalogue under test

Confirmed against the running build before the suite was invoked:

- Write-enabled: **39** tools advertised, including `get_frontmatter`,
  `update_frontmatter`, `get_attachment`, and `batch`.
- Read-only: **17** tools, matching the golden read-only list — the four new
  tools' read side (`get_frontmatter`, `get_attachment`, `batch`) present,
  `update_frontmatter` correctly absent.

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

Run once per write posture (`HATCHDOOR_MCP_WRITE_ENABLED=true|false`, server
restarted between postures via `just dev-start`).

## Results

| Posture       | Revision   | Result                                  | Totals            | Results dir              |
| ------------- | ---------- | --------------------------------------- | ----------------- | ------------------------ |
| Write-enabled | 2025-11-25 | exit 0 — "Baseline check passed"        | 44 passed, 25 expected failures | `./2026-08-28-write-1125/` |
| Write-enabled | 2026-07-28 | exit 0 — "Baseline check passed"        | 99 passed, 70 expected failures | `./2026-08-28-write-0728/` |
| Read-only     | 2025-11-25 | exit 0 — "Baseline check passed"        | 44 passed, 25 expected failures | `./2026-08-28-ro-1125/`    |
| Read-only     | 2026-07-28 | exit 0 — "Baseline check passed"        | 99 passed, 70 expected failures | `./2026-08-28-ro-0728/`    |

Each directory carries the suite's own per-scenario output plus a `summary.txt`
holding that invocation's full console summary.

## Pass criteria

All five hold, for both postures:

1. Every invocation exited **0**.
2. Every failure that occurred has a reviewed entry in
   `conformance-baseline.yml`; the summary reports zero unexpected failures.
3. No stale-baseline exit — the baseline needed no amendment this run.
4. All `wire-schema-valid` checks passed in every run.
5. Evidence archived here: runner version, command lines, summaries, and the
   four `results/` directories.

The failure counts are identical to the superseded run's at each revision, which
is the expected outcome: the four added tools are additive and the baseline's
entries are all for capabilities Hatchdoor intentionally does not implement
(prompts, resources, logging, completion, elicitation/sampling) or for
suite-owned fixture and diagnostic tool families.
