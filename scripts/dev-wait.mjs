// `just dev-wait [seconds]` (#616): returns once the dev backend's health
// endpoint and the dev frontend both answer, so a live acceptance pass has no
// wait loop of its own. `just dev-start` returns while the backend is still
// compiling.
//
// It fails, naming the half that did not come, when the time limit runs out,
// and at once when that half is not running at all: a backend that was
// stopped, or one whose build failed, will never answer.

import fs from "node:fs";
import http from "node:http";
import net from "node:net";
import path from "node:path";
import {
  BACKEND_PORT,
  DEV_DIR,
  DEV_HOST,
  FRONTEND_PORT,
  portFrom,
  sleep,
  takeOption,
} from "./dev-common.mjs";
import { ToolError, run } from "./proto-common.mjs";

const DEFAULT_SECONDS = 600;
const USAGE =
  "usage: just dev-wait [seconds]   the time limit, 600 when left out";

// The halves of the dev server and the address each answers on. The backend's
// is `/health`, not `/api/health`.
export function halvesFor({
  backendPort = BACKEND_PORT,
  frontendPort = FRONTEND_PORT,
} = {}) {
  return {
    backend: `http://${DEV_HOST}:${backendPort}/health`,
    frontend: `http://${DEV_HOST}:${frontendPort}/`,
  };
}

// A connection that came back to its own socket (#749). The dev backend's
// port sits inside Linux's range of source ports, so while the backend is
// still compiling, a probe to it can be handed that same port as its source
// and connect to itself. Open, or closed normally and then in TIME-WAIT for
// about a minute, such a connection keeps the backend from binding the port.
//
// So the probe's connection is checked before anything is sent, and one that
// reached itself is reset, which frees the port at once. `localPort` is for
// the test that forces it.
export function probeConnection({ host, port, localPort }) {
  const socket = net.connect({ host, port, localPort });
  socket.once("connect", () => {
    if (
      socket.localPort === socket.remotePort &&
      socket.localAddress === socket.remoteAddress
    ) {
      socket.resetAndDestroy();
    }
  });
  return socket;
}

// Whether `url` answers with a 2xx. A connection that reached itself counts
// as not answering.
export function answers(url, { localPort } = {}) {
  const parsed = new URL(url);
  const host = parsed.hostname;
  // A URL leaves out the scheme's default port, which `net.connect` refuses.
  const port = Number(parsed.port) || 80;
  return new Promise((resolve) => {
    const request = http.get(
      {
        host,
        port,
        path: parsed.pathname,
        agent: false,
        createConnection: () => probeConnection({ host, port, localPort }),
      },
      (response) => {
        response.on("error", () => {});
        response.resume();
        resolve(response.statusCode >= 200 && response.statusCode < 300);
      },
    );
    // Five seconds for the whole request, not for each pause in it.
    const limit = setTimeout(() => request.destroy(), 5_000);
    request.on("close", () => clearTimeout(limit));
    request.on("error", () => resolve(false));
  });
}

// Whether the process `just dev-start` tracks for a half is alive: "running",
// "stopped" when its pid file names a process that has gone, and "untracked"
// when there is no pid file.
export function trackedState(name, devDir = DEV_DIR) {
  let pid;
  try {
    pid = Number(fs.readFileSync(path.join(devDir, `${name}.pid`), "utf8"));
  } catch {
    return "untracked";
  }
  try {
    process.kill(pid, 0);
    return "running";
  } catch (error) {
    // EPERM is a live process owned by someone else.
    return error.code === "EPERM" ? "running" : "stopped";
  }
}

// A half's log, as a path to read from where the command was run.
function logOf(name, devDir) {
  return path.join(path.relative(process.cwd(), devDir) || ".", `${name}.log`);
}

function whyNot(name, state, devDir) {
  const log = logOf(name, devDir);
  const start =
    name === "backend"
      ? "`just dev-restart-backend` starts it alone, `just dev-start` starts both"
      : "`just dev-start` starts it";
  if (state === "untracked") {
    return `the ${name} is not running; ${start}`;
  }
  return `the ${name} stopped before it answered; ${log} says why, and ${start}`;
}

// Wait for every half in `names`. Resolves with the seconds it took.
export async function waitFor({
  names = ["backend", "frontend"],
  seconds = DEFAULT_SECONDS,
  ports = {},
  devDir = DEV_DIR,
  intervalMs = 500,
} = {}) {
  const halves = halvesFor(ports);
  const started = Date.now();
  const deadline = started + seconds * 1000;
  let waiting = [...names];
  for (;;) {
    const up = await Promise.all(waiting.map((name) => answers(halves[name])));
    waiting = waiting.filter((_, index) => !up[index]);
    if (waiting.length === 0) {
      return Math.round((Date.now() - started) / 1000);
    }
    // Asked after the address, so a server started by hand still counts.
    const dead = waiting
      .map((name) => [name, trackedState(name, devDir)])
      .filter(([, state]) => state !== "running");
    if (dead.length) {
      throw new ToolError(
        dead.map(([name, state]) => whyNot(name, state, devDir)).join("\n"),
      );
    }
    if (Date.now() >= deadline) {
      throw new ToolError(
        waiting
          .map(
            (name) =>
              `the ${name} did not answer at ${halves[name]} within ${seconds} seconds; ` +
              `${logOf(name, devDir)} shows what it is doing`,
          )
          .join("\n"),
      );
    }
    await sleep(intervalMs);
  }
}

async function main() {
  const args = process.argv.slice(2);
  const only = takeOption(args, "--only");
  const ports = {
    backendPort: portFrom(takeOption(args, "--backend-port"), BACKEND_PORT),
    frontendPort: portFrom(takeOption(args, "--frontend-port"), FRONTEND_PORT),
  };
  const devDir = takeOption(args, "--dev-dir") ?? DEV_DIR;
  const [limit, ...extra] = args.filter((arg) => arg !== "");
  const seconds = limit === undefined ? DEFAULT_SECONDS : Number(limit);
  if (
    extra.length ||
    !Number.isFinite(seconds) ||
    seconds <= 0 ||
    (only !== undefined && !(only in halvesFor()))
  ) {
    throw new ToolError(USAGE);
  }
  const names = only ? [only] : ["backend", "frontend"];
  console.log(
    `waiting for the ${names.join(" and the ")}, ${seconds} seconds at most...`,
  );
  const took = await waitFor({ names, seconds, ports, devDir });
  console.log(
    `the ${names.join(" and the ")} ${names.length > 1 ? "answer" : "answers"} (${took} s)`,
  );
}

if (process.argv[1] === import.meta.filename) {
  await run(main);
}
