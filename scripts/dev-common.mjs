// What the live-acceptance helpers share (#616): where the dev server
// listens, how a setting is read the way the backend reads it, and how a
// token is kept out of anything a helper prints.
//
// The helpers are `just mcp-call` (`mcp-call.mjs`), `just dev-note`
// (`dev-note.mjs`), `just dev-wait` (`dev-wait.mjs`) and the signed-in
// browser for Playwright scripts (`dev-browser.mjs`).

import fs from "node:fs";
import path from "node:path";
import { ROOT, ToolError } from "./proto-common.mjs";

// The dev server answers on this machine and nowhere else. A helper builds
// every address from these, so none can reach a deployed instance.
export const DEV_HOST = "127.0.0.1";
// `backend_port` and `frontend_port` in the `justfile`; `dev-common.test.mjs`
// fails when they drift apart.
export const BACKEND_PORT = 42824;
export const FRONTEND_PORT = 5173;

export const DEV_DIR = path.join(ROOT, ".dev");

export const WEB_TOKEN_SETTING = "HATCHDOOR_WEB_BEARER_TOKEN";
export const MCP_TOKEN_SETTING = "HATCHDOOR_MCP_BEARER_TOKEN";

// One setting: the environment first, then a `KEY=value` file (`.env` unless
// another is named), as the backend and
// `scripts/dev-browser-token.py` find it. Empty when neither has it.
export function settingFrom(name, environment, envFileText) {
  const exported = (environment[name] ?? "").trim();
  if (exported) {
    return exported;
  }
  for (const line of (envFileText ?? "").split("\n")) {
    const [key, ...rest] = line.trim().split("=");
    if (key === name) {
      return rest.join("=").trim().replace(/^(["'])(.*)\1$/, "$2");
    }
  }
  return "";
}

// The same, read from this process and the checkout's `.env`.
export function devSetting(name, root = ROOT, file = ".env") {
  let envFile = "";
  try {
    envFile = fs.readFileSync(path.join(root, file), "utf8");
  } catch {
    // The environment may carry the setting alone.
  }
  return settingFrom(name, process.env, envFile);
}

// `text` with every secret replaced, in the forms a server or a browser may
// hand it back in. Everything a helper prints goes through this.
export function scrubbed(text, secrets) {
  let clean = String(text);
  for (const secret of secrets) {
    if (!secret) {
      continue;
    }
    for (const form of new Set([
      secret,
      encodeURIComponent(secret),
      Buffer.from(secret).toString("base64"),
    ])) {
      clean = clean.replaceAll(form, "[token]");
    }
  }
  return clean;
}

// A port given on the command line, for the tests that stand a fake server
// up. The host is never an argument.
export function portFrom(value, fallback) {
  if (value === undefined) {
    return fallback;
  }
  const port = Number(value);
  if (!Number.isInteger(port) || port < 1 || port > 65535) {
    throw new ToolError(`${value} is not a port`);
  }
  return port;
}

// Take `--name value` out of `args` and return the value, or undefined.
export function takeOption(args, name) {
  const at = args.indexOf(name);
  return at >= 0 ? args.splice(at, 2)[1] : undefined;
}

// All of standard input, as bytes.
export async function standardInput() {
  const chunks = [];
  for await (const chunk of process.stdin) {
    chunks.push(chunk);
  }
  return Buffer.concat(chunks);
}

// What a helper says when the dev server is not there to ask.
export function notAnswering(what, url) {
  return `the ${what} did not answer at ${url}; \`just dev-start\` starts it and \`just dev-wait\` waits for it`;
}

// Run a helper's `main`. A `ToolError` is printed without a stack and with
// every secret cut from it, as `run` in `proto-common.mjs` does for the
// tools that hold none.
export async function runScrubbed(main, secrets) {
  try {
    await main();
  } catch (error) {
    if (!(error instanceof ToolError)) {
      throw error;
    }
    console.error(scrubbed(`error: ${error.message}`, secrets()));
    process.exitCode = 1;
  }
}

export function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
