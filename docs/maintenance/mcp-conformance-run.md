# Manual MCP conformance run

Mandatory manual release evidence for any release that touches `/mcp`, the MCP
tools surface, or MCP-facing security behavior. This is deliberately **not** a
CI check (decision package #43 item 10): a clean recorded run before each
release is the evidence. The release-runbook pre-merge checklist links here.

## Runner

Official suite: [`@modelcontextprotocol/conformance`](https://github.com/modelcontextprotocol/conformance),
invoked via `npx`. Record the version used (`just conformance` prints its `--version`)
in the evidence.

**Pin the version explicitly:** `@0.2.0-alpha.11`, which is what every recorded
run has used. The bare unpinned package resolves to the stable dist-tag,
**0.1.16**, which has no `--requirements` option and therefore cannot run the
invocations below at all. It offers only the superseded
`--suite`/`--spec-version`. The pin is required, not cosmetic, and
`just conformance` carries it.

## Running it

```bash
just dev-start
just dev-wait
just conformance
```

`just conformance` (`scripts/conformance.mjs`) does the whole run against the
local dev build and stops at once, naming `just dev-start`, when the dev server
does not answer. It:

1. Prints the runner's `--version`.
2. Starts a proxy on `127.0.0.1:42999` that adds the MCP bearer token (see
   [Authentication](#authentication)).
3. Restarts the backend write-enabled (`HATCHDOOR_MCP_ENABLED=true`,
   `HATCHDOOR_MCP_WRITE_ENABLED=true`), counts the tools it lists, and runs the
   suite once per protocol revision the build supports, read from
   `SUPPORTED_PROTOCOL_VERSIONS` in `src/mcp/config.rs`.
4. Does the same read-only (`HATCHDOOR_MCP_WRITE_ENABLED=false`).
5. Restarts the backend on its usual settings, and prints each run's exit
   status and the two tool counts. It fails when any run exited non-zero, or
   when both postures listed the same number of tools.

Each run is, for revision `<rev>`:

```bash
npx -y @modelcontextprotocol/conformance@0.2.0-alpha.11 server \
  --url http://127.0.0.1:42999/mcp \
  --expected-failures docs/maintenance/conformance-baseline.yml \
  --requirements <rev> --output-dir <evidence dir>
```

`--requirements` runs exactly what that revision requires at that revision's
wire. Each run's results go to
`docs/maintenance/conformance-evidence/<date>-<write|ro>-<MMDD of the revision>/`,
such as `2026-10-27-write-1125/`, with the console summary in `summary.txt`. The
command refuses to start when a directory for today is already there, so it
never overwrites evidence.

## Authentication

The conformance harness sends plain MCP requests with **no** `Authorization`
header, while Hatchdoor requires its bearer token on every request. The proxy
`just conformance` starts adds the token (`HATCHDOOR_MCP_BEARER_TOKEN`, from
the environment or `.env`) instead of weakening the server, and cuts it from
everything it prints or writes.

Missing `Origin` headers are accepted by Hatchdoor (only *present* Origins are
allowlist-checked, `src/mcp/auth.rs`), so injecting the token is sufficient;
the suite's own `dns-rebinding-protection` scenario exercises Origin/Host
validation explicitly and must pass unbaselined.

## Scope baseline

Many scenarios assume capabilities or a tool catalogue Hatchdoor intentionally
does not have (prompts, resources, logging, completion, elicitation/sampling;
the `tools-call-*` scenarios call the suite's reference fixtures such as
`test_simple_text`). These are excused via the framework's own
expected-failures mechanism — see
[`conformance-baseline.yml`](./conformance-baseline.yml) next to this file.
Baseline rules:

- Every entry carries a reason; entries are reviewed at each conformance run.
- The framework exits non-zero when a baselined scenario **passes** (stale
  entry) — so entries prune themselves when reality changes.
- `wire-schema-valid` failures are **never** baselined; they are always
  release blockers.

## Pass criteria

A run is clean — and only a clean run is valid release evidence — when all of
the following hold for **each posture** (write-enabled and read-only):

1. Every invocation exits **0**.
2. The printed summary shows zero unexpected failures; every failure that did
   occur has a reviewed entry in `conformance-baseline.yml`.
3. No stale-baseline exit occurred (a baselined scenario passing means the
   entry must be removed and the run repeated).
4. All `wire-schema-valid` checks pass.
5. Evidence is archived with the release: the result directories
   `just conformance` wrote, and beside them a `<date>-v<version>-release.md`
   written by hand, as the earlier ones are, recording the runner version, the
   command lines, the tool counts, each run's totals, and whether the baseline
   was reviewed and changed.

Any other outcome blocks the release until fixed or, for baseline changes,
until the baseline is amended with a reason and the run is repeated clean.
