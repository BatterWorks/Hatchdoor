// `just mcp-call <tool> '<json arguments>'` (#616): one tool call on the local
// dev server's `/mcp`, printed as the tool's own JSON.
//
// The handshake lives here so a live acceptance pass never writes it again:
// `initialize`, `notifications/initialized`, the `mcp-session-id` the server
// hands back on every later request, the `Origin` header, and the answer read
// off the `data:` lines of the event stream.
//
// What is printed is the tool's result as the tool returns it. A collection
// read keeps its `scope`, `partial` and `participants`, with the rows under
// `data`. A failure prints the server's message and exits non-zero.
//
// Several calls in one session:
//
// - `just mcp-call -` reads one call a line from standard input, each written
//   `<tool> <json arguments>`, and prints one line of JSON for each.
// - A script imports `openSession` and calls `session.call(tool, arguments)`
//   as often as it needs, reading one answer before it makes the next call.
//
// The address is built from `DEV_HOST` alone, so this cannot reach a deployed
// instance. The bearer token is read from the environment or `.env` and is
// cut from everything printed.

import {
  BACKEND_PORT,
  DEV_HOST,
  MCP_TOKEN_SETTING,
  devSetting,
  notAnswering,
  portFrom,
  runScrubbed,
  scrubbed,
  standardInput,
  takeOption,
} from "./dev-common.mjs";
import { ToolError } from "./proto-common.mjs";

// The newest version that still opens with `initialize`
// (`SUPPORTED_PROTOCOL_VERSIONS` in `src/mcp/config.rs`).
const PROTOCOL_VERSION = "2025-11-25";
const USAGE =
  "usage: just mcp-call <tool> '<json arguments>'   as in: just mcp-call list_vaults\n" +
  "       just mcp-call -                            one `<tool> <json arguments>` a line on standard input";

// The JSON-RPC messages in a reply, whether it came as one JSON body or as an
// event stream. The stream opens with a `data:` line holding nothing.
export function messagesIn(contentType, body) {
  if (!(contentType ?? "").includes("text/event-stream")) {
    return body.trim() ? [JSON.parse(body)] : [];
  }
  const messages = [];
  for (const event of body.split(/\r?\n\r?\n/)) {
    const data = event
      .split(/\r?\n/)
      .filter((line) => line.startsWith("data:"))
      .map((line) => line.slice(5).replace(/^ /, ""))
      .join("\n");
    if (data.trim()) {
      messages.push(JSON.parse(data));
    }
  }
  return messages;
}

function textOf(result) {
  return (result.content ?? [])
    .filter((block) => block.type === "text")
    .map((block) => block.text)
    .join("\n");
}

// What a tool returned, out of the envelope MCP wraps it in: the structured
// result when there is one, the text parsed as JSON when it is JSON, and the
// bare text otherwise.
export function payloadOf(result) {
  if (result.structuredContent !== undefined) {
    return result.structuredContent;
  }
  const text = textOf(result);
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}

// A session on the local dev server. `call` returns a tool's result and
// throws a `ToolError` carrying the server's message when the tool fails.
export async function openSession({
  port = BACKEND_PORT,
  token = devSetting(MCP_TOKEN_SETTING),
} = {}) {
  const url = `http://${DEV_HOST}:${port}/mcp`;
  const clean = (text) => scrubbed(text, [token]);
  let sessionId;
  let protocolVersion;
  let nextId = 1;

  async function send(message) {
    let response;
    try {
      response = await fetch(url, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Accept: "application/json, text/event-stream",
          Origin: `http://${DEV_HOST}`,
          ...(token ? { Authorization: `Bearer ${token}` } : {}),
          ...(sessionId ? { "Mcp-Session-Id": sessionId } : {}),
          ...(protocolVersion
            ? { "MCP-Protocol-Version": protocolVersion }
            : {}),
        },
        body: JSON.stringify(message),
      });
    } catch {
      throw new ToolError(notAnswering("dev server", url));
    }
    const body = await response.text();
    if (!response.ok) {
      throw new ToolError(
        clean(`the dev server answered ${response.status}: ${body.trim()}`),
      );
    }
    sessionId = response.headers.get("mcp-session-id") ?? sessionId;
    if (message.id === undefined) {
      return undefined;
    }
    let messages;
    try {
      messages = messagesIn(response.headers.get("content-type"), body);
    } catch {
      throw new ToolError(
        clean(`the dev server's reply was not JSON: ${body.trim()}`),
      );
    }
    const answer = messages.find((candidate) => candidate.id === message.id);
    if (!answer) {
      throw new ToolError(`the dev server sent no answer to ${message.method}`);
    }
    if (answer.error) {
      const detail =
        answer.error.data === undefined
          ? ""
          : ` ${JSON.stringify(answer.error.data)}`;
      throw new ToolError(clean(`${answer.error.message}${detail}`));
    }
    return answer.result;
  }

  const hello = await send({
    jsonrpc: "2.0",
    id: nextId++,
    method: "initialize",
    params: {
      protocolVersion: PROTOCOL_VERSION,
      capabilities: {},
      clientInfo: { name: "hatchdoor-dev-mcp-call", version: "1" },
    },
  });
  protocolVersion = hello.protocolVersion;
  await send({ jsonrpc: "2.0", method: "notifications/initialized" });

  return {
    async call(tool, args = {}) {
      const result = await send({
        jsonrpc: "2.0",
        id: nextId++,
        method: "tools/call",
        params: { name: tool, arguments: args },
      });
      if (result.isError) {
        const payload = payloadOf(result);
        throw new ToolError(
          clean(
            typeof payload === "string" ? payload : JSON.stringify(payload),
          ),
        );
      }
      return payloadOf(result);
    },
    // Best effort: the server drops an idle session by itself.
    async close() {
      if (!sessionId) {
        return;
      }
      try {
        await fetch(url, {
          method: "DELETE",
          headers: {
            Origin: `http://${DEV_HOST}`,
            "Mcp-Session-Id": sessionId,
            ...(token ? { Authorization: `Bearer ${token}` } : {}),
          },
        });
      } catch {
        // Nothing to tidy when the server has gone.
      }
    },
  };
}

// The arguments a call was given, as an object.
export function argumentsFrom(text) {
  const source = (text ?? "").trim() || "{}";
  let parsed;
  try {
    parsed = JSON.parse(source);
  } catch (error) {
    throw new ToolError(`the arguments are not JSON (${error.message})`);
  }
  if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new ToolError('the arguments are a JSON object, as in {"scope":"all"}');
  }
  return parsed;
}

// The calls on standard input: `<tool> <json arguments>` a line, with blank
// lines and `#` comments left out. Each keeps its line number for an error.
export function callsFrom(text) {
  const calls = [];
  text.split("\n").forEach((raw, index) => {
    const line = raw.trim();
    if (!line || line.startsWith("#")) {
      return;
    }
    const [, tool, rest] = /^(\S+)\s*(.*)$/.exec(line);
    let args;
    try {
      args = argumentsFrom(rest);
    } catch (error) {
      throw new ToolError(`line ${index + 1}: ${error.message}`);
    }
    calls.push({ line: index + 1, tool, args });
  });
  return calls;
}

async function main() {
  const args = process.argv.slice(2);
  const port = portFrom(takeOption(args, "--port"), BACKEND_PORT);
  const [tool, json, ...extra] = args;
  if (!tool || extra.length) {
    throw new ToolError(USAGE);
  }
  const token = devSetting(MCP_TOKEN_SETTING);
  const print = (text) => console.log(scrubbed(text, [token]));

  const batch = tool === "-";
  const calls = batch
    ? callsFrom(String(await standardInput()))
    : [{ tool, args: argumentsFrom(json) }];
  const session = await openSession({ port, token });
  try {
    for (const call of calls) {
      let payload;
      try {
        payload = await session.call(call.tool, call.args);
      } catch (error) {
        if (batch && error instanceof ToolError) {
          throw new ToolError(`line ${call.line} (${call.tool}): ${error.message}`);
        }
        throw error;
      }
      print(JSON.stringify(payload, null, batch ? 0 : 2));
    }
  } finally {
    await session.close();
  }
}

if (process.argv[1] === import.meta.filename) {
  await runScrubbed(main, () => [devSetting(MCP_TOKEN_SETTING)]);
}
