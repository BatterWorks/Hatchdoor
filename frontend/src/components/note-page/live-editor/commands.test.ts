import { describe, expect, it } from "vitest";

import { EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";

import {
  indentLines,
  insertLink,
  setLinePrefix,
  wrapSelection,
} from "./commands";

function editor(doc: string, anchor: number, head = anchor) {
  return new EditorView({
    state: EditorState.create({ doc, selection: { anchor, head } }),
    parent: document.body,
  });
}

describe("wrapSelection", () => {
  it("wraps the selection and unwraps it on the second press", () => {
    const view = editor("plain word here", 6, 10);
    wrapSelection(view, "**");
    expect(view.state.doc.toString()).toBe("plain **word** here");
    expect(
      view.state.sliceDoc(
        view.state.selection.main.from,
        view.state.selection.main.to,
      ),
    ).toBe("word");
    wrapSelection(view, "**");
    expect(view.state.doc.toString()).toBe("plain word here");
    view.destroy();
  });

  it("puts an empty pair around the caret so typing lands inside", () => {
    const view = editor("ab", 1);
    wrapSelection(view, "`");
    expect(view.state.doc.toString()).toBe("a``b");
    expect(view.state.selection.main.head).toBe(2);
    view.destroy();
  });
});

describe("insertLink", () => {
  it("selects the placeholder url so it can be typed over", () => {
    const view = editor("see docs", 4, 8);
    insertLink(view);
    expect(view.state.doc.toString()).toBe("see [docs](url)");
    expect(
      view.state.sliceDoc(
        view.state.selection.main.from,
        view.state.selection.main.to,
      ),
    ).toBe("url");
    view.destroy();
  });
});

describe("setLinePrefix", () => {
  it("swaps one prefix for another and toggles the same one off", () => {
    const view = editor("- item", 3);
    setLinePrefix(view, "- [ ] ");
    expect(view.state.doc.toString()).toBe("- [ ] item");
    setLinePrefix(view, "- [ ] ");
    expect(view.state.doc.toString()).toBe("item");
    setLinePrefix(view, "## ");
    expect(view.state.doc.toString()).toBe("## item");
    view.destroy();
  });

  it("keeps the line's indentation", () => {
    const view = editor("  - nested", 5);
    setLinePrefix(view, "1. ");
    expect(view.state.doc.toString()).toBe("  1. nested");
    view.destroy();
  });
});

describe("indentLines", () => {
  it("indents and outdents every line the selection touches by two spaces", () => {
    const view = editor("- a\n- b\n- c", 0, 6);
    indentLines(view, true);
    expect(view.state.doc.toString()).toBe("  - a\n  - b\n- c");
    indentLines(view, false);
    expect(view.state.doc.toString()).toBe("- a\n- b\n- c");
    expect(indentLines(view, false)).toBe(false);
    view.destroy();
  });
});
