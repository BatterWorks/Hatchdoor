import { describe, expect, it } from "vitest";

import { diffConflictLines, type ConflictDiffLine } from "./conflictDiff";

/** Every line the diff accounts for, as the side(s) it belongs to. */
function rebuild(lines: ConflictDiffLine[], side: "disk" | "draft"): string[] {
  return lines.flatMap((line) =>
    line.kind === "same" || line.kind === side ? [line.text] : [],
  );
}

describe("diffConflictLines", () => {
  it("marks unchanged context and changed disk/draft lines", () => {
    expect(diffConflictLines("# Home\nDisk", "# Home\nDraft")).toEqual([
      { kind: "same", text: "# Home" },
      { kind: "disk", text: "Disk" },
      { kind: "draft", text: "Draft" },
    ]);
  });

  // #331: the old walk compared line i with line i, so one line added at the
  // top put every later pair out of step and the whole note read as changed.
  it("keeps the lines after an insertion aligned", () => {
    const draft = ["# Home", "one", "two", "three"].join("\n");
    const disk = ["tags: [new]", "# Home", "one", "two", "three"].join("\n");

    expect(diffConflictLines(disk, draft)).toEqual([
      { kind: "disk", text: "tags: [new]" },
      { kind: "same", text: "# Home" },
      { kind: "same", text: "one" },
      { kind: "same", text: "two" },
      { kind: "same", text: "three" },
    ]);
  });

  it("aligns edits on both sides of a shared middle", () => {
    const disk = ["a", "disk only", "b", "c", "d", "e"].join("\n");
    const draft = ["a", "b", "c", "d", "draft only", "e"].join("\n");

    expect(diffConflictLines(disk, draft)).toEqual([
      { kind: "same", text: "a" },
      { kind: "disk", text: "disk only" },
      { kind: "same", text: "b" },
      { kind: "same", text: "c" },
      { kind: "same", text: "d" },
      { kind: "draft", text: "draft only" },
      { kind: "same", text: "e" },
    ]);
  });

  it("folds long unchanged runs down to the context around each change", () => {
    const body = Array.from({ length: 40 }, (_, index) => `line ${index + 1}`);
    const draft = [...body];
    draft[19] = "line 20, edited";

    const diff = diffConflictLines(body.join("\n"), draft.join("\n"));

    expect(diff).toEqual([
      { kind: "skip", count: 16 },
      { kind: "same", text: "line 17" },
      { kind: "same", text: "line 18" },
      { kind: "same", text: "line 19" },
      { kind: "disk", text: "line 20" },
      { kind: "draft", text: "line 20, edited" },
      { kind: "same", text: "line 21" },
      { kind: "same", text: "line 22" },
      { kind: "same", text: "line 23" },
      { kind: "skip", count: 17 },
    ]);
  });

  it("accounts for every line of both versions", () => {
    const disk = "a\nb\nc\nd\ne\nf\ng".split("\n");
    const draft = "a\nx\nc\ny\nz\nf\ng\nh".split("\n");

    const diff = diffConflictLines(disk.join("\n"), draft.join("\n"));

    expect(rebuild(diff, "disk")).toEqual(disk);
    expect(rebuild(diff, "draft")).toEqual(draft);
  });

  it("still accounts for every line when the versions share almost nothing", () => {
    const disk = Array.from({ length: 3000 }, (_, index) => `disk ${index}`);
    const draft = Array.from({ length: 3000 }, (_, index) => `draft ${index}`);

    const diff = diffConflictLines(disk.join("\n"), draft.join("\n"));

    expect(rebuild(diff, "disk")).toEqual(disk);
    expect(rebuild(diff, "draft")).toEqual(draft);
  });
});
