// The image labels in the Dockerfile's runtime stage repeat facts Cargo.toml
// already states. This fails when one changes without the other.

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const repositoryRoot = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);

async function read(file) {
  return readFile(path.join(repositoryRoot, file), "utf8");
}

// The instructions of the last stage, the one a plain `docker build` ships.
function runtimeStage(dockerfile) {
  const stages = dockerfile.split(/^FROM /m);
  const runtime = stages.at(-1);
  assert.match(runtime, /^\S+ AS runtime$/m, "the last stage is `runtime`");
  return runtime;
}

function labels(stage) {
  return Object.fromEntries(
    [...stage.matchAll(/^(?:LABEL)?\s+([a-z.]+)="([^"]*)"/gm)].map(
      ([, key, value]) => [key, value],
    ),
  );
}

function cargoField(cargo, field) {
  const match = new RegExp(`^${field} = "([^"]*)"$`, "m").exec(cargo);
  assert.ok(match, `Cargo.toml declares ${field}`);
  return match[1];
}

test("the runtime image carries the labels that link it to the project", async () => {
  const cargo = await read("Cargo.toml");
  const found = labels(runtimeStage(await read("Dockerfile")));
  const repository = cargoField(cargo, "repository");

  assert.deepEqual(found, {
    "org.opencontainers.image.title": "Hatchdoor",
    "org.opencontainers.image.description": cargoField(cargo, "description"),
    "org.opencontainers.image.licenses": cargoField(cargo, "license"),
    "org.opencontainers.image.source": repository,
    "org.opencontainers.image.url": repository,
    "org.opencontainers.image.documentation":
      "https://docs-hatchdoor.battercloud.cc",
    "org.opencontainers.image.version": "${VERSION}",
    "org.opencontainers.image.revision": "${GIT_SHA}",
    "io.modelcontextprotocol.server.name": `io.github.${new URL(repository).pathname.split("/")[1]}/hatchdoor`,
  });
});

test("the version and revision labels come from build arguments declared in the runtime stage", async () => {
  const runtime = runtimeStage(await read("Dockerfile"));
  // An ARG declared in an earlier stage is out of scope here and would
  // expand to nothing without an error.
  assert.match(runtime, /^ARG VERSION=""$/m);
  assert.match(runtime, /^ARG GIT_SHA=""$/m);
});
