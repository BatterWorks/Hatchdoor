# Manual MCP conformance run

Mandatory manual release evidence for any release that touches `/mcp`, the MCP
tools surface, or MCP-facing security behavior. This is deliberately **not** a
CI check (decision package #43 item 10): a clean recorded run before each
release is the evidence. The release-runbook pre-merge checklist links here.

## Runner

Official suite: [`@modelcontextprotocol/conformance`](https://github.com/modelcontextprotocol/conformance),
invoked via `npx`. Record the version used (`npx @modelcontextprotocol/conformance --version`)
in the evidence; first verified with **0.1.16** (2026-08-24).

## Target configuration

Start a local dev build (`just dev-start`) and let it settle Ready. Endpoint is
`http://127.0.0.1:<PORT>/mcp` (`HOST`/`PORT` from `.env`; the development
default is `42824`).

Run twice, once per write posture:

1. **Write-enabled** (superset): `HATCHDOOR_MCP_ENABLED=true`,
   `HATCHDOOR_MCP_WRITE_ENABLED=true`.
2. **Read-only**: same but `HATCHDOOR_MCP_WRITE_ENABLED=false`.

Advertised revisions of the build under test decide the invocation:

- Current `development` advertises `2025-03-26`, `2025-06-18`, `2025-11-25`
  (initialize-negotiated): use the **Stage A** invocation.
- After ADR-17 lands (#168/#169), exactly `2025-11-25` and `2026-07-28`: use
  the two **Stage B** invocations — one per revision, each at its own wire.

## Authentication

The conformance harness sends plain MCP requests with **no** `Authorization`
header, while Hatchdoor requires its bearer token on every request. Put a tiny
header-injecting proxy in front instead of weakening the server. Save as
`mcp-auth-proxy.mjs`:

```js
// Usage: MCP_TARGET=http://127.0.0.1:PORT MCP_TOKEN=... node mcp-auth-proxy.mjs [listenPort]
import http from 'node:http';
const target = new URL(process.env.MCP_TARGET);
const token = process.env.MCP_TOKEN;
const port = Number(process.argv[2] ?? 42999);
http.createServer(async (req, res) => {
  const chunks = [];
  for await (const c of req) chunks.push(c);
  const proxied = http.request({
    protocol: target.protocol, hostname: target.hostname,
    port: target.port, path: req.url, method: req.method,
    headers: { ...req.headers, host: target.host,
               authorization: `Bearer ${token}` },
  }, (up) => { res.writeHead(up.statusCode, up.headers); up.pipe(res); });
  proxied.on('error', (e) => { res.writeHead(502); res.end(String(e)); });
  proxied.end(Buffer.concat(chunks));
}).listen(port, '127.0.0.1', () => console.log(`proxy 127.0.0.1:${port} -> ${target.origin}`));
```

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

## Invocations

```bash
# Stage A — current development (legacy revisions, initialize-negotiated)
npx @modelcontextprotocol/conformance server \
  --url http://127.0.0.1:42999/mcp \
  --expected-failures docs/maintenance/conformance-baseline.yml \
  --output-dir /tmp/conformance-$(date +%F)

# Stage B — after ADR-17 lands: one run per advertised revision
npx @modelcontextprotocol/conformance server \
  --url http://127.0.0.1:42999/mcp \
  --expected-failures docs/maintenance/conformance-baseline.yml \
  --requirements 2025-11-25 --output-dir /tmp/conformance-$(date +%F)-1125
npx @modelcontextprotocol/conformance server \
  --url http://127.0.0.1:42999/mcp \
  --expected-failures docs/maintenance/conformance-baseline.yml \
  --requirements 2026-07-28 --output-dir /tmp/conformance-$(date +%F)-0728
```

Run from the repository root so the relative `--expected-failures` path
resolves (`--requirements` replaces `--suite`/`--spec-version` and runs exactly
what that revision requires at that revision's wire).

## Pass criteria

A run is clean — and only a clean run is valid release evidence — when all of
the following hold for **each posture** (write-enabled and read-only):

1. Every invocation exits **0**.
2. The printed summary shows zero unexpected failures; every failure that did
   occur has a reviewed entry in `conformance-baseline.yml`.
3. No stale-baseline exit occurred (a baselined scenario passing means the
   entry must be removed and the run repeated).
4. All `wire-schema-valid` checks pass.
5. Evidence is archived with the release: runner version, full command lines,
   the summary output, and the `results/` directory.

Any other outcome blocks the release until fixed or, for baseline changes,
until the baseline is amended with a reason and the run is repeated clean.
