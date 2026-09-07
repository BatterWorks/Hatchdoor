import { describe, expect, it } from "vitest";

import { linePrefix } from "./linePrefix";

describe("linePrefix", () => {
  it("finds a bullet marker", () => {
    expect(linePrefix("- item")).toBe("- ");
  });

  it("finds an indented bullet marker", () => {
    expect(linePrefix("  - nested")).toBe("  - ");
  });

  it("finds an ordered marker", () => {
    expect(linePrefix("12. item")).toBe("12. ");
  });

  it("includes a task box", () => {
    expect(linePrefix("- [x] done")).toBe("- [x] ");
  });

  it("finds heading hashes", () => {
    expect(linePrefix("### Heading")).toBe("### ");
  });

  it("finds quote arrows, including nested ones", () => {
    expect(linePrefix("> quoted")).toBe("> ");
    expect(linePrefix("> > deep")).toBe("> > ");
  });

  it("is empty for a plain paragraph", () => {
    expect(linePrefix("just prose")).toBe("");
  });

  it("does not treat a hyphen inside text as a marker", () => {
    expect(linePrefix("well-formed prose")).toBe("");
  });

  it("finds a bare indent on a continuation line", () => {
    expect(linePrefix("  continues here")).toBe("  ");
  });

  it("finds the wider bare indent of a nested continuation line", () => {
    expect(linePrefix("      continues deeper")).toBe("      ");
  });

  it("counts a tab indent", () => {
    expect(linePrefix("\tcontinues here")).toBe("\t");
  });

  it("finds an indented heading", () => {
    expect(linePrefix("  # Heading")).toBe("  # ");
  });

  it("finds an indented quote", () => {
    expect(linePrefix("  > quoted")).toBe("  > ");
  });

  it("finds an indented task box", () => {
    expect(linePrefix("  - [ ] todo")).toBe("  - [ ] ");
  });

  it("takes a whitespace-only line as invisible throughout", () => {
    expect(linePrefix("   ")).toBe("   ");
  });

  it("stops at the indent when what follows is not a marker", () => {
    expect(linePrefix("  well-formed prose")).toBe("  ");
  });

  // Only a space or a tab separates a marker from its content. A no-break
  // space renders as a visible glyph, so hanging it into the gutter would pull
  // text that is actually on screen.
  it("does not count a no-break space as marker separation", () => {
    expect(linePrefix("-\u00a0tight")).toBe("");
    expect(linePrefix("#\u00a0tight")).toBe("");
  });

  it("does not count a no-break space as indentation", () => {
    expect(linePrefix("\u00a0 leading")).toBe("");
  });
});
