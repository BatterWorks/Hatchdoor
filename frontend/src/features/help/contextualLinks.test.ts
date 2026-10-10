import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, resolve } from "node:path";

import { describe, expect, it } from "vitest";

import {
  extractMarkdownHeadings,
  slugifyHeading,
} from "../../lib/noteHeadings";
import {
  conflictVault,
  healthyVault,
  pausedVault,
  staleVault,
  syncFailedVault,
  unavailableVault,
} from "../../test/fixtures/vaults";
import {
  CONTEXTUAL_HELP,
  gitConsoleHelp,
  vaultConditionHelp,
} from "./contextualLinks";

// vitest runs with the frontend package root as cwd.
const MANUAL = resolve(process.cwd(), "..", "docs", "user-vault");

/** A page's name the way the server derives it (`page_name` in
 * `src/docs_bundle.rs`): each path segment loses its ordering number and goes
 * through the note slug rule. */
function pageName(relativePath: string): string {
  return relativePath
    .replace(/\.md$/, "")
    .split("/")
    .map((segment) => slugifyHeading(segment.replace(/^\d+\s*/, "")))
    .join("/");
}

function manualPages(dir = MANUAL, prefix = ""): Map<string, string> {
  const pages = new Map<string, string>();
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    const relative = prefix ? `${prefix}/${entry}` : entry;
    if (statSync(path).isDirectory()) {
      for (const [name, file] of manualPages(path, relative)) {
        pages.set(name, file);
      }
    } else if (entry.endsWith(".md")) {
      pages.set(pageName(relative), path);
    }
  }
  return pages;
}

describe("CONTEXTUAL_HELP", () => {
  const pages = manualPages();

  it.each(Object.entries(CONTEXTUAL_HELP))(
    "%s points at a page and heading in docs/user-vault",
    (_topic, target) => {
      const file = pages.get(target.page);
      expect(file, `no manual page is named ${target.page}`).toBeDefined();
      if ("heading" in target) {
        const ids = extractMarkdownHeadings(readFileSync(file!, "utf8")).map(
          (heading) => heading.id,
        );
        expect(ids).toContain(target.heading);
      }
    },
  );

  it.each(Object.entries(CONTEXTUAL_HELP))(
    "%s has a topic for its accessible name",
    (_key, target) => {
      // Read through a plain string so a blank topic fails here even though
      // the type check already refuses a missing one.
      const topic: string = target.topic;
      expect(topic.trim()).not.toBe("");
      expect(topic).toBe(topic.trim());
    },
  );

  it("gives every target a different topic, so no two links on a screen share a name", () => {
    const topics = Object.values(CONTEXTUAL_HELP).map((target) =>
      target.topic.toLowerCase(),
    );
    expect(new Set(topics).size).toBe(topics.length);
  });

  it("names pages the way the server does", () => {
    expect(pages.has("guides/how-to-troubleshoot-common-problems")).toBe(true);
    expect(pages.has("whats-new")).toBe(true);
  });
});

describe("vaultConditionHelp", () => {
  it("links a paused Vault to pausing and resuming", () => {
    expect(vaultConditionHelp(pausedVault(), true)).toBe(
      CONTEXTUAL_HELP.vaultPaused,
    );
  });

  it("links a missing or unreadable folder to the permissions section", () => {
    const vault = unavailableVault();
    vault.activation_error = {
      code: "vault_path_unavailable",
      message: "No such file or directory (os error 2)",
      retryable: true,
    };
    expect(vaultConditionHelp(vault, false)).toBe(CONTEXTUAL_HELP.vaultFolder);
  });

  it("links any other unavailable Vault to the bad-state section", () => {
    expect(vaultConditionHelp(unavailableVault(), false)).toBe(
      CONTEXTUAL_HELP.vaultUnavailable,
    );
    expect(vaultConditionHelp(staleVault(), false)).toBe(
      CONTEXTUAL_HELP.vaultUnavailable,
    );
  });

  it("leaves a Git condition to the Git console", () => {
    expect(vaultConditionHelp(syncFailedVault(), false)).toBeNull();
  });

  it("links a healthy Vault to managing Vaults", () => {
    expect(vaultConditionHelp(healthyVault("Notes"), false)).toBe(
      CONTEXTUAL_HELP.vaultSettings,
    );
  });
});

describe("gitConsoleHelp", () => {
  it("links each Git state to its section", () => {
    expect(gitConsoleHelp(healthyVault("Notes"))).toBe(
      CONTEXTUAL_HELP.gitSetup,
    );
    expect(gitConsoleHelp(syncFailedVault())).toBe(CONTEXTUAL_HELP.gitFailing);
    expect(gitConsoleHelp(conflictVault())).toBe(CONTEXTUAL_HELP.gitConflict);
  });
});
