import assert from "node:assert/strict";
import path from "node:path";
import { test } from "node:test";

import {
  evidenceDir,
  protocolRevisions,
  startProxy,
  withoutColour,
} from "./conformance.mjs";
import {
  FAKE_TOKEN,
  bodyOf,
  freePort,
  runNode,
  serve,
} from "./dev-test-support.mjs";
import { ToolError } from "./proto-common.mjs";

const SCRIPT = path.join(import.meta.dirname, "conformance.mjs");

test("reads the supported revisions oldest first, and refuses a file without them", () => {
  assert.deepEqual(
    protocolRevisions(
      'pub const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2026-07-28", "2025-11-25"];',
    ),
    ["2025-11-25", "2026-07-28"],
  );
  assert.throws(() => protocolRevisions("nothing here"), ToolError);
});

test("names the evidence directories the way the recorded runs do", () => {
  assert.equal(
    evidenceDir("2026-10-27", "write", "2025-11-25"),
    path.join("docs/maintenance/conformance-evidence", "2026-10-27-write-1125"),
  );
  assert.equal(
    evidenceDir("2026-10-27", "ro", "2026-07-28"),
    path.join("docs/maintenance/conformance-evidence", "2026-10-27-ro-0728"),
  );
  assert.equal(withoutColour("\u001b[32mpassed\u001b[0m"), "passed");
});

test("the proxy adds the bearer token and passes everything else through", async () => {
  const seen = [];
  const upstream = await serve(async (request, response) => {
    seen.push({
      authorization: request.headers.authorization,
      method: request.headers["mcp-method"],
      url: request.url,
      body: await bodyOf(request),
    });
    response
      .writeHead(201, { "mcp-session-id": "s1", "content-type": "text/plain" })
      .end("answer");
  });
  const port = await freePort();
  const proxy = await startProxy({
    target: `http://127.0.0.1:${upstream.port}`,
    token: FAKE_TOKEN,
    port,
  });
  try {
    const response = await fetch(`http://127.0.0.1:${port}/mcp?x=1`, {
      method: "POST",
      headers: { "Mcp-Method": "tools/list" },
      body: "{}",
    });
    assert.equal(response.status, 201);
    assert.equal(response.headers.get("mcp-session-id"), "s1");
    assert.equal(await response.text(), "answer");
    assert.deepEqual(seen, [
      {
        authorization: `Bearer ${FAKE_TOKEN}`,
        method: "tools/list",
        url: "/mcp?x=1",
        body: "{}",
      },
    ]);
    await assert.rejects(
      startProxy({ target: "http://127.0.0.1:1", token: FAKE_TOKEN, port }),
      /port \d+ is taken/,
    );
  } finally {
    await proxy.close();
    await upstream.close();
  }
});

test("fails at once, naming dev-start, when the dev server is not up", async () => {
  const port = await freePort();
  const result = await runNode([SCRIPT, "--port", String(port)], {
    env: { HATCHDOOR_MCP_BEARER_TOKEN: FAKE_TOKEN },
  });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /did not answer .*just dev-start/);
});
