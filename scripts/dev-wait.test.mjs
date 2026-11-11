import assert from "node:assert/strict";
import fs from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { test } from "node:test";

import { freePort, runNode, serve } from "./dev-test-support.mjs";
import { halvesFor, trackedState, waitFor } from "./dev-wait.mjs";

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
