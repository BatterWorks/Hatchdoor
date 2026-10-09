// Editing commands the toolbar, the keyboard bar and the keymap share. Each
// is a plain function over the view, so one gesture has one implementation
// whether it came from a key, a button above the keyboard, or the floating
// toolbar.

import { Prec } from "@codemirror/state";
import { EditorView, keymap } from "@codemirror/view";

/**
 * Wrap the selection in `left`…`right`, or unwrap it when it already is. With
 * nothing selected the markers go in around the caret, so typing continues
 * inside them.
 */
export function wrapSelection(
  view: EditorView,
  left: string,
  right = left,
): boolean {
  const { from, to } = view.state.selection.main;
  const selected = view.state.sliceDoc(from, to);
  const before = view.state.sliceDoc(Math.max(0, from - left.length), from);
  const after = view.state.sliceDoc(to, to + right.length);
  if (before === left && after === right) {
    view.dispatch({
      changes: [
        { from: from - left.length, to: from, insert: "" },
        { from: to, to: to + right.length, insert: "" },
      ],
      selection: { anchor: from - left.length, head: to - left.length },
      userEvent: "input",
    });
  } else {
    view.dispatch({
      changes: { from, to, insert: `${left}${selected}${right}` },
      selection: {
        anchor: from + left.length,
        head: from + left.length + selected.length,
      },
      userEvent: "input",
    });
  }
  view.focus();
  return true;
}

/** `[text](url)` around the selection, with `url` selected for typing over. */
export function insertLink(view: EditorView): boolean {
  const { from, to } = view.state.selection.main;
  const text = view.state.sliceDoc(from, to) || "link text";
  const urlStart = from + text.length + 3;
  view.dispatch({
    changes: { from, to, insert: `[${text}](url)` },
    selection: { anchor: urlStart, head: urlStart + 3 },
    userEvent: "input",
  });
  view.focus();
  return true;
}

const LINE_PREFIX =
  /^(\s*)((?:[-*+]|\d+\.)\s+(?:\[[ xX]\]\s+)?|#{1,6}\s+|>\s*(?:\[![a-z]+\]\s*)?)?/;

/**
 * Replace the line's leading Markdown prefix (list marker, task box, heading
 * hashes, quote arrow) with `prefix`. The same prefix twice turns it off, so
 * a bullet button toggles.
 */
export function setLinePrefix(view: EditorView, prefix: string): boolean {
  const line = view.state.doc.lineAt(view.state.selection.main.head);
  const match = LINE_PREFIX.exec(line.text);
  const indent = match?.[1] ?? "";
  const existing = (match?.[0] ?? "").length;
  const next = (match?.[2] ?? "") === prefix ? "" : prefix;
  view.dispatch({
    changes: {
      from: line.from,
      to: line.from + existing,
      insert: indent + next,
    },
    userEvent: "input",
  });
  view.focus();
  return true;
}

/** Two spaces in or out at the start of every line the selection touches. */
export function indentLines(view: EditorView, deeper: boolean): boolean {
  const { from, to } = view.state.selection.main;
  const first = view.state.doc.lineAt(from).number;
  const last = view.state.doc.lineAt(to).number;
  const changes = [];
  for (let n = first; n <= last; n += 1) {
    const line = view.state.doc.line(n);
    if (deeper) {
      changes.push({ from: line.from, insert: "  " });
    } else {
      const spaces = /^ {1,2}/.exec(line.text)?.[0].length ?? 0;
      if (spaces > 0) {
        changes.push({ from: line.from, to: line.from + spaces });
      }
    }
  }
  if (changes.length === 0) {
    return false;
  }
  view.dispatch({ changes, userEvent: "input" });
  view.focus();
  return true;
}

/** Whether the caret's line is a list item, where Tab means indentation. */
function inListItem(view: EditorView): boolean {
  const line = view.state.doc.lineAt(view.state.selection.main.head);
  return /^\s*(?:[-*+]|\d+\.)\s/.test(line.text);
}

/**
 * Bold, italic, code, strikethrough and link on the usual keys, and Tab as
 * list indentation. Above the defaults, which bind Mod-i and Tab to motion.
 */
export const markKeymap = Prec.high(
  keymap.of([
    { key: "Mod-b", run: (view) => wrapSelection(view, "**") },
    { key: "Mod-i", run: (view) => wrapSelection(view, "*") },
    { key: "Mod-e", run: (view) => wrapSelection(view, "`") },
    { key: "Mod-Shift-x", run: (view) => wrapSelection(view, "~~") },
    { key: "Mod-k", run: insertLink },
    {
      key: "Tab",
      run: (view) => (inListItem(view) ? indentLines(view, true) : false),
    },
    {
      key: "Shift-Tab",
      run: (view) => (inListItem(view) ? indentLines(view, false) : false),
    },
  ]),
);
