import { describe, expect, it } from "vitest";

import {
  decodePercent,
  findMarkdownNoteLinks,
  noteLinkTarget,
} from "./markdownLinks";

function rawPaths(markdown: string): string[] {
  return findMarkdownNoteLinks(markdown).map((link) => link.rawPath);
}

describe("findMarkdownNoteLinks (ADR-28)", () => {
  it("finds inline, angle-bracketed and reference-definition note links", () => {
    const markdown = [
      "See [a](../20-projects/Beacon%20Launch.md) and [b](<../x/Two Words.md>).",
      'Then [c][plan] and [plan]. [d](/20-projects/Plan.md#Goals "t")',
      "",
      "[plan]: ../20-projects/Plan.md",
    ].join("\n");
    expect(rawPaths(markdown)).toEqual([
      "../20-projects/Beacon%20Launch.md",
      "../x/Two Words.md",
      "/20-projects/Plan.md",
      "../20-projects/Plan.md",
    ]);
  });

  it("skips non-note targets, images, code, wikilinks and external URLs", () => {
    const markdown = [
      "[pdf](report.pdf) [video](clip.mp4) [web](https://example.com/a.md)",
      "![img](Note.md) `[code](Note.md)` [[Note.md]] [mail](mailto:x.md)",
      "```",
      "[fenced](Note.md)",
      "```",
      "[proto](//host/a.md) [bare](.md) [ext](Note.markdown)",
    ].join("\n");
    expect(rawPaths(markdown)).toEqual([]);
  });

  it("treats the rest of a line after an unclosed backtick as code", () => {
    expect(rawPaths("[a](A.md) ` [b](B.md)\n[c](C.md)")).toEqual([
      "A.md",
      "C.md",
    ]);
  });

  it("locates the destination so an angle-bracketed one can be replaced whole", () => {
    const markdown = "x [a](<Two Words.md#Top>) y";
    const [link] = findMarkdownNoteLinks(markdown);
    expect(link.angle).toBe(true);
    expect(markdown.slice(link.start, link.end)).toBe("Two Words.md#Top");
    expect(link.anchor).toBe("Top");
  });
});

describe("noteLinkTarget", () => {
  it("separates the anchor and needs .md on the path, not the anchor", () => {
    expect(noteLinkTarget("Install.md#First%20Run")).toEqual({
      rawPath: "Install.md",
      anchor: "First%20Run",
    });
    expect(noteLinkTarget("Install#x.md")).toBeNull();
  });
});

describe("decodePercent", () => {
  it("keeps a bare percent literal", () => {
    expect(decodePercent("Save%2020%%20now.md")).toBe("Save 20% now.md");
    expect(decodePercent("Caf%C3%A9.md")).toBe("Café.md");
    expect(decodePercent("%FF.md")).toBe("%FF.md");
  });
});
