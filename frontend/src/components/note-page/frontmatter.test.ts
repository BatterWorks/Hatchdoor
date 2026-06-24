import { describe, expect, it } from "vitest";

import {
  buildContentWithFrontmatter,
  parseFrontmatterEntries,
  splitFrontmatter,
} from "./frontmatter";

describe("splitFrontmatter", () => {
  it("splits fenced YAML frontmatter from the markdown body", () => {
    expect(splitFrontmatter("---\ntitle: Home\n---\n# Body")).toEqual({
      raw: "title: Home",
      body: "# Body",
    });
  });

  it("leaves content unchanged when there is no closing fence", () => {
    const content = "---\ntitle: Home\n# Body";
    expect(splitFrontmatter(content)).toEqual({ raw: null, body: content });
  });
});

describe("parseFrontmatterEntries", () => {
  it("parses flat scalar fields and simple lists", () => {
    expect(
      parseFrontmatterEntries("title: Home\ntags:\n  - work\n  - planning"),
    ).toEqual({
      editable: true,
      entries: [
        { id: "title", key: "title", value: "Home", kind: "text" },
        { id: "tags", key: "tags", value: "work, planning", kind: "list" },
      ],
    });
  });

  it("refuses nested mappings so structured editing cannot corrupt them", () => {
    expect(
      parseFrontmatterEntries("title: Home\nowner:\n  name: Aurelien"),
    ).toEqual({ editable: false, entries: [] });
  });
});

describe("buildContentWithFrontmatter", () => {
  it("serializes edited entries and preserves the body", () => {
    expect(
      buildContentWithFrontmatter(
        [
          { id: "title", key: "title", value: "Home Base", kind: "text" },
          { id: "tags", key: "tags", value: "work, planning", kind: "list" },
        ],
        "# Body",
      ),
    ).toBe("---\ntitle: Home Base\ntags:\n  - work\n  - planning\n---\n# Body");
  });

  it("drops empty property names and removes frontmatter when no fields remain", () => {
    expect(
      buildContentWithFrontmatter(
        [{ id: "blank", key: " ", value: "ignored", kind: "text" }],
        "# Body",
      ),
    ).toBe("# Body");
  });
});
