import assert from "node:assert/strict";
import fs from "node:fs";
import { tmpdir } from "node:os";
import net from "node:net";
import path from "node:path";
import { test } from "node:test";

import { freePort, runNode, runTo, serve } from "./dev-test-support.mjs";
import {
  answers,
  halvesFor,
  probeConnection,
  trackedState,
  waitFor,
} from "./dev-wait.mjs";

const SCRIPT = path.join(import.meta.dirname, "dev-wait.mjs");

function scratch() {
  return fs.mkdtempSync(path.join(tmpdir(), "dev-wait-test-"));
}

// A pid that names no process: one that has just ended.
async function deadPid() {
  const { spawn } = await import("node:child_process");
  const child = spawn(process.execPath, ["-e", ""]);
  await new Promise((resolve) => child.on("exit", resolve));
  return child.pid;
}

test("the backend is asked at /health and the frontend at its root", () => {
  assert.deepEqual(halvesFor({ backendPort: 1, frontendPort: 2 }), {
    backend: "http://127.0.0.1:1/health",
    frontend: "http://127.0.0.1:2/",
  });
});

test("it returns once both halves answer, a late one included", async () => {
  const devDir = scratch();
  let healthy = false;
  const asked = [];
  const backend = await serve((request, response) => {
    asked.push(request.url);
    response.writeHead(healthy && request.url === "/health" ? 200 : 503).end();
  });
  const frontend = await serve((_, response) => response.writeHead(200).end());
  // Compiling: the tracked process is alive but does not answer yet.
  fs.writeFileSync(path.join(devDir, "backend.pid"), String(process.pid));
  setTimeout(() => (healthy = true), 300);
  try {
    await waitFor({
      seconds: 10,
      ports: { backendPort: backend.port, frontendPort: frontend.port },
      devDir,
      intervalMs: 50,
    });
    assert.ok(healthy);
    assert.deepEqual([...new Set(asked)], ["/health"]);
  } finally {
    await backend.close();
    await frontend.close();
    fs.rmSync(devDir, { recursive: true, force: true });
  }
});

test("a stopped backend fails at once and is named", async () => {
  const devDir = scratch();
  const frontend = await serve((_, response) => response.writeHead(200).end());
  const backendPort = await freePort();
  const args = [
    SCRIPT,
    "--backend-port",
    String(backendPort),
    "--frontend-port",
    String(frontend.port),
    "--dev-dir",
    devDir,
    "30",
  ];
  try {
    // `just dev-stop-backend` removes the pid file.
    const started = Date.now();
    const stopped = await runNode(args);
    assert.equal(stopped.status, 1);
    assert.match(stopped.stderr, /the backend is not running/);
    assert.match(stopped.stderr, /just dev-restart-backend/);
    assert.doesNotMatch(stopped.stderr, /frontend/);
    assert.ok(Date.now() - started < 10_000, "it did not wait the limit out");

    // A build that failed leaves a pid file naming a process that has gone.
    fs.writeFileSync(path.join(devDir, "backend.pid"), String(await deadPid()));
    assert.equal(trackedState("backend", devDir), "stopped");
    const failed = await runNode(args);
    assert.equal(failed.status, 1);
    assert.match(failed.stderr, /the backend stopped before it answered/);
    assert.match(failed.stderr, /backend\.log/);
  } finally {
    await frontend.close();
    fs.rmSync(devDir, { recursive: true, force: true });
  }
});

test("the time limit names the half that did not answer", async () => {
  const devDir = scratch();
  const backend = await serve((_, response) => response.writeHead(200).end());
  const frontendPort = await freePort();
  fs.writeFileSync(path.join(devDir, "frontend.pid"), String(process.pid));
  try {
    const run = await runNode([
      SCRIPT,
      "--backend-port",
      String(backend.port),
      "--frontend-port",
      String(frontendPort),
      "--dev-dir",
      devDir,
      "1",
    ]);
    assert.equal(run.status, 1);
    assert.match(run.stderr, /the frontend did not answer .* within 1 seconds/);
    assert.doesNotMatch(run.stderr, /backend/);

    const only = await runNode([
      SCRIPT,
      "--only",
      "backend",
      "--backend-port",
      String(backend.port),
      "--frontend-port",
      String(frontendPort),
      "--dev-dir",
      devDir,
      "5",
    ]);
    assert.equal(only.status, 0, only.stderr);
    assert.match(only.stdout, /the backend answers/);
  } finally {
    await backend.close();
    fs.rmSync(devDir, { recursive: true, force: true });
  }
});

test("a time limit that is no number is refused", async () => {
  const run = await runNode([SCRIPT, "soon"]);
  assert.equal(run.status, 1);
  assert.match(run.stderr, /usage: just dev-wait/);
});

// #749: while nothing listens on a port, Linux may hand a connection to it
// that same port as its source, and the connection then connects to itself.
// A probe's source port carries no address reuse, so while that connection
// is open, and for about a minute in TIME-WAIT once it is closed normally,
// the backend that finishes compiling cannot bind the port.

// Whether no socket at all holds `port`, TIME-WAIT included: a bind without
// address reuse, which Node cannot make, so Python makes it. Undefined when
// there is no Python.
async function strictlyFree(port) {
  const run = await runTo("python3", [
    "-c",
    "import socket,sys; socket.socket().bind(('127.0.0.1', int(sys.argv[1])))",
    String(port),
  ]);
  if (run.status === "ENOENT") {
    return undefined;
  }
  return run.status === 0;
}

// Close a socket on `port` connected to `port` the normal way. False where
// the system refuses to connect a socket to itself.
function selfConnectedAndClosed(port) {
  return new Promise((resolve) => {
    const socket = net.connect({ host: "127.0.0.1", port, localPort: port });
    socket.once("connect", () => {
      const self = socket.localPort === socket.remotePort;
      socket.end().once("close", () => resolve(self));
    });
    socket.once("error", () => resolve(false));
  });
}

test("a probe that connects to itself leaves the port free", async (t) => {
  const control = await freePort();
  if ((await strictlyFree(control)) === undefined) {
    t.skip("there is no python3 to bind without address reuse");
    return;
  }
  if (!(await selfConnectedAndClosed(control))) {
    t.skip("this system does not connect a socket to itself");
    return;
  }
  // The control: closed the normal way, the connection still holds the port.
  assert.equal(await strictlyFree(control), false);

  // The probe's own connection, on its own: reset at once, holding nothing.
  const own = await freePort();
  const socket = probeConnection({
    host: "127.0.0.1",
    port: own,
    localPort: own,
  });
  socket.on("error", () => {});
  const closed = await new Promise((resolve) => {
    const timer = setTimeout(() => resolve(false), 2_000);
    socket.once("close", () => {
      clearTimeout(timer);
      resolve(true);
    });
  });
  socket.destroy();
  assert.ok(closed, "the probe left the connection to itself open");
  assert.equal(await strictlyFree(own), true, "nothing holds the port");

  // And through the whole HTTP probe.
  const port = await freePort();
  const up = await answers(`http://127.0.0.1:${port}/health`, {
    localPort: port,
  });
  assert.equal(up, false);
  assert.equal(await strictlyFree(port), true, "nothing holds the port");
});

test("a probe answers true only for a 2xx", async () => {
  const healthy = await serve((_, response) => response.writeHead(200).end());
  const failing = await serve((_, response) => response.writeHead(503).end());
  try {
    assert.equal(
      await answers(`http://127.0.0.1:${healthy.port}/health`),
      true,
    );
    assert.equal(
      await answers(`http://127.0.0.1:${failing.port}/health`),
      false,
    );
    assert.equal(await answers(`http://127.0.0.1:${await freePort()}/`), false);
  } finally {
    await healthy.close();
    await failing.close();
  }
});
