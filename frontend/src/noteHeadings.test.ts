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
      { level: 1, text: "Title", id: "title" },
      { level: 2, text: "Overview", id: "overview" },
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
});
