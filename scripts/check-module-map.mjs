#!/usr/bin/env node

import { lstat, readFile, readdir } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repositoryRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);
const moduleMapPath = path.join(
  repositoryRoot,
  "docs",
  "architecture",
  "module-map.md",
);

const toRepositoryPath = (absolutePath) =>
  path.relative(repositoryRoot, absolutePath).split(path.sep).join("/");

async function walk(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const paths = [];

  for (const entry of entries) {
    const absolutePath = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      paths.push(...(await walk(absolutePath)));
    } else if (entry.isFile()) {
      paths.push(toRepositoryPath(absolutePath));
    }
  }

  return paths;
}

async function productionSourcePaths() {
  const backend = (await walk(path.join(repositoryRoot, "src"))).filter(
    (file) => file.endsWith(".rs") && !file.endsWith("/tests.rs"),
  );
  const frontend = (
    await walk(path.join(repositoryRoot, "frontend", "src"))
  ).filter(
    (file) =>
      /\.(?:ts|tsx|css)$/.test(file) &&
      !/\.test\.(?:ts|tsx)$/.test(file) &&
      !file.startsWith("frontend/src/test/"),
  );

  return [...backend, ...frontend].sort();
}

function pathsInBackticks(line) {
  return [...line.matchAll(/`([^`]+)`/g)].map((match) => match[1]);
}

function ownershipAssignments(markdown) {
  const assignments = new Map();
  const errors = [];
  let section = null;
  let collection = null;
  let expectSharedPaths = false;

  const add = (file, kind, lineNumber) => {
    if (!section) {
      errors.push(
        `line ${lineNumber}: ownership path ${file} is outside a module section`,
      );
      return;
    }
    const normalized = path.posix.normalize(file);
    const parts = file.split("/");
    const absolutePath = path.resolve(repositoryRoot, ...parts);
    const relativePath = path.relative(repositoryRoot, absolutePath);
    if (
      !/^(?:src|frontend\/src)\//.test(file) ||
      /[*?[\]{}]/.test(file) ||
      path.posix.isAbsolute(file) ||
      file.includes("\\") ||
      normalized !== file ||
      parts.includes(".") ||
      parts.includes("..") ||
      relativePath === "" ||
      relativePath === ".." ||
      relativePath.startsWith(`..${path.sep}`) ||
      path.isAbsolute(relativePath)
    ) {
      errors.push(
        `line ${lineNumber}: ownership path ${file} is not a canonical repository-relative path`,
      );
      return;
    }
    const owners = assignments.get(file) ?? [];
    owners.push({ section, kind, lineNumber });
    assignments.set(file, owners);
  };

  for (const [index, line] of markdown.split(/\r?\n/).entries()) {
    const lineNumber = index + 1;
    const heading = line.match(/^(#{1,6})\s+(.+)$/);
    if (heading) {
      section = heading[1] === "###" ? heading[2] : null;
      collection = null;
      expectSharedPaths = false;
      continue;
    }

    if (line.startsWith("**Owned paths:**")) {
      collection = null;
      const inlinePaths = pathsInBackticks(line);
      for (const file of inlinePaths) {
        add(file, "owned", lineNumber);
      }
      expectSharedPaths = line.includes("none by default");
      if (inlinePaths.length === 0 && !expectSharedPaths) {
        collection = "owned";
      }
      continue;
    }

    if (line.startsWith("**Shared path:**")) {
      const inlinePaths = pathsInBackticks(line);
      for (const file of inlinePaths) {
        add(file, "shared", lineNumber);
      }
      collection = inlinePaths.length === 0 ? "shared" : null;
      continue;
    }

    if (line.startsWith("**Paths:**")) {
      const inlinePaths = pathsInBackticks(line);
      if (!expectSharedPaths && inlinePaths.length > 0) {
        errors.push(
          `line ${lineNumber}: inline Paths ownership is not preceded by "Owned paths: none by default"`,
        );
      }
      if (expectSharedPaths) {
        for (const file of inlinePaths) {
          add(file, "shared", lineNumber);
        }
      }
      collection =
        expectSharedPaths && inlinePaths.length === 0 ? "shared" : null;
      expectSharedPaths = false;
      continue;
    }

    if (!collection) {
      continue;
    }
    if (line.trim() === "") {
      continue;
    }
    if (line.startsWith("- ")) {
      for (const file of pathsInBackticks(line)) {
        add(file, collection, lineNumber);
      }
      continue;
    }
    collection = null;
  }

  return { assignments, errors };
}

async function invalidAssignedPaths(assignments) {
  const stale = [];
  await Promise.all(
    [...assignments.keys()].map(async (file) => {
      try {
        const stat = await lstat(path.join(repositoryRoot, file));
        if (!stat.isFile()) {
          stale.push(file);
        }
      } catch {
        stale.push(file);
      }
    }),
  );
  return stale.sort();
}

function printGroup(title, values, format = (value) => `  ${value}`) {
  if (values.length === 0) {
    return;
  }
  console.error(`\n${title}`);
  for (const value of values) {
    console.error(format(value));
  }
}

const markdown = await readFile(moduleMapPath, "utf8");
const productionFiles = await productionSourcePaths();
const { assignments, errors } = ownershipAssignments(markdown);

const unowned = productionFiles.filter((file) => !assignments.has(file));
const duplicates = [...assignments.entries()]
  .filter(([, owners]) => owners.length !== 1)
  .sort(([left], [right]) => left.localeCompare(right));
const stale = await invalidAssignedPaths(assignments);

if (
  errors.length > 0 ||
  unowned.length > 0 ||
  duplicates.length > 0 ||
  stale.length > 0
) {
  console.error("Module map check failed.");
  printGroup("INVALID OWNERSHIP MARKUP", errors);
  printGroup("UNOWNED PRODUCTION FILES", unowned);
  printGroup("STALE ASSIGNED PATHS", stale);
  printGroup("DUPLICATE ASSIGNMENTS", duplicates, ([file, owners]) => {
    const details = owners
      .map(
        ({ section, kind, lineNumber }) =>
          `${section} (${kind}, line ${lineNumber})`,
      )
      .join("; ");
    return `  ${file}\n    ${details}`;
  });
  process.exitCode = 1;
} else {
  console.log(
    `Module map OK: ${productionFiles.length} production source files have exactly one assignment.`,
  );
}
