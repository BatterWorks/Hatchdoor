import { describe, expect, it } from "vitest";

import {
  isExplorerTreeEqual,
  isNoteEqual,
  isNoteLinksEqual,
} from "./stateCompare";
import type { ExplorerFolder, Note, NoteLinks } from "../types";

describe("stateCompare", () => {
  it("isNoteEqual matches identical note payloads", () => {
    const left: Note = {
      title: "Atlas",
      slug: "atlas",
      relative_path: "Notes/Atlas",
      content: "# Atlas",
      content_hash: "hash-left",
    };
    const right: Note = {
      title: "Atlas",
      slug: "atlas",
      relative_path: "Notes/Atlas",
      content: "# Atlas",
      content_hash: "hash-right",
    };

    expect(isNoteEqual(left, right)).toBe(true);
  });

  it("isNoteEqual detects note content changes", () => {
    const left: Note = {
      title: "Atlas",
      slug: "atlas",
      relative_path: "Notes/Atlas",
      content: "# Atlas",
      content_hash: "hash-left",
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

  it("isNoteLinksEqual matches identical link payloads", () => {
    const left: NoteLinks = {
      outgoing: [{ title: "Plan", slug: "plan", relative_path: "Plan" }],
      backlinks: [{ title: "Home", slug: "home", relative_path: "Home" }],
    };
    const right: NoteLinks = {
      outgoing: [{ title: "Plan", slug: "plan", relative_path: "Plan" }],
      backlinks: [{ title: "Home", slug: "home", relative_path: "Home" }],
    };

    expect(isNoteLinksEqual(left, right)).toBe(true);
  });

  it("isNoteLinksEqual detects changed backlinks", () => {
    const left: NoteLinks = {
      outgoing: [],
      backlinks: [{ title: "Home", slug: "home", relative_path: "Home" }],
    };
    const right: NoteLinks = {
      outgoing: [],
      backlinks: [{ title: "Atlas", slug: "atlas", relative_path: "Atlas" }],
    };

    expect(isNoteLinksEqual(left, right)).toBe(false);
  });
});
