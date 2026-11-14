import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { test } from "node:test";

import {
  BACKEND_PORT,
  FRONTEND_PORT,
  portFrom,
  scrubbed,
  settingFrom,
  takeOption,
} from "./dev-common.mjs";
import { ROOT, ToolError } from "./proto-common.mjs";

test("the helpers and the justfile agree on the dev ports", () => {
  const justfile = fs.readFileSync(path.join(ROOT, "justfile"), "utf8");
  const port = (name) =>
    Number(new RegExp(`^${name} := "(\\d+)"$`, "m").exec(justfile)[1]);
  assert.equal(port("backend_port"), BACKEND_PORT);
  assert.equal(port("frontend_port"), FRONTEND_PORT);
});

test("a setting comes from the environment first, then .env", () => {
  const env = 'OTHER=1\nHATCHDOOR_MCP_BEARER_TOKEN="from=file"\n';
  assert.equal(
    settingFrom("HATCHDOOR_MCP_BEARER_TOKEN", { HATCHDOOR_MCP_BEARER_TOKEN: " x " }, env),
    "x",
  );
  assert.equal(settingFrom("HATCHDOOR_MCP_BEARER_TOKEN", {}, env), "from=file");
  assert.equal(settingFrom("HATCHDOOR_WEB_BEARER_TOKEN", {}, env), "");
});

test("a secret is cut from text in every form it may come back in", () => {
  const secret = "s3cret/+ value";
  const text = [
    secret,
    encodeURIComponent(secret),
    Buffer.from(secret).toString("base64"),
  ].join(" | ");
  const clean = scrubbed(`Bearer ${text}`, [secret, ""]);
  assert.equal(clean, "Bearer [token] | [token] | [token]");
  assert.equal(scrubbed("nothing to hide", [""]), "nothing to hide");
});

test("a port is a number in range, and an option leaves the arguments", () => {
  assert.equal(portFrom(undefined, 42824), 42824);
  assert.equal(portFrom("8080", 1), 8080);
  assert.throws(() => portFrom("example.com", 1), ToolError);
  const args = ["a", "--port", "9", "b"];
  assert.equal(takeOption(args, "--port"), "9");
  assert.deepEqual(args, ["a", "b"]);
  assert.equal(takeOption(args, "--port"), undefined);
});
