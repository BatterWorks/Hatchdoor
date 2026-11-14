// `just conformance` (#617): the MCP conformance run that
// `docs/maintenance/mcp-conformance-run.md` describes, as one command.
//
// It needs the dev server already up (`just dev-start`, then `just
// dev-wait`). It puts a proxy that adds the bearer token in front of `/mcp`,
// because the suite sends none, then restarts the backend in each write
// posture (write-enabled, then read-only) and runs the pinned suite once per
// protocol revision the build supports. Each run's results, with its console
// summary in `summary.txt`, land in
// `docs/maintenance/conformance-evidence/<date>-<posture>-<revision>/`.
// The backend is restarted once more at the end, on its usual settings.
//
// The written account of the run (`<date>-v<version>-release.md`) is still
// written by hand: whether a baseline entry is still justified is a judgment.
//
// Every address is built from `DEV_HOST`, so this cannot reach a deployed
// instance. The token is cut from everything printed and written.

import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import path from "node:path";

import {
  BACKEND_PORT,
  DEV_HOST,
  MCP_TOKEN_SETTING,
  devSetting,
  notAnswering,
  portFrom,
  runScrubbed,
  scrubbed,
  takeOption,
} from "./dev-common.mjs";
import { openSession } from "./mcp-call.mjs";
import { ROOT, ToolError } from "./proto-common.mjs";

// The pin is required: the unpinned package resolves to 0.1.16, which has no
// `--requirements` option (mcp-conformance-run.md, "Runner").
export const RUNNER = "@modelcontextprotocol/conformance@0.2.0-alpha.11";
export const PROXY_PORT = 42999;
export const BASELINE = "docs/maintenance/conformance-baseline.yml";
export const EVIDENCE_DIR = "docs/maintenance/conformance-evidence";
const PROTOCOL_CONFIG = "src/mcp/config.rs";

export const POSTURES = [
  { name: "write", label: "Write-enabled", write: true },
  { name: "ro", label: "Read-only", write: false },
];

// The revisions the build supports, from `SUPPORTED_PROTOCOL_VERSIONS`,
// oldest first, so the run follows the server without an edit here.
export function protocolRevisions(configSource) {
  const list = /SUPPORTED_PROTOCOL_VERSIONS[^=]*=\s*&\[([^\]]*)\]/.exec(
    configSource,
  );
  const revisions = list
    ? [...list[1].matchAll(/"(\d{4}-\d{2}-\d{2})"/g)].map((match) => match[1])
    : [];
  if (revisions.length === 0) {
    throw new ToolError(
      `found no SUPPORTED_PROTOCOL_VERSIONS list in ${PROTOCOL_CONFIG}`,
    );
  }
  return revisions.sort();
}

// `2026-10-27`, `write`, `2025-11-25` -> `.../2026-10-27-write-1125`, the
// names the recorded runs use.
export function evidenceDir(date, posture, revision) {
  return path.join(
    EVIDENCE_DIR,
    `${date}-${posture}-${revision.slice(5).replace("-", "")}`,
  );
}

export function today(now = new Date()) {
  const pad = (value) => String(value).padStart(2, "0");
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
}

export function withoutColour(text) {
  return text.replace(/\u001b\[[0-9;]*m/g, "");
}

// Forwards every request to `target` with the bearer token added. Resolves
// once it listens on 127.0.0.1:`port`.
export function startProxy({ target, token, port }) {
  const upstream = new URL(target);
  const server = http.createServer(async (request, response) => {
    const chunks = [];
    for await (const chunk of request) {
      chunks.push(chunk);
    }
    const proxied = http.request(
      {
        hostname: upstream.hostname,
        port: upstream.port,
        path: request.url,
        method: request.method,
        headers: {
          ...request.headers,
          host: upstream.host,
          authorization: `Bearer ${token}`,
        },
      },
      (answer) => {
        response.writeHead(answer.statusCode, answer.headers);
        answer.pipe(response);
      },
    );
    proxied.on("error", (error) => {
      if (response.headersSent) {
        response.destroy(error);
      } else {
        response.writeHead(502).end(String(error));
      }
    });
    proxied.end(Buffer.concat(chunks));
  });
  return new Promise((resolve, reject) => {
    server.once("error", (error) =>
      reject(
        new ToolError(
          error.code === "EADDRINUSE"
            ? `port ${port} is taken, and the proxy needs it; stop what holds it (\`fuser ${port}/tcp\` names it)`
            : `the proxy could not start: ${error.message}`,
        ),
      ),
    );
    server.listen(port, DEV_HOST, () =>
      resolve({
        close: () =>
          new Promise((done) => {
            server.close(done);
            server.closeAllConnections();
          }),
      }),
    );
  });
}

async function answers(url) {
  try {
    return (await fetch(url, { signal: AbortSignal.timeout(5_000) })).ok;
  } catch {
    return false;
  }
}

function restartBackend(settings) {
  const result = spawnSync("just", ["dev-restart-backend"], {
    cwd: ROOT,
    env: { ...process.env, ...settings },
    stdio: "inherit",
  });
  if (result.error || result.status !== 0) {
    throw new ToolError(
      "`just dev-restart-backend` failed; .dev/backend.log says why",
    );
  }
}

// Never throws, so a failure here cannot hide the one that ended the run.
function restoreBackend() {
  try {
    restartBackend({});
  } catch (error) {
    console.error(
      `warning: ${error.message}. The backend may still run in a test posture; run \`just dev-restart-backend\`.`,
    );
  }
}

// Runs one command, its output shown as it comes and kept, colour removed,
// with the token cut.
function runShown(command, args, secrets) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { cwd: ROOT });
    let output = "";
    const keep = (stream) => (chunk) => {
      const text = scrubbed(String(chunk), secrets);
      output += text;
      stream.write(text);
    };
    child.stdout.on("data", keep(process.stdout));
    child.stderr.on("data", keep(process.stderr));
    child.on("error", reject);
    child.on("close", (status) =>
      resolve({ status, output: withoutColour(output) }),
    );
  });
}

async function main() {
  const args = process.argv.slice(2);
  const port = portFrom(takeOption(args, "--port"), BACKEND_PORT);
  if (args.length) {
    throw new ToolError("usage: just conformance");
  }
  const health = `http://${DEV_HOST}:${port}/health`;
  if (!(await answers(health))) {
    throw new ToolError(notAnswering("dev server", health));
  }
  const token = devSetting(MCP_TOKEN_SETTING);
  if (!token) {
    throw new ToolError(
      `${MCP_TOKEN_SETTING} is not set in the environment or .env, and /mcp refuses every request without it`,
    );
  }

  const date = today();
  const revisions = protocolRevisions(
    fs.readFileSync(path.join(ROOT, PROTOCOL_CONFIG), "utf8"),
  );
  const runs = POSTURES.flatMap((posture) =>
    revisions.map((revision) => ({
      posture,
      revision,
      dir: evidenceDir(date, posture.name, revision),
    })),
  );
  const taken = runs.filter(({ dir }) => fs.existsSync(path.join(ROOT, dir)));
  if (taken.length) {
    throw new ToolError(
      `evidence from today is already there, and a run never overwrites evidence: ${taken.map(({ dir }) => dir).join(", ")}. Move or delete it first.`,
    );
  }

  const version = spawnSync("npx", ["-y", RUNNER, "--version"], {
    cwd: ROOT,
    encoding: "utf8",
  });
  if (version.status !== 0) {
    throw new ToolError(
      `\`npx -y ${RUNNER} --version\` failed: ${(version.stderr || version.stdout).trim()}`,
    );
  }
  const runnerVersion = version.stdout.trim();
  console.log(`Runner: ${RUNNER} (--version: ${runnerVersion})`);

  const proxy = await startProxy({
    target: `http://${DEV_HOST}:${port}`,
    token,
    port: PROXY_PORT,
  });
  const tools = {};
  // Ctrl+C must not leave the backend in a test posture.
  const interrupted = () => {
    console.error("\nInterrupted: restarting the backend on its usual settings.");
    restoreBackend();
    process.exit(130);
  };
  process.once("SIGINT", interrupted);
  try {
    for (const posture of POSTURES) {
      console.log(
        `\n${posture.label}: restarting the backend with HATCHDOOR_MCP_WRITE_ENABLED=${posture.write}.`,
      );
      restartBackend({
        HATCHDOOR_MCP_ENABLED: "true",
        HATCHDOOR_MCP_WRITE_ENABLED: String(posture.write),
      });
      const session = await openSession({ port, token });
      try {
        tools[posture.name] = (await session.tools()).length;
      } finally {
        await session.close();
      }
      console.log(`${posture.label}: the server lists ${tools[posture.name]} tools.`);

      for (const run of runs.filter((each) => each.posture === posture)) {
        const command = [
          "-y",
          RUNNER,
          "server",
          "--url",
          `http://${DEV_HOST}:${PROXY_PORT}/mcp`,
          "--expected-failures",
          BASELINE,
          "--requirements",
          run.revision,
          "--output-dir",
          run.dir,
        ];
        console.log(`\n$ npx ${command.join(" ")}`);
        const result = await runShown("npx", command, [token]);
        fs.mkdirSync(path.join(ROOT, run.dir), { recursive: true });
        fs.writeFileSync(path.join(ROOT, run.dir, "summary.txt"), result.output);
        run.status = result.status;
      }
    }
  } finally {
    process.removeListener("SIGINT", interrupted);
    await proxy.close();
    console.log("\nRestarting the backend on its usual settings.");
    restoreBackend();
  }

  console.log(`\nRunner ${RUNNER}, --version ${runnerVersion}, ${date}.`);
  console.log(
    `Tools listed: ${tools.write} write-enabled, ${tools.ro} read-only.`,
  );
  for (const run of runs) {
    console.log(
      `${run.posture.label.padEnd(13)} ${run.revision}  exit ${run.status}  ${run.dir}/`,
    );
  }
  const failed = runs.filter((run) => run.status !== 0);
  if (failed.length) {
    throw new ToolError(
      `${failed.length} of ${runs.length} runs exited non-zero, so this is not release evidence; each summary.txt says which scenarios failed`,
    );
  }
  if (!(tools.write > tools.ro)) {
    throw new ToolError(
      `the write-enabled server listed ${tools.write} tools and the read-only one ${tools.ro}, so the postures did not differ; check HATCHDOOR_MCP_WRITE_ENABLED handling`,
    );
  }
  console.log(
    "\nAll runs exited 0. Write the account of the run beside the directories, as the earlier `*-release.md` files there do.",
  );
}

if (process.argv[1] === import.meta.filename) {
  await runScrubbed(main, () => [devSetting(MCP_TOKEN_SETTING)]);
}
