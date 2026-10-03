// Shared release logic for `just release-prepare` and `just release-publish`
// (ADR-36, ADR-37). Everything here is pure text handling: version strings,
// the version files, the changelog, and the release checklist. The scripts
// that call it do the git and GitHub work.
//
// The checklist is a contract between two scripts and an agent. Prepare
// renders it into the version-bump pull request, the agent ticks the boxes on
// GitHub, and publish parses it back and refuses while any box is unticked.
// Only this file renders or parses it, so the wording cannot drift between
// the two ends.

// A bare semantic version: no `v`, no pre-release or build suffix, and no
// leading zeros, so "2.07.0" cannot sort as a different release than "2.7.0".
const VERSION_PATTERN = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;

export function parseVersion(text) {
  const match = VERSION_PATTERN.exec(text);
  return match ? match.slice(1).map(Number) : null;
}

export function compareVersions(left, right) {
  const a = parseVersion(left);
  const b = parseVersion(right);
  if (!a || !b) {
    throw new Error(`cannot compare ${left} and ${right}`);
  }
  for (let index = 0; index < 3; index += 1) {
    if (a[index] !== b[index]) {
      return a[index] < b[index] ? -1 : 1;
    }
  }
  return 0;
}

// The highest version among `v<version>` tags. Tags in any other shape, such
// as image tags that leaked into git, are not releases and are ignored.
export function latestReleaseVersion(tags) {
  let latest = null;
  for (const tag of tags) {
    if (!tag.startsWith("v")) {
      continue;
    }
    const version = tag.slice(1);
    if (!parseVersion(version)) {
      continue;
    }
    if (latest === null || compareVersions(version, latest) > 0) {
      latest = version;
    }
  }
  return latest;
}

// ---------------------------------------------------------------------------
// Changelog

const UNRELEASED_HEADING = "## Unreleased";

export function releaseHeadingPrefix(version) {
  return `## v${version} - `;
}

// A section runs from its `## ` heading line to the line before the next
// `## ` heading, or to the end of the file. `###` subheadings stay inside it.
function findSection(text, isHeading) {
  const lines = text.split("\n");
  const start = lines.findIndex(isHeading);
  if (start === -1) {
    return null;
  }
  let end = lines.length;
  for (let index = start + 1; index < lines.length; index += 1) {
    if (lines[index].startsWith("## ")) {
      end = index;
      break;
    }
  }
  return {
    heading: lines[start],
    body: lines.slice(start + 1, end).join("\n"),
  };
}

export function unreleasedSection(text) {
  return findSection(text, (line) => line.trimEnd() === UNRELEASED_HEADING);
}

export function releaseSection(text, version) {
  const prefix = releaseHeadingPrefix(version);
  return findSection(text, (line) => line.startsWith(prefix));
}

// An entry is a list item. Headings, intro prose and link definitions alone
// do not make a release.
export function hasEntries(sectionBody) {
  return /^[ \t]*[-*+][ \t]+\S/m.test(sectionBody);
}

export function renameUnreleased(text, version, date) {
  const lines = text.split("\n");
  const index = lines.findIndex(
    (line) => line.trimEnd() === UNRELEASED_HEADING,
  );
  if (index === -1) {
    throw new Error(`CHANGELOG.md has no "${UNRELEASED_HEADING}" section.`);
  }
  lines[index] = `${releaseHeadingPrefix(version)}${date}`;
  return lines.join("\n");
}

const LINK_DEFINITION = /^\[#(\d+)\]:[ \t]*\S/;

function linkDefinitions(text) {
  const definitions = new Map();
  for (const line of text.split("\n")) {
    const match = LINK_DEFINITION.exec(line);
    if (match && !definitions.has(match[1])) {
      definitions.set(match[1], line.trimEnd());
    }
  }
  return definitions;
}

// `[#275]` used as a reference, not the `[#275]:` that defines it.
function linkReferences(sectionBody) {
  const references = new Set();
  for (const line of sectionBody.split("\n")) {
    if (LINK_DEFINITION.test(line)) {
      continue;
    }
    for (const match of line.matchAll(/\[#(\d+)\](?!:)/g)) {
      references.add(match[1]);
    }
  }
  return [...references];
}

// Markdown link definitions are global to the file, so a reference counts as
// defined wherever in CHANGELOG.md its definition sits.
export function undefinedReferences(text, sectionBody) {
  const definitions = linkDefinitions(text);
  return linkReferences(sectionBody)
    .filter((number) => !definitions.has(number))
    .map((number) => `#${number}`);
}

// The GitHub Release body: the version's section without its heading, which
// the release title already carries. A definition that lives elsewhere in the
// file is copied in, because the release page cannot see the rest of
// CHANGELOG.md and would show the marker as literal text.
export function releaseNotes(text, version) {
  const section = releaseSection(text, version);
  if (!section) {
    throw new Error(
      `CHANGELOG.md has no "${releaseHeadingPrefix(version)}" section.`,
    );
  }
  const body = section.body.trim();
  const local = linkDefinitions(body);
  const all = linkDefinitions(text);
  const borrowed = linkReferences(body)
    .filter((number) => !local.has(number) && all.has(number))
    .map((number) => all.get(number));
  return borrowed.length === 0
    ? `${body}\n`
    : `${body}\n\n${borrowed.join("\n")}\n`;
}

// ---------------------------------------------------------------------------
// What's new highlights (ADR-42)
//
// The manual's What's new page carries 3 to 6 plain lines per release,
// action-needed items first, under the same `## v<version> - <date>` heading
// the changelog uses. `src/handlers/whats_new.rs` parses the page the binary
// bundles, and its `the_bundled_page_parses` test, which `just check-full`
// runs, holds the rest of the format: one link at most per line, into the
// manual, at its end.

export const WHATS_NEW_PAGE = "docs/user-vault/What's new.md";
export const MIN_HIGHLIGHTS = 3;
export const MAX_HIGHLIGHTS = 6;
const ACTION_NEEDED = "**Action needed:**";

// A changelog subheading whose entries ask something of the person
// upgrading, such as "Breaking changes — action required on upgrade".
const ACTION_HEADING = /breaking|action/i;

// One draft line per changelog entry: its first sentence, without issue
// references, action-needed entries first. The draft is a starting point;
// a release with more than six entries refuses until someone cuts it down.
export function draftHighlights(sectionBody) {
  const action = [];
  const plain = [];
  let actionNeeded = false;
  for (const line of sectionBody.split("\n")) {
    if (line.startsWith("### ")) {
      actionNeeded = ACTION_HEADING.test(line);
      continue;
    }
    const entry = /^[-*+][ \t]+(.*\S)/.exec(line);
    if (!entry) {
      continue;
    }
    const text = entry[1]
      .replace(/\s*\(\[#\d+\]\)/g, "")
      .replace(/\s*\[#\d+\]/g, "")
      .trim();
    const sentence = /^.+?[.!?](?=\s|$)/.exec(text)?.[0] ?? text;
    (actionNeeded ? action : plain).push(
      actionNeeded ? `- ${ACTION_NEEDED} ${sentence}` : `- ${sentence}`,
    );
  }
  return [...action, ...plain];
}

export function whatsNewSection(page, version) {
  const prefix = releaseHeadingPrefix(version);
  return findSection(page, (line) => line.startsWith(prefix));
}

// The page with a new release section, above every older one.
export function addWhatsNewSection(page, version, date, lines) {
  const section = [`${releaseHeadingPrefix(version)}${date}`, "", ...lines, ""];
  const pageLines = page.split("\n");
  const first = pageLines.findIndex((line) => line.startsWith("## "));
  if (first !== -1) {
    pageLines.splice(first, 0, ...section);
    return pageLines.join("\n");
  }
  return `${page.trimEnd()}\n\n${section.join("\n")}`;
}

// Why a release section cannot ship, or an empty list when it can.
export function highlightProblems(sectionBody) {
  const problems = [];
  const items = [];
  for (const line of sectionBody.split("\n")) {
    if (line.trim() === "") {
      continue;
    }
    if (line.startsWith("- ")) {
      items.push(line);
    } else {
      problems.push(`"${line}" is not a one-line "- " item.`);
    }
  }
  if (items.length < MIN_HIGHLIGHTS || items.length > MAX_HIGHLIGHTS) {
    problems.push(
      `it has ${items.length} lines; a release has ${MIN_HIGHLIGHTS} to ${MAX_HIGHLIGHTS}.`,
    );
  }
  const firstPlain = items.findIndex(
    (item) => !item.startsWith(`- ${ACTION_NEEDED}`),
  );
  if (
    firstPlain !== -1 &&
    items.slice(firstPlain).some((item) => item.startsWith(`- ${ACTION_NEEDED}`))
  ) {
    problems.push("an action-needed line comes after a plain one; they go first.");
  }
  return problems;
}

// ---------------------------------------------------------------------------
// Version files

export const VERSION_FILES = [
  "Cargo.toml",
  "Cargo.lock",
  "frontend/package.json",
  "frontend/package-lock.json",
];

// The line holding `key = "value"` inside Cargo.toml's `[package]` table,
// which ends at the next table header.
function cargoPackageField(cargoToml, key) {
  const lines = cargoToml.split("\n");
  const start = lines.findIndex((line) => line.trim() === "[package]");
  if (start === -1) {
    return null;
  }
  const pattern = new RegExp(`^(${key}[ \\t]*=[ \\t]*")([^"]*)(".*)$`);
  for (let index = start + 1; index < lines.length; index += 1) {
    if (lines[index].trimStart().startsWith("[")) {
      break;
    }
    const match = pattern.exec(lines[index]);
    if (match) {
      return { lines, index, match };
    }
  }
  return null;
}

function setCargoPackageVersion(cargoToml, version) {
  const field = cargoPackageField(cargoToml, "version");
  if (!field) {
    throw new Error("Cargo.toml has no version field to set.");
  }
  field.lines[field.index] = `${field.match[1]}${version}${field.match[3]}`;
  return field.lines.join("\n");
}

function cargoLockVersion(name) {
  const escaped = name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return new RegExp(
    `(^\\[\\[package\\]\\]\\nname = "${escaped}"\\nversion = ")([^"]*)(")`,
    "m",
  );
}

function cargoPackageName(cargoToml) {
  const field = cargoPackageField(cargoToml, "name");
  if (!field) {
    throw new Error("Cargo.toml has no [package] name.");
  }
  return field.match[2];
}

// Reads every version the four files declare. package-lock.json declares the
// root package's version twice, and both copies must agree.
export function readVersions(files) {
  const name = cargoPackageName(files["Cargo.toml"]);
  const lock = JSON.parse(files["frontend/package-lock.json"]);
  return {
    "Cargo.toml":
      cargoPackageField(files["Cargo.toml"], "version")?.match[2] ?? null,
    "Cargo.lock": cargoLockVersion(name).exec(files["Cargo.lock"])?.[2] ?? null,
    "frontend/package.json":
      JSON.parse(files["frontend/package.json"]).version ?? null,
    "frontend/package-lock.json": lock.version ?? null,
    'frontend/package-lock.json packages[""]':
      lock.packages?.[""]?.version ?? null,
  };
}

// Every declared version that is not `version`, as `file (found)` strings.
export function versionDisagreements(files, version) {
  return Object.entries(readVersions(files))
    .filter(([, found]) => found !== version)
    .map(([file, found]) => `${file} (${found ?? "no version found"})`);
}

function replaceOnce(text, pattern, version, file) {
  if (!pattern.test(text)) {
    throw new Error(`${file} has no version field to set.`);
  }
  return text.replace(pattern, `$1${version}$3`);
}

// Returns the four files with their version set. The JSON files round-trip
// through npm's own layout (two-space indent, trailing newline), so nothing
// but the version moves.
export function setVersions(files, version) {
  const name = cargoPackageName(files["Cargo.toml"]);
  const pkg = JSON.parse(files["frontend/package.json"]);
  pkg.version = version;
  const lock = JSON.parse(files["frontend/package-lock.json"]);
  lock.version = version;
  if (!lock.packages?.[""]) {
    throw new Error('frontend/package-lock.json has no packages[""] entry.');
  }
  lock.packages[""].version = version;
  return {
    "Cargo.toml": setCargoPackageVersion(files["Cargo.toml"], version),
    "Cargo.lock": replaceOnce(
      files["Cargo.lock"],
      cargoLockVersion(name),
      version,
      "Cargo.lock",
    ),
    "frontend/package.json": `${JSON.stringify(pkg, null, 2)}\n`,
    "frontend/package-lock.json": `${JSON.stringify(lock, null, 2)}\n`,
  };
}

// ---------------------------------------------------------------------------
// Release checklist (ADR-36 decision 3, on the version-bump pull request per
// ADR-37)

export const DOCS_FRESHNESS_ITEM =
  "The notes named by `just docs-freshness main` were read, and the review was recorded with `just docs-freshness-ack main`.";

export const CHECKLIST_ITEMS = [
  "The changelog section lists everything that shipped since the previous release, and nothing that did not.",
  "The README describes what shipped.",
  "The roadmap is current.",
  DOCS_FRESHNESS_ITEM,
  "No ADR is contradicted by what shipped.",
  "An MCP conformance run is recorded, if MCP behavior changed.",
  "The release title and notes are drafted.",
];

// A fence longer than any backtick run in the content, so pasted output that
// itself contains a fence cannot close the block early.
function fenced(content, indent) {
  const longest = Math.max(
    2,
    ...[...content.matchAll(/`+/g)].map((match) => match[0].length),
  );
  const fence = "`".repeat(longest + 1);
  const lines = content.replace(/\n+$/, "").split("\n");
  return [`${fence}text`, ...lines, fence]
    .map((line) => (line === "" ? "" : `${indent}${line}`))
    .join("\n");
}

export function renderChecklist(docsFreshnessOutput) {
  return CHECKLIST_ITEMS.map((item) => {
    const line = `- [ ] ${item}`;
    if (item !== DOCS_FRESHNESS_ITEM) {
      return line;
    }
    const output = docsFreshnessOutput.trim() || "(no output)";
    return `${line}\n\n${fenced(output, "  ")}\n`;
  }).join("\n");
}

// Reads the boxes back from a pull request body. An item counts only in its
// exact wording; an item whose line was edited or deleted is reported as
// missing, never as ticked. Lines inside fenced blocks are pasted output, not
// boxes.
export function parseChecklist(body) {
  const found = new Map();
  let fence = null;
  for (const raw of body.replace(/\r\n/g, "\n").split("\n")) {
    const line = raw.trim();
    const opener = /^(`{3,}|~{3,})/.exec(line);
    if (fence) {
      if (
        opener &&
        opener[1][0] === fence[0] &&
        opener[1].length >= fence.length &&
        line === opener[1]
      ) {
        fence = null;
      }
      continue;
    }
    if (opener) {
      fence = opener[1];
      continue;
    }
    const box = /^[-*+] \[([ xX])\] (.*)$/.exec(line);
    if (
      box &&
      CHECKLIST_ITEMS.includes(box[2].trim()) &&
      !found.has(box[2].trim())
    ) {
      found.set(box[2].trim(), box[1] !== " ");
    }
  }
  return {
    items: CHECKLIST_ITEMS.filter((item) => found.has(item)).map((item) => ({
      text: item,
      checked: found.get(item),
    })),
    missing: CHECKLIST_ITEMS.filter((item) => !found.has(item)),
  };
}
