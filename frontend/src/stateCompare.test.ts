import { describe, expect, it } from "vitest";

import { isExplorerTreeEqual, isNoteEqual } from "./stateCompare";
import type { ExplorerFolder, Note } from "./types";

describe("stateCompare", () => {
  it("isNoteEqual matches identical note payloads", () => {
    const left: Note = {
      title: "Atlas",
      slug: "atlas",
      relative_path: "Notes/Atlas",
      content: "# Atlas",
    };
    const right: Note = {
      title: "Atlas",
      slug: "atlas",
      relative_path: "Notes/Atlas",
      content: "# Atlas",
    };

    expect(isNoteEqual(left, right)).toBe(true);
  });

  it("isNoteEqual detects note content changes", () => {
    const left: Note = {
      title: "Atlas",
      slug: "atlas",
      relative_path: "Notes/Atlas",
      content: "# Atlas",
    };
    const right: Note = {
      ...left,
      content: "# Atlas v2",
    };

    expect(isNoteEqual(left, right)).toBe(false);
  });

  it("isExplorerTreeEqual matches identical trees", () => {
    const left: ExplorerFolder = {
      name: "Vault",
      folders: [
        {
          name: "Notes",
          folders: [],
          notes: [{ title: "Atlas", slug: "atlas" }],
        },
      ],
      notes: [{ title: "Home", slug: "home" }],
    };
    const right: ExplorerFolder = {
      name: "Vault",
      folders: [
        {
          name: "Notes",
          folders: [],
          notes: [{ title: "Atlas", slug: "atlas" }],
        },
      ],
      notes: [{ title: "Home", slug: "home" }],
    };

    expect(isExplorerTreeEqual(left, right)).toBe(true);
  });

  it("isExplorerTreeEqual detects nested folder changes", () => {
    const left: ExplorerFolder = {
      name: "Vault",
      folders: [
        {
          name: "Notes",
          folders: [],
          notes: [{ title: "Atlas", slug: "atlas" }],
        },
      ],
      notes: [{ title: "Home", slug: "home" }],
    };
    const right: ExplorerFolder = {
      name: "Vault",
      folders: [
        {
          name: "Notes",
          folders: [],
          notes: [{ title: "Runbook", slug: "runbook" }],
        },
      ],
      notes: [{ title: "Home", slug: "home" }],
    };

    expect(isExplorerTreeEqual(left, right)).toBe(false);
  });
});
