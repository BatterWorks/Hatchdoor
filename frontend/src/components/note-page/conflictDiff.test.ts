import { describe, expect, it } from "vitest";

import { diffConflictLines } from "./conflictDiff";

describe("diffConflictLines", () => {
  it("marks unchanged context and changed disk/draft lines", () => {
    expect(diffConflictLines("# Home\nDisk", "# Home\nDraft")).toEqual([
      { kind: "same", text: "# Home" },
      { kind: "disk", text: "Disk" },
      { kind: "draft", text: "Draft" },
    ]);
  });
});
