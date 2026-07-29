import { describe, expect, it } from "vitest";

import {
  blockRange,
  detectLineEnding,
  frontmatterLineOffset,
  linesMatch,
  replaceLines,
  sliceLines,
} from "./sourceMap";

describe("frontmatterLineOffset", () => {
  it("is zero for a note with no frontmatter", () => {
    expect(frontmatterLineOffset("# Heading\n\nBody.")).toBe(0);
  });

  it("counts the frontmatter block and its closing fence", () => {
    expect(frontmatterLineOffset("---\ntitle: Home\n---\n# Heading")).toBe(3);
  });

  // parseFrontmatter does lines.slice(end + 1), so it does not consume a
  // trailing blank line. Frontmatter followed by zero, one, or two blank lines
  // all yield the same offset.
  it("does not consume blank lines after the closing fence", () => {
    expect(frontmatterLineOffset("---\ntitle: Home\n---\n# Heading")).toBe(3);
    expect(frontmatterLineOffset("---\ntitle: Home\n---\n\n# Heading")).toBe(3);
    expect(frontmatterLineOffset("---\ntitle: Home\n---\n\n\n# Heading")).toBe(
      3,
    );
  });

  // The trap is the opposite of what it looks like: parseFrontmatter returns
  // body: input in three separate bail-out cases, all of which mean offset 0.
  // Anything that pattern-matches "find the second ---" reports 3 here and
  // misaddresses every block in the note.
  it("is zero when the header is prose rather than key: value", () => {
    expect(frontmatterLineOffset("---\njust prose here\n---\n# Heading")).toBe(
      0,
    );
  });

  it("is zero when the frontmatter is never closed", () => {
    expect(frontmatterLineOffset("---\ntitle: Home\n# Heading")).toBe(0);
  });

  it("is zero when the note is too short to hold frontmatter", () => {
    expect(frontmatterLineOffset("---\ntitle: Home")).toBe(0);
  });

  it("is zero when the first line is not a fence", () => {
    expect(frontmatterLineOffset("# Heading\n---\ntitle: Home\n---")).toBe(0);
  });

  it("counts CRLF frontmatter the same as LF", () => {
    expect(
      frontmatterLineOffset("---\r\ntitle: Home\r\n---\r\n# Heading"),
    ).toBe(3);
  });
});

describe("detectLineEnding", () => {
  it("reports LF for a plain file", () => {
    expect(detectLineEnding("a\nb\nc")).toBe("\n");
  });

  it("reports CRLF for a Windows file", () => {
    expect(detectLineEnding("a\r\nb\r\nc")).toBe("\r\n");
  });

  it("reports the dominant ending for a mixed file", () => {
    expect(detectLineEnding("a\r\nb\r\nc\nd")).toBe("\r\n");
    expect(detectLineEnding("a\nb\nc\r\nd")).toBe("\n");
  });

  it("reports LF for a single-line file", () => {
    expect(detectLineEnding("just one line")).toBe("\n");
  });
});

describe("sliceLines", () => {
  it("returns the requested inclusive line range", () => {
    expect(sliceLines("one\ntwo\nthree\nfour", 2, 3)).toBe("two\nthree");
  });

  it("returns a single line when start and end match", () => {
    expect(sliceLines("one\ntwo\nthree", 1, 1)).toBe("one");
  });

  it("reads CRLF files without carrying the carriage return into the slice", () => {
    expect(sliceLines("one\r\ntwo\r\nthree", 2, 2)).toBe("two");
  });
});

describe("replaceLines", () => {
  it("swaps the given range for the replacement text", () => {
    expect(replaceLines("one\ntwo\nthree", 2, 2, "TWO")).toBe(
      "one\nTWO\nthree",
    );
  });

  it("accepts a replacement spanning more lines than it replaces", () => {
    expect(replaceLines("one\ntwo\nthree", 2, 2, "a\nb")).toBe(
      "one\na\nb\nthree",
    );
  });

  // A naive split(/\r?\n/).join("\n") rewrites a CRLF file to LF on the first
  // block edit: a whole-file spurious diff on every note touched, which is the
  // exact failure mode that ruled out a WYSIWYG editor.
  it("preserves CRLF line endings", () => {
    expect(replaceLines("one\r\ntwo\r\nthree", 2, 2, "TWO")).toBe(
      "one\r\nTWO\r\nthree",
    );
  });

  it("preserves LF line endings", () => {
    expect(replaceLines("one\ntwo\nthree", 2, 2, "TWO")).toBe(
      "one\nTWO\nthree",
    );
  });

  it("normalizes a multi-line replacement to the file's own ending", () => {
    expect(replaceLines("one\r\ntwo\r\nthree", 2, 2, "a\nb")).toBe(
      "one\r\na\r\nb\r\nthree",
    );
  });
});

describe("linesMatch", () => {
  // A unit test proves something about the code; it proves nothing about the
  // user's note. This guard is what protects the vault at runtime.
  it("accepts a transform that preserved the line count", () => {
    expect(linesMatch("a\nb\nc", "a\nB\nc")).toBe(true);
  });

  it("rejects a transform that lost a line", () => {
    expect(linesMatch("a\nb\nc", "a\nbc")).toBe(false);
  });

  it("compares CRLF source against an LF-rendered body", () => {
    expect(linesMatch("a\r\nb\r\nc", "a\nb\nc")).toBe(true);
  });
});

function node(
  type: string,
  startLine: number | null,
  endLine?: number,
  children: unknown[] = [],
) {
  return {
    type,
    children,
    position:
      startLine === null
        ? undefined
        : { start: { line: startLine }, end: { line: endLine ?? startLine } },
  };
}

// react-markdown hands components hast nodes, not mdast ones: the type is
// always "element" and the kind lives in tagName. A rule written against mdast
// type names silently never fires.
function hast(
  tagName: string,
  startLine: number | null,
  endLine?: number,
  children: unknown[] = [],
) {
  return {
    type: "element",
    tagName,
    children,
    position:
      startLine === null
        ? undefined
        : { start: { line: startLine }, end: { line: endLine ?? startLine } },
  };
}

describe("blockRange with hast nodes", () => {
  it("stops a list item at its nested list", () => {
    const li = hast("li", 1, 5, [hast("p", 1, 1), hast("ul", 2, 5)]);

    expect(blockRange(li, 0)).toEqual({ startLine: 1, endLine: 1 });
  });

  it("stops a list item at a nested ordered list", () => {
    const li = hast("li", 4, 9, [hast("p", 4, 5), hast("ol", 6, 9)]);

    expect(blockRange(li, 0)).toEqual({ startLine: 4, endLine: 5 });
  });

  it("keeps the full range for a list item with no nested list", () => {
    expect(blockRange(hast("li", 3, 4, [hast("p", 3, 4)]), 0)).toEqual({
      startLine: 3,
      endLine: 4,
    });
  });
});

describe("blockRange", () => {
  it("maps a node's position through the frontmatter offset", () => {
    expect(blockRange(node("paragraph", 2, 4), 3)).toEqual({
      startLine: 5,
      endLine: 7,
    });
  });

  it("maps a single-line node", () => {
    expect(blockRange(node("heading", 1, 1), 0)).toEqual({
      startLine: 1,
      endLine: 1,
    });
  });

  // Display math, raw HTML blocks, link reference definitions, and generated
  // footnote nodes all reach the renderer without a position. They own no
  // lines and must render unchanged rather than becoming editable.
  it("returns null for a node with no position", () => {
    expect(blockRange(node("span", null), 0)).toBeNull();
  });

  // A list item must not swallow its sublist, or clicking the parent bullet
  // drops the whole nested list into raw markdown.
  it("stops a list item at the start of its first nested list", () => {
    const li = node("listItem", 1, 5, [
      node("paragraph", 1, 1),
      node("list", 2, 5),
    ]);

    expect(blockRange(li, 0)).toEqual({ startLine: 1, endLine: 1 });
  });

  it("stops a list item at a nested ordered list too", () => {
    const li = node("listItem", 4, 9, [
      node("paragraph", 4, 5),
      node("list", 6, 9),
    ]);

    expect(blockRange(li, 0)).toEqual({ startLine: 4, endLine: 5 });
  });

  it("keeps the full range for a list item with no nested list", () => {
    const li = node("listItem", 3, 4, [node("paragraph", 3, 4)]);

    expect(blockRange(li, 0)).toEqual({ startLine: 3, endLine: 4 });
  });

  it("keeps the full range for a multi-line paragraph", () => {
    expect(blockRange(node("paragraph", 7, 9), 0)).toEqual({
      startLine: 7,
      endLine: 9,
    });
  });
});
