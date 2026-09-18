import { describe, expect, it } from "vitest";

import {
  assignHeadingId,
  extractMarkdownHeadings,
  slugifyHeading,
} from "./noteHeadings";

describe("noteHeadings", () => {
  it("extracts headings and skips fenced code blocks", () => {
    const headings = extractMarkdownHeadings(
      ["# Title", "", "```md", "## Not a heading", "```", "## Overview"].join(
        "\n",
      ),
    );

    expect(headings).toEqual([
      { level: 1, text: "Title", id: "title", sourceLine: 1 },
      { level: 2, text: "Overview", id: "overview", sourceLine: 6 },
    ]);
  });

  it("assignHeadingId appends numeric suffixes for duplicates", () => {
    const counts = new Map<string, number>();

    expect(assignHeadingId("Alpha", counts)).toBe("alpha");
    expect(assignHeadingId("Alpha", counts)).toBe("alpha-2");
  });

  it("slugifyHeading normalizes markdown punctuation", () => {
    expect(slugifyHeading("**API** [Guide](x) / v1")).toBe("api-guide-v1");
  });

  it("slugifyHeading folds accents and keeps other scripts", () => {
    expect(slugifyHeading("Gerard Veá")).toBe("gerard-vea");
    expect(slugifyHeading("Cafe\u0301")).toBe(slugifyHeading("Caf\u00e9"));
    expect(slugifyHeading("Straße")).toBe("strasse");
    expect(slugifyHeading("Łódź")).toBe("lodz");
    expect(slugifyHeading("資料 Обзор")).toBe("資料-обзор");
    expect(slugifyHeading("हिन्दी")).toBe("हिन्दी");
  });

  it("slugifyHeading falls back when nothing addressable is left", () => {
    expect(slugifyHeading("!!! ???")).toBe("section");
  });

  it("extracts obsidian wikilink heading labels for toc ids", () => {
    const headings = extractMarkdownHeadings("## [[Project/Plan|Plan Home]]");

    expect(headings).toEqual([
      { level: 2, text: "Plan Home", id: "plan-home", sourceLine: 1 },
    ]);
  });
});
