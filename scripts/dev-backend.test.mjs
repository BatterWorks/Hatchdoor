// `just dev-stop-backend` and the stop half of `just dev-restart-backend`
// (#616), run as the real recipes against stand-in processes: the justfile's
// own variables point them at a scratch directory and at free ports.

import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import fs from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { test } from "node:test";

import { freePort, runTo } from "./dev-test-support.mjs";
import { ROOT } from "./proto-common.mjs";

// A stand-in for one half of the dev server: it listens on a port and keeps
// a child, as `cargo run` keeps the binary, in a process group of its own.
const STAND_IN = `
const http = require("node:http");
const { spawn } = require("node:child_process");
const [port, stubborn] = process.argv.slice(2);
const child = spawn(process.execPath, ["-e", stubborn ? "process.on('SIGTERM', () => {}); setInterval(() => {}, 1000)" : "setInterval(() => {}, 1000)"], { stdio: "ignore" });
if (stubborn) process.on("SIGTERM", () => {});
http.createServer((_, response) => response.end("ok")).listen(Number(port), "127.0.0.1", () => console.log("up " + child.pid));
`;

async function standIn(folder, name, port, { stubborn = false } = {}) {
  const script = path.join(folder, "stand-in.cjs");
  fs.writeFileSync(script, STAND_IN);
  const child = spawn(
    process.execPath,
    [script, String(port), ...(stubborn ? ["stubborn"] : [])],
    { detached: true, stdio: ["ignore", "pipe", "ignore"] },
  );
  const exited = new Promise((resolve) => child.on("exit", resolve));
  const line = await new Promise((resolve) =>
    child.stdout.once("data", (chunk) => resolve(String(chunk))),
  );
  fs.writeFileSync(path.join(folder, `${name}.pid`), String(child.pid));
  return { pid: child.pid, childPid: Number(line.split(" ")[1]), exited };
}

function alive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

async function answers(port) {
  try {
    return (await fetch(`http://127.0.0.1:${port}/`)).ok;
  } catch {
    return false;
  }
}

async function scene(t) {
  const folder = fs.mkdtempSync(path.join(tmpdir(), "dev-backend-test-"));
  const ports = { backend: await freePort(), frontend: await freePort() };
  const started = [];
  t.after(() => {
    for (const { pid } of started) {
      try {
        process.kill(-pid, "SIGKILL");
      } catch {
        // Already gone, as it should be.
      }
    }
    fs.rmSync(folder, { recursive: true, force: true });
  });
  return {
    folder,
    ports,
    async start(name, options) {
      const process = await standIn(folder, name, ports[name], options);
      started.push(process);
      return process;
    },
    just: (...recipe) =>
      runTo("just", [
        "--justfile",
        path.join(ROOT, "justfile"),
        "--set",
        "dev_dir",
        folder,
        "--set",
        "backend_port",
        String(ports.backend),
        "--set",
        "frontend_port",
        String(ports.frontend),
        // Nothing to measure, so `dev-status` prints the two halves alone.
        "--set",
        "cargo_target_dir",
        path.join(folder, "no-target"),
        ...recipe,
      ]),
  };
}

test("dev-stop-backend stops the backend's whole group and leaves the frontend", async (t) => {
  const { folder, ports, start, just } = await scene(t);
  const backend = await start("backend");
  const frontend = await start("frontend");

  const before = await just("dev-status");
  assert.match(before.stdout, /^backend: running \(pid \d+/m);
  assert.match(before.stdout, /^frontend: running/m);

  const stop = await just("dev-stop-backend");
  assert.equal(stop.status, 0, stop.stderr);
  assert.match(stop.stdout, /backend stopped/);
  await backend.exited;

  assert.equal(alive(backend.pid), false);
  assert.equal(alive(backend.childPid), false, "the child went with it");
  assert.equal(await answers(ports.backend), false, "nothing is left on the port");
  assert.equal(fs.existsSync(path.join(folder, "backend.pid")), false);
  assert.equal(alive(frontend.pid), true);
  assert.equal(await answers(ports.frontend), true);

  const after = await just("dev-status");
  assert.match(after.stdout, /^backend: not running$/m);
  assert.match(after.stdout, /^frontend: running/m);

  // Stopping what is already stopped is not an error.
  assert.equal((await just("dev-stop-backend")).status, 0);
});

test("a backend that ignores TERM is killed, and dev-status still tells the truth", async (t) => {
  const { folder, ports, start, just } = await scene(t);
  const backend = await start("backend", { stubborn: true });

  const stop = await just("dev-stop-backend");
  assert.equal(stop.status, 0, stop.stderr);
  assert.match(stop.stdout, /ignored TERM for five seconds, killing it/);
  await backend.exited;

  assert.equal(alive(backend.pid), false);
  assert.equal(alive(backend.childPid), false);
  assert.equal(await answers(ports.backend), false);
  assert.equal(fs.existsSync(path.join(folder, "backend.pid")), false);
  assert.match((await just("dev-status")).stdout, /^backend: not running$/m);
});

test("a server started by hand on the backend port is stopped too", async (t) => {
  const { folder, ports, start, just } = await scene(t);
  const stray = await start("backend");
  // Untracked: on the port, with no pid file to find it by.
  fs.rmSync(path.join(folder, "backend.pid"));

  assert.equal((await just("dev-stop-backend")).status, 0);
  await stray.exited;
  assert.equal(await answers(ports.backend), false);
});

test("dev-restart-backend refuses without dev Vaults, before it stops or starts anything", async (t) => {
  const { folder, ports, start, just } = await scene(t);
  const backend = await start("backend");
  const restart = await just("dev-restart-backend");
  assert.equal(restart.status, 1);
  assert.match(restart.stderr, /no dev Vault fixtures yet; 'just dev-start'/);
  assert.equal(alive(backend.pid), true);
  assert.equal(await answers(ports.backend), true);
  assert.equal(fs.existsSync(path.join(folder, "backend.pid")), true);
});
