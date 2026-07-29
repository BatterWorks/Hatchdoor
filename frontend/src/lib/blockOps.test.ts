import { describe, expect, it } from "vitest";

import {
  indentListItem,
  mergeBlockUp,
  outdentListItem,
  splitBlock,
  toggleCheckbox,
} from "./blockOps";

const r = (startLine: number, endLine = startLine) => ({ startLine, endLine });

describe("splitBlock", () => {
  it("splits a paragraph into two, separated by a blank line", () => {
    const out = splitBlock("hello world", r(1), 5);

    expect(out.content).toBe("hello\n\n world");
    expect(out.caretLine).toBe(3);
    expect(out.caretOffset).toBe(0);
  });

  it("continues a list item with the same marker", () => {
    const out = splitBlock("- one\n- two", r(2), 5);

    expect(out.content).toBe("- one\n- two\n- ");
    expect(out.caretLine).toBe(3);
    expect(out.caretOffset).toBe(2);
  });

  it("carries the text after the caret into the new item", () => {
    const out = splitBlock("- one two", r(1), 6);

    expect(out.content).toBe("- one \n- two");
  });

  it("preserves list indentation", () => {
    const out = splitBlock("  - nested", r(1), 10);

    expect(out.content).toBe("  - nested\n  - ");
  });

  it("continues an ordered list without renumbering", () => {
    // Renumbering would touch lines outside the edited range, and renderers
    // ignore the literal numbers anyway.
    const out = splitBlock("1. one\n1. two", r(2), 6);

    expect(out.content).toBe("1. one\n1. two\n1. ");
  });

  it("continues a task item unchecked, whatever the source was", () => {
    expect(splitBlock("- [x] done", r(1), 10).content).toBe(
      "- [x] done\n- [ ] ",
    );
    expect(splitBlock("- [ ] todo", r(1), 10).content).toBe(
      "- [ ] todo\n- [ ] ",
    );
  });

  it("keeps the quote prefix when splitting inside a callout", () => {
    const out = splitBlock("> quoted", r(1), 8);

    expect(out.content).toBe("> quoted\n> ");
  });

  it("keeps a nested quote prefix", () => {
    expect(splitBlock("> > deep", r(1), 8).content).toBe("> > deep\n> > ");
  });

  it("produces a bare paragraph below a heading", () => {
    const out = splitBlock("## A heading", r(1), 12);

    expect(out.content).toBe("## A heading\n\n");
  });

  it("leaves surrounding lines untouched", () => {
    const out = splitBlock("before\n\n- one\n\nafter", r(3), 5);

    expect(out.content).toBe("before\n\n- one\n- \n\nafter");
  });
});

describe("mergeBlockUp", () => {
  it("joins a list item into the one above it", () => {
    const out = mergeBlockUp("- one\n- two", r(2), r(1));

    expect(out).toEqual({
      content: "- onetwo",
      caretLine: 1,
      caretOffset: 5,
    });
  });

  it("joins a paragraph into the paragraph above, dropping the blank line", () => {
    const out = mergeBlockUp("one\n\ntwo", r(3), r(1));

    expect(out?.content).toBe("onetwo");
    expect(out?.caretOffset).toBe(3);
  });

  // Merging across a range no block owns would silently absorb or delete it.
  it("refuses when there is no previous unit", () => {
    expect(mergeBlockUp("- one\n- two", r(1), null)).toBeNull();
  });

  it("keeps the previous block's prefix", () => {
    const out = mergeBlockUp("> quote\n- item", r(2), r(1));

    expect(out?.content).toBe("> quoteitem");
  });

  it("leaves lines after the merge untouched", () => {
    const out = mergeBlockUp("- one\n- two\n- three", r(2), r(1));

    expect(out?.content).toBe("- onetwo\n- three");
  });
});

describe("indentListItem and outdentListItem", () => {
  it("indents a list item by two spaces", () => {
    expect(indentListItem("- one\n- two", r(2))?.content).toBe(
      "- one\n  - two",
    );
  });

  it("outdents an indented item", () => {
    expect(outdentListItem("- one\n  - two", r(2))?.content).toBe(
      "- one\n- two",
    );
  });

  it("refuses to outdent an item already at the left margin", () => {
    expect(outdentListItem("- one\n- two", r(2))).toBeNull();
  });

  it("refuses to indent the first item of a list", () => {
    // Nothing to nest under; markdown would just treat it as a new list.
    expect(indentListItem("- one\n- two", r(1))).toBeNull();
  });

  it("refuses on a line that is not a list item", () => {
    expect(indentListItem("one\ntwo", r(2))).toBeNull();
    expect(outdentListItem("  one", r(1))).toBeNull();
  });

  it("indents a task item without disturbing its checkbox", () => {
    expect(indentListItem("- one\n- [x] two", r(2))?.content).toBe(
      "- one\n  - [x] two",
    );
  });
});

describe("toggleCheckbox", () => {
  it("checks an unchecked task", () => {
    expect(toggleCheckbox("- [ ] task", 1)).toBe("- [x] task");
  });

  it("unchecks a checked task", () => {
    expect(toggleCheckbox("- [x] task", 1)).toBe("- [ ] task");
  });

  it("accepts an uppercase X", () => {
    expect(toggleCheckbox("- [X] task", 1)).toBe("- [ ] task");
  });

  it("leaves a line that is not a task alone", () => {
    expect(toggleCheckbox("- plain", 1)).toBe("- plain");
  });

  it("toggles only the named line", () => {
    expect(toggleCheckbox("- [ ] a\n- [ ] b", 2)).toBe("- [ ] a\n- [x] b");
  });

  it("keeps indentation", () => {
    expect(toggleCheckbox("  - [ ] nested", 1)).toBe("  - [x] nested");
  });
});
