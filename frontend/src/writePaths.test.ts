import { describe, expect, it } from "vitest";

import { validateNotePath } from "./writePaths";

describe("validateNotePath", () => {
  it("accepts simple relative paths", () => {
    expect(validateNotePath("Projects/New Note.md")).toBeNull();
    expect(validateNotePath("Home.md")).toBeNull();
  });

  it("requires a value unless empty is allowed", () => {
    expect(validateNotePath("", { label: "Note path" })).toBe(
      "Note path is required.",
    );
    expect(validateNotePath("   ", { label: "Note path" })).toBe(
      "Note path is required.",
    );
    expect(validateNotePath("", { allowEmpty: true })).toBeNull();
    expect(validateNotePath("   ", { allowEmpty: true })).toBeNull();
  });

  it("rejects parent-directory traversal", () => {
    expect(validateNotePath("../secret.md")).toContain('must not contain ".."');
    expect(validateNotePath("Projects/../../etc/passwd")).toContain(
      'must not contain ".."',
    );
  });

  it("rejects absolute paths and backslashes", () => {
    expect(validateNotePath("/etc/passwd")).toContain("relative to the vault");
    expect(validateNotePath("Projects\\Note.md")).toContain("forward slashes");
  });

  it("rejects single-dot and control characters", () => {
    expect(validateNotePath("Projects/./Note.md")).toContain(
      'must not contain "."',
    );
    expect(validateNotePath("Note.md")).toContain("control characters");
  });
});
