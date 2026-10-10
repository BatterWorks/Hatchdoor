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

// Each module section runs from its H3 heading to the next heading of the
// same or a higher level. Headings inside a fenced code block do not count.
function moduleSections(lines) {
  const sections = new Map();
  let open = null;
  let fenced = false;

  for (const [index, line] of lines.entries()) {
    if (/^(?:```|~~~)/.test(line)) {
      fenced = !fenced;
      continue;
    }
    const heading = fenced ? null : line.match(/^(#{1,3})\s+(.+)$/);
    if (!heading) {
      continue;
    }
    if (open) {
      open.end = index;
      open = null;
    }
    if (heading[1] === "###") {
      open = { start: index, end: lines.length };
      sections.set(heading[2], open);
    }
  }

  return sections;
}

// `--owner <path>...` answers "which module sections do I need to read?"
// without reading the whole map: it names the section that owns each file and
// prints those sections in full. A directory lists the sections under it, and
// a path outside the production inventory is pointed at the map's own section
// for such paths.
function printOwners(markdown, assignments, requested) {
  const lines = markdown.split(/\r?\n/);
  const sections = moduleSections(lines);
  const span = (name) => {
    const { start, end } = sections.get(name);
    return `module-map.md lines ${start + 1}-${end}`;
  };
  const sectionsOwning = (matches) => [
    ...new Set(
      [...assignments.entries()]
        .filter(([file]) => matches(file))
        .flatMap(([, owners]) => owners.map(({ section }) => section)),
    ),
  ];
  const auxiliaryHeading = "## Auxiliary repository paths";
  const auxiliaryLine = lines.indexOf(auxiliaryHeading) + 1;
  const toPrint = new Set();

  for (const raw of requested) {
    const file = path.isAbsolute(raw)
      ? toRepositoryPath(raw)
      : path.posix.normalize(raw.split(path.sep).join("/"));
    const owners = assignments.get(file);
    if (owners) {
      for (const { section, kind } of owners) {
        console.log(`${file}: ${section} (${kind}), ${span(section)}`);
        toPrint.add(section);
      }
      continue;
    }

    const directory = `${file.replace(/\/$/, "")}/`;
    const sectionsInDirectory = sectionsOwning((assigned) =>
      assigned.startsWith(directory),
    );
    if (sectionsInDirectory.length > 0) {
      console.log(`${file}: a directory, with files in these sections:`);
      for (const section of sectionsInDirectory) {
        console.log(`  ${section}, ${span(section)}`);
      }
      continue;
    }

    if (!/^(?:src|frontend\/src)\//.test(file)) {
      const where =
        auxiliaryLine > 0
          ? `"${auxiliaryHeading}" (module-map.md line ${auxiliaryLine})`
          : "its work packet";
      console.log(
        `${file}: outside the production inventory, so no module owns it. It needs its own work-packet scope: see ${where}`,
      );
      continue;
    }

    const beside = path.posix.dirname(file);
    const siblings = sectionsOwning(
      (assigned) => path.posix.dirname(assigned) === beside,
    );
    console.log(
      siblings.length > 0
        ? `${file}: no assignment. Files beside it belong to: ${siblings.join("; ")}`
        : `${file}: no assignment, and none for any file beside it`,
    );
  }

  for (const section of toPrint) {
    const { start, end } = sections.get(section);
    console.log(
      `\n${"-".repeat(72)}\n${lines.slice(start, end).join("\n").trimEnd()}`,
    );
  }
}

const markdown = await readFile(moduleMapPath, "utf8");
const { assignments, errors } = ownershipAssignments(markdown);

const argv = process.argv.slice(2);
if (argv[0] === "--owner") {
  const requested = argv.slice(1);
  if (requested.length === 0) {
    console.error("Usage: check-module-map.mjs --owner <path>...");
    process.exit(2);
  }
  printOwners(markdown, assignments, requested);
  process.exit(0);
}

const productionFiles = await productionSourcePaths();

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
