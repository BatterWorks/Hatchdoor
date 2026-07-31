import { describe, expect, it } from "vitest";

import {
  indentListItem,
  mergeBlockUp,
  outdentListItem,
  splitBlock,
  exitEmptyListItem,
  insertParagraphAt,
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
      caretEndLine: 1,
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

describe("mergeBlockUp refuses to cross unowned lines", () => {
  // Raw HTML, link reference definitions and display math reach the renderer
  // with no node at all, so no block registers them. Merging across one
  // deletes it silently, then autosave persists the deletion.
  it("refuses across a raw HTML block", () => {
    expect(
      mergeBlockUp("para one\n\n<div>keep me</div>\n\npara two", r(5), r(1)),
    ).toBeNull();
  });

  it("refuses across a link reference definition", () => {
    expect(
      mergeBlockUp("para one\n\n[ref]: https://x.com\n\npara two", r(5), r(1)),
    ).toBeNull();
  });

  it("refuses across display math", () => {
    expect(
      mergeBlockUp("para one\n\n$$\nx=1\n$$\n\npara two", r(7), r(1)),
    ).toBeNull();
  });

  it("still merges across blank lines, which separate blocks rather than hide content", () => {
    expect(mergeBlockUp("one\n\ntwo", r(3), r(1))?.content).toBe("onetwo");
  });

  it("still merges adjacent lines", () => {
    expect(mergeBlockUp("- one\n- two", r(2), r(1))?.content).toBe("- onetwo");
  });
});

describe("block ops preserve CRLF", () => {
  it("splitBlock keeps CRLF throughout a multi-line block", () => {
    const out = splitBlock("alpha\r\nbeta\r\ngamma\r\n\r\nnext", r(1, 3), 2);

    expect(out.content).not.toMatch(/(?<!\r)\n/);
  });

  it("splitBlock keeps CRLF for a single-line block", () => {
    expect(splitBlock("- one\r\n- two", r(2), 5).content).toBe(
      "- one\r\n- two\r\n- ",
    );
  });

  it("mergeBlockUp keeps CRLF", () => {
    expect(mergeBlockUp("one\r\n\r\ntwo\r\nthree", r(3), r(1))?.content).toBe(
      "onetwo\r\nthree",
    );
  });

  it("indent and outdent keep CRLF", () => {
    expect(indentListItem("- one\r\n- two", r(2))?.content).toBe(
      "- one\r\n  - two",
    );
    expect(outdentListItem("- one\r\n  - two", r(2))?.content).toBe(
      "- one\r\n- two",
    );
  });

  it("toggleCheckbox keeps CRLF", () => {
    expect(toggleCheckbox("- [ ] a\r\n- [ ] b", 2)).toBe("- [ ] a\r\n- [x] b");
  });
});

describe("splitBlock caret range", () => {
  // The remainder of a split can itself span several lines, and the caller
  // makes the active range from these, so a single-line answer would open an
  // input showing only part of the block.
  it("reports the whole remainder when it spans several lines", () => {
    const out = splitBlock("one\ntwo\nthree", r(1, 3), 2);

    expect(out.caretLine).toBe(3);
    expect(out.caretEndLine).toBe(5);
  });

  it("reports a single line when the remainder is one line", () => {
    const out = splitBlock("- one\n- two", r(2), 5);

    expect(out.caretLine).toBe(3);
    expect(out.caretEndLine).toBe(3);
  });
});

describe("exitEmptyListItem", () => {
  it("lifts a nested empty item one level instead of leaving the list", () => {
    const out = exitEmptyListItem("- one\n  - ", r(2));

    expect(out).toEqual({
      content: "- one\n- ",
      caretLine: 2,
      caretEndLine: 2,
      caretOffset: 2,
    });
  });

  it("turns a top-level empty item into a paragraph below the list", () => {
    // Two blank lines, not one: the first ends the list, the second is where
    // the caret lands. One line would leave the caret on a lazy continuation
    // of the item above.
    const out = exitEmptyListItem("- one\n- ", r(2));

    expect(out).toEqual({
      content: "- one\n\n",
      caretLine: 3,
      caretEndLine: 3,
      caretOffset: 0,
    });
  });

  it("ends an empty task item the same way", () => {
    expect(exitEmptyListItem("- [ ] done\n- [ ] ", r(2))?.content).toBe(
      "- [ ] done\n\n",
    );
  });

  it("outdents a deeply nested empty item one level at a time", () => {
    expect(exitEmptyListItem("- a\n  - b\n    - ", r(3))?.content).toBe(
      "- a\n  - b\n  - ",
    );
  });

  it("refuses an item that still has text, which Enter should split", () => {
    expect(exitEmptyListItem("- one\n- two", r(2))).toBeNull();
  });

  it("refuses a line that is not a list item at all", () => {
    expect(exitEmptyListItem("a paragraph", r(1))).toBeNull();
  });

  it("treats trailing whitespace after the marker as empty", () => {
    expect(exitEmptyListItem("- one\n-   ", r(2))?.content).toBe("- one\n\n");
  });

  it("ends an empty ordered item without renumbering the list", () => {
    expect(exitEmptyListItem("1. one\n1. ", r(2))?.content).toBe("1. one\n\n");
  });

  it("leaves the lines around it untouched", () => {
    const out = exitEmptyListItem("intro\n\n- one\n- \n\nafter", r(4));

    expect(out?.content).toBe("intro\n\n- one\n\n\n\nafter");
    expect(out?.caretLine).toBe(5);
  });

  it("refuses a multi-line range, which an empty item never is", () => {
    expect(
      exitEmptyListItem("- one\n- ", { startLine: 1, endLine: 2 }),
    ).toBeNull();
  });
});

describe("insertParagraphAt", () => {
  it("opens a line in the gap between two paragraphs", () => {
    // one / blank / caret / blank / two: the gap's existing blank line becomes
    // the separator below, so only the one above has to be added.
    const out = insertParagraphAt("one\n\ntwo", 1);

    expect(out.content).toBe("one\n\n\n\ntwo");
    expect(out.caretLine).toBe(3);
    expect(out.caretOffset).toBe(0);
  });

  it("appends below the last block", () => {
    const out = insertParagraphAt("one", 1);

    expect(out.content).toBe("one\n\n");
    expect(out.caretLine).toBe(3);
  });

  it("opens a line above the first block", () => {
    const out = insertParagraphAt("one", 0);

    expect(out.content).toBe("\n\none");
    expect(out.caretLine).toBe(1);
  });

  it("does not pad where a blank line already separates", () => {
    const out = insertParagraphAt("one\n\n\ntwo", 2);

    expect(out.content).toBe("one\n\n\n\ntwo");
    expect(out.caretLine).toBe(3);
  });

  it("keeps the new line clear of a list above it", () => {
    // Without the padding the typed text becomes a lazy continuation of the
    // item and silently rejoins the bullet.
    const out = insertParagraphAt("- one\n- two", 2);

    expect(out.content).toBe("- one\n- two\n\n");
    expect(out.caretLine).toBe(4);
  });

  it("preserves CRLF line endings", () => {
    expect(insertParagraphAt("one\r\n\r\ntwo", 1).content).toBe(
      "one\r\n\r\n\r\n\r\ntwo",
    );
  });
});
