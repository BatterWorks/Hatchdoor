// What the tests of the live-acceptance helpers share (#616): running a
// helper as its own process while a fake server answers it from this one.

import { execFile } from "node:child_process";
import http from "node:http";

// A token no real server has, so finding it in output is always a leak.
export const FAKE_TOKEN = "fake-token-3f9c1e7a5b2d";

// Run a command to its end without blocking this process, which a fake
// server may be answering from. Never rejects: the status tells.
export function runTo(command, args, { env = {}, input, cwd } = {}) {
  return new Promise((resolve) => {
    const child = execFile(
      command,
      args,
      { env: { ...process.env, ...env }, cwd, encoding: "utf8" },
      (error, stdout, stderr) => {
        resolve({ status: error ? (error.code ?? 1) : 0, stdout, stderr });
      },
    );
    child.stdin.end(input ?? "");
  });
}

export function runNode(args, options) {
  return runTo(process.execPath, args, options);
}

// Serve `handler` on a free port of 127.0.0.1. Resolves with the port and a
// `close` that also drops connections kept open.
export function serve(handler) {
  return new Promise((resolve) => {
    const server = http.createServer(handler);
    server.listen(0, "127.0.0.1", () => {
      resolve({
        port: server.address().port,
        close: () =>
          new Promise((done) => {
            server.close(done);
            server.closeAllConnections();
          }),
      });
    });
  });
}

// A port nothing listens on: one that was free a moment ago.
export async function freePort() {
  const server = await serve(() => {});
  await server.close();
  return server.port;
}

export function bodyOf(request) {
  return new Promise((resolve) => {
    let text = "";
    request.on("data", (chunk) => (text += chunk));
    request.on("end", () => resolve(text));
  });
}
