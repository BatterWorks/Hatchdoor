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
  return fs.existsSync(file) ? file : undefined;
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
    response.writeHead(200, { "Content-Type": type });
    fs.createReadStream(file).pipe(response);
  });
}

// This machine's LAN address, the one another device can open.
function lanAddress() {
  for (const addresses of Object.values(os.networkInterfaces())) {
    for (const address of addresses ?? []) {
      if (address.family === "IPv4" && !address.internal) {
        return address.address;
      }
    }
  }
  return "localhost";
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
          console.log(`http://${lanAddress()}:${port}/`);
          resolve(undefined);
        });
    });
  });
}
