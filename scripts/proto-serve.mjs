// `just proto-serve <name> <port>` (#602): serves one prototype folder to the
// LAN and tells the browser to keep nothing, so a phone always shows the
// latest build of a page.
//
// The port is the caller's to choose: which ports this host lets through its
// firewall is the host's business.

import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { ROOT, ToolError, folderOf, run } from "./proto-common.mjs";

const TYPES = {
  ".html": "text/html; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".mjs": "text/javascript; charset=utf-8",
  ".json": "application/json",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".jpg": "image/jpeg",
  ".jpeg": "image/jpeg",
  ".gif": "image/gif",
  ".webp": "image/webp",
  ".avif": "image/avif",
  ".ico": "image/x-icon",
  ".woff2": "font/woff2",
  ".woff": "font/woff",
  ".ttf": "font/ttf",
  ".txt": "text/plain; charset=utf-8",
};

// The file an address asks for, or nothing when it is not inside the folder.
// Checked again once symlinks are resolved, so a link placed in the folder
// cannot serve what lies outside it.
export function fileFor(folder, url) {
  let wanted;
  try {
    wanted = decodeURIComponent(new URL(url, "http://prototype").pathname);
  } catch {
    return undefined;
  }
  let file = path.join(folder, wanted);
  if (file !== folder && !file.startsWith(folder + path.sep)) {
    return undefined;
  }
  if (fs.existsSync(file) && fs.statSync(file).isDirectory()) {
    file = path.join(file, "index.html");
  }
  if (!fs.existsSync(file)) {
    return undefined;
  }
  const real = fs.realpathSync(file);
  const root = fs.realpathSync(folder);
  return real.startsWith(root + path.sep) ? real : undefined;
}

export function server(folder) {
  return http.createServer((request, response) => {
    const file = fileFor(folder, request.url ?? "/");
    response.setHeader("Cache-Control", "no-store");
    if (!file) {
      response.writeHead(404, { "Content-Type": "text/plain; charset=utf-8" });
      response.end("Nothing here.\n");
      return;
    }
    const type =
      TYPES[path.extname(file).toLowerCase()] ?? "application/octet-stream";
    const stream = fs.createReadStream(file);
    // A file removed or made unreadable mid-request ends that request, not
    // the server.
    stream.once("error", () => {
      if (!response.headersSent) {
        response.writeHead(500, { "Content-Type": "text/plain; charset=utf-8" });
      }
      response.end();
    });
    stream.once("open", () => {
      response.writeHead(200, { "Content-Type": type });
      stream.pipe(response);
    });
  });
}

// Every address another device might open this machine at. A host with
// Docker, a VPN or a bridge has several, and only the person holding the
// phone knows which network it is on, so all are printed.
function lanAddresses() {
  const found = [];
  for (const addresses of Object.values(os.networkInterfaces())) {
    for (const address of addresses ?? []) {
      if (address.family === "IPv4" && !address.internal) {
        found.push(address.address);
      }
    }
  }
  return found.length ? found : ["localhost"];
}

if (process.argv[1] === import.meta.filename) {
  await run(async () => {
    const [, , name, portText] = process.argv;
    const port = Number(portText);
    if (!name || !Number.isInteger(port) || port < 1 || port > 65535) {
      throw new ToolError("usage: just proto-serve <name> <port>");
    }
    const folder = folderOf(name);
    await new Promise((resolve, reject) => {
      server(folder)
        .once("error", (error) =>
          reject(
            error.code === "EADDRINUSE"
              ? new ToolError(`port ${port} is taken; choose another`)
              : error,
          ),
        )
        .listen(port, "0.0.0.0", () => {
          console.log(`serving ${path.relative(ROOT, folder)} with caching off`);
          for (const address of lanAddresses()) {
            console.log(`http://${address}:${port}/`);
          }
          resolve(undefined);
        });
    });
  });
}
