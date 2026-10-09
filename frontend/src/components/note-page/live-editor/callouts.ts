// Obsidian callouts in the editor (#544): a block quote opening with
// `[!kind]` takes the kind's accent on every line, and its first line reads
// as the title the reading view draws, the kind's name or the text after
// the marker, with the marker itself hidden until the caret lands on that
// line. The body lines stay editable prose; only the marker is replaced,
// so the rest of the quote is still the file's text under the caret.
//
// The per-kind accents are the reading view's own `.callout-<kind>` rules,
// which set `--callout-accent`: each line carries that class and the
// editor's own rule draws the spine and tint from the variable.

import { ensureSyntaxTree } from "@codemirror/language";
import {
  RangeSetBuilder,
  type EditorState,
  type Extension,
} from "@codemirror/state";
import { Decoration, WidgetType, type DecorationSet } from "@codemirror/view";

import { caretAwareField, caretTouches } from "./focusState";

// The same marker the reading view recognises, after the quote mark.
const CALLOUT_MARKER = /^(\s*>\s*)(\[!([A-Za-z0-9_-]+)\][+-]?)([ \t]*)(.*)$/;

class CalloutTitleWidget extends WidgetType {
  readonly title: string;

  constructor(title: string) {
    super();
    this.title = title;
  }

  eq(other: CalloutTitleWidget) {
    return other.title === this.title;
  }

  toDOM() {
    const span = document.createElement("span");
    span.className = "live-editor-callout-kind";
    span.textContent = this.title;
    return span;
  }

  ignoreEvent() {
    return false;
  }
}

type Callout = {
  kind: string;
  firstLine: number;
  lastLine: number;
  /** The marker's range on the title line, hidden while the caret is away. */
  markerFrom: number;
  markerTo: number;
  /** The kind's name, drawn when the line carries no title of its own. */
  title: string | null;
};

function calloutsIn(state: EditorState): Callout[] {
  const found: Callout[] = [];
  const tree = ensureSyntaxTree(state, state.doc.length, 50);
  tree?.iterate({
    enter: (node) => {
      if (node.name !== "Blockquote") {
        return;
      }
      const first = state.doc.lineAt(node.from);
      const match = CALLOUT_MARKER.exec(first.text);
      if (!match) {
        return;
      }
      const kind = match[3].toLowerCase();
      const custom = match[5].trim();
      const markerFrom = first.from + match[1].length;
      found.push({
        kind,
        firstLine: first.number,
        lastLine: state.doc.lineAt(node.to).number,
        markerFrom,
        // With a title of its own the spaces after the marker go with it,
        // so the title starts where the marker was.
        markerTo: markerFrom + match[2].length + (custom ? match[4].length : 0),
        title: custom ? null : kind[0].toUpperCase() + kind.slice(1),
      });
    },
  });
  return found;
}

function decorate(
  state: EditorState,
  callouts: Callout[],
  focused: boolean,
): DecorationSet {
  const ranges: Array<{ from: number; to: number; decoration: Decoration }> =
    [];
  for (const callout of callouts) {
    for (let n = callout.firstLine; n <= callout.lastLine; n += 1) {
      const line = state.doc.line(n);
      const classes = ["live-editor-callout", `callout-${callout.kind}`];
      if (n === callout.firstLine) {
        classes.push("live-editor-callout-title");
      } else {
        classes.push("live-editor-callout-body");
      }
      if (n === callout.lastLine) {
        classes.push("live-editor-callout-last");
      }
      ranges.push({
        from: line.from,
        to: line.from,
        decoration: Decoration.line({ class: classes.join(" ") }),
      });
    }
    const titleLine = state.doc.line(callout.firstLine);
    const caretOnTitle =
      focused && caretTouches(state, titleLine.from, titleLine.to);
    if (!caretOnTitle) {
      ranges.push({
        from: callout.markerFrom,
        to: callout.markerTo,
        decoration: Decoration.replace(
          callout.title
            ? { widget: new CalloutTitleWidget(callout.title) }
            : {},
        ),
      });
    }
  }
  ranges.sort(
    (a, b) =>
      a.from - b.from ||
      a.decoration.startSide - b.decoration.startSide ||
      a.to - b.to,
  );
  const builder = new RangeSetBuilder<Decoration>();
  for (const range of ranges) {
    builder.add(range.from, range.to, range.decoration);
  }
  return builder.finish();
}

export const callouts: Extension = caretAwareField<Callout[]>({
  find: calloutsIn,
  decorate,
});
