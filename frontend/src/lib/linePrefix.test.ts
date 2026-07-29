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
});
