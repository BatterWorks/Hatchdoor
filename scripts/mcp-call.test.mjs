import assert from "node:assert/strict";
import path from "node:path";
import { test } from "node:test";

import {
  FAKE_TOKEN,
  bodyOf,
  freePort,
  runNode,
  serve,
} from "./dev-test-support.mjs";
import {
  argumentsFrom,
  callsFrom,
  messagesIn,
  openSession,
  payloadOf,
} from "./mcp-call.mjs";
import { ToolError } from "./proto-common.mjs";

const SCRIPT = path.join(import.meta.dirname, "mcp-call.mjs");

// A stand-in for the dev server's `/mcp`: the handshake the real one insists
// on, answers on an event stream that opens with an empty `data:` line, and
// tools that hand the caller's bearer token back, as a careless server might.
async function fakeMcp() {
  const seen = { initialized: 0, sessions: new Set(), origins: new Set() };
  const server = await serve(async (request, response) => {
    const bearer = request.headers.authorization ?? "";
    seen.origins.add(request.headers.origin);
    if (request.method === "DELETE") {
      response.writeHead(204).end();
      return;
    }
    const message = JSON.parse(await bodyOf(request));
    const reply = (payload, headers = {}) => {
      response.writeHead(200, {
        "content-type": "text/event-stream",
        ...headers,
      });
      response.end(
        `data: \nid: 0\nretry: 3000\n\ndata: ${JSON.stringify({ jsonrpc: "2.0", id: message.id, ...payload })}\n\n`,
      );
    };
    if (message.method === "initialize") {
      seen.initialized += 1;
      reply(
        { result: { protocolVersion: "2025-11-25", capabilities: {} } },
        { "mcp-session-id": `session-${seen.initialized}` },
      );
      return;
    }
    const session = request.headers["mcp-session-id"];
    if (!session) {
      response
        .writeHead(422, { "content-type": "text/plain" })
        .end(`expect initialize request, got ${bearer}`);
      return;
    }
    seen.sessions.add(session);
    if (message.method === "notifications/initialized") {
      response.writeHead(202).end();
      return;
    }
    const { name, arguments: args } = message.params;
    if (name === "echo") {
      reply({
        result: {
          content: [{ type: "text", text: JSON.stringify({ data: args }) }],
          structuredContent: { data: args, session },
        },
      });
    } else if (name === "text_only") {
      reply({ result: { content: [{ type: "text", text: '{"data":[1]}' }] } });
    } else if (name === "leaky") {
      reply({ result: { structuredContent: { you_sent: bearer } } });
    } else if (name === "fails") {
      reply({
        result: {
          isError: true,
          content: [
            {
              type: "text",
              text: JSON.stringify({ code: "note_not_found", auth: bearer }),
            },
          ],
        },
      });
    } else {
      reply({
        error: { code: -32602, message: `Unknown MCP tool: ${name} (${bearer})` },
      });
    }
  });
  return { ...server, seen };
}

const env = { HATCHDOOR_MCP_BEARER_TOKEN: FAKE_TOKEN };

test("an event stream gives up its answer past the empty data line", () => {
  assert.deepEqual(
    messagesIn(
      "text/event-stream",
      'data: \nid: 0\nretry: 3000\n\ndata: {"id":1,"result":{}}\n\n',
    ),
    [{ id: 1, result: {} }],
  );
  assert.deepEqual(messagesIn("application/json", '{"id":2}'), [{ id: 2 }]);
  assert.deepEqual(messagesIn("application/json", ""), []);
});

test("a result comes out of its envelope", () => {
  assert.deepEqual(
    payloadOf({ structuredContent: { data: [] }, content: [] }),
    { data: [] },
  );
  assert.deepEqual(
    payloadOf({ content: [{ type: "text", text: '{"a":1}' }] }),
    { a: 1 },
  );
  assert.equal(payloadOf({ content: [{ type: "text", text: "plain" }] }), "plain");
});

test("arguments are a JSON object, and a batch is one call a line", () => {
  assert.deepEqual(argumentsFrom(""), {});
  assert.deepEqual(argumentsFrom('{"scope":"all"}'), { scope: "all" });
  assert.throws(() => argumentsFrom("[1]"), ToolError);
  assert.throws(() => argumentsFrom("{nope"), ToolError);
  assert.deepEqual(
    callsFrom('# first\nlist_vaults\n\nget_note {"slug": "a b"}\n'),
    [
      { line: 2, tool: "list_vaults", args: {} },
      { line: 4, tool: "get_note", args: { slug: "a b" } },
    ],
  );
  assert.throws(() => callsFrom("ok\nbad {"), /line 2/);
});

test("one call prints the tool's result and does the handshake itself", async () => {
  const server = await fakeMcp();
  try {
    const run = await runNode(
      [SCRIPT, "--port", String(server.port), "echo", '{"scope":"all"}'],
      { env },
    );
    assert.equal(run.status, 0, run.stderr);
    assert.deepEqual(JSON.parse(run.stdout), {
      data: { scope: "all" },
      session: "session-1",
    });
    assert.equal(server.seen.initialized, 1);
    assert.deepEqual([...server.seen.origins], ["http://127.0.0.1"]);

    const text = await runNode(
      [SCRIPT, "--port", String(server.port), "text_only"],
      { env },
    );
    assert.deepEqual(JSON.parse(text.stdout), { data: [1] });
  } finally {
    await server.close();
  }
});

test("several calls share one session, from standard input or a script", async () => {
  const server = await fakeMcp();
  try {
    const run = await runNode([SCRIPT, "--port", String(server.port), "-"], {
      env,
      input: 'echo {"n":1}\necho {"n":2}\n',
    });
    assert.equal(run.status, 0, run.stderr);
    const lines = run.stdout.trim().split("\n").map((line) => JSON.parse(line));
    assert.deepEqual(lines, [
      { data: { n: 1 }, session: "session-1" },
      { data: { n: 2 }, session: "session-1" },
    ]);
    assert.equal(server.seen.initialized, 1);

    const session = await openSession({ port: server.port, token: FAKE_TOKEN });
    const first = await session.call("echo", { n: 3 });
    const second = await session.call("echo", { n: first.data.n + 1 });
    await session.close();
    assert.equal(second.session, first.session);
    assert.equal(second.data.n, 4);
  } finally {
    await server.close();
  }
});

test("a bad tool, a failing tool and a batch that fails exit non-zero with the server's words", async () => {
  const server = await fakeMcp();
  try {
    const port = String(server.port);
    const unknown = await runNode([SCRIPT, "--port", port, "nope"], { env });
    assert.equal(unknown.status, 1);
    assert.match(unknown.stderr, /Unknown MCP tool: nope/);
    assert.equal(unknown.stdout, "");

    const failing = await runNode([SCRIPT, "--port", port, "fails"], { env });
    assert.equal(failing.status, 1);
    assert.match(failing.stderr, /note_not_found/);

    const batch = await runNode([SCRIPT, "--port", port, "-"], {
      env,
      input: "echo\nfails\necho\n",
    });
    assert.equal(batch.status, 1);
    assert.equal(batch.stdout.trim().split("\n").length, 1);
    assert.match(batch.stderr, /line 2 \(fails\)/);

    const noJson = await runNode([SCRIPT, "--port", port, "echo", "{bad"], { env });
    assert.equal(noJson.status, 1);
    assert.match(noJson.stderr, /not JSON/);
    assert.equal(server.seen.initialized, 3, "bad arguments never reach the server");
  } finally {
    await server.close();
  }
});

test("mcp-call never prints the bearer token", async () => {
  const server = await fakeMcp();
  try {
    const port = String(server.port);
    const runs = [
      await runNode([SCRIPT, "--port", port, "leaky"], { env }),
      await runNode([SCRIPT, "--port", port, "fails"], { env }),
      await runNode([SCRIPT, "--port", port, "nope"], { env }),
      await runNode([SCRIPT, "--port", port, "-"], { env, input: "leaky\nfails\n" }),
      await runNode([SCRIPT, "--port", String(await freePort()), "echo"], { env }),
      await runNode([SCRIPT], { env }),
    ];
    // The server did get the token, or this would prove nothing.
    assert.match(runs[0].stdout, /"you_sent": "Bearer \[token\]"/);
    for (const run of runs) {
      assert.doesNotMatch(run.stdout + run.stderr, new RegExp(FAKE_TOKEN));
    }
    assert.match(runs[4].stderr, /did not answer.*just dev-start/);
    assert.match(runs[5].stderr, /usage: just mcp-call/);
  } finally {
    await server.close();
  }
});

test("the address is this machine's, whatever the arguments say", async () => {
  const run = await runNode([SCRIPT, "--port", "example.com:443", "echo"], { env });
  assert.notEqual(run.status, 0);
  assert.match(run.stderr, /is not a port/);
});
