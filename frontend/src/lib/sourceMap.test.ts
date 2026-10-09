import { describe, expect, it } from "vitest";

import { detectLineEnding, frontmatterLineOffset } from "./sourceMap";

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
