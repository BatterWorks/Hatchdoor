// Obsidian's highlight colours in the editor (#577): a `==🔴text==` takes
// the colour its emoji names, and the emoji is hidden until the caret is on
// its line, the way the inline preview hides the `==` around it. The file
// keeps the emoji; only the drawing changes.
//
// The inline preview of `@atomic-editor/editor` draws every highlight in one
// tint. This layers over it instead of changing the package: a mark on the
// highlight's text carries the colour class, and a replace hides the emoji.
//
// Table cells are drawn by the package's table widget, outside the
// document's decorations, so a second pass colours their highlights in the
// DOM. It wraps the emoji in a span that CSS hides, which leaves the cell's
// text, the source the widget reads back, unchanged.

import { ensureSyntaxTree, syntaxTree } from "@codemirror/language";
import type { EditorState, Extension } from "@codemirror/state";
import {
  Decoration,
  ViewPlugin,
  type DecorationSet,
  type EditorView,
} from "@codemirror/view";

import {
  highlightColourMarker,
  type HighlightColour,
} from "../../../lib/markdownSyntax";
import { caretAwareField, caretTouches } from "./focusState";

type ColouredHighlight = {
  colour: HighlightColour;
  /** The highlight's text, inside its signs, emoji included. */
  textFrom: number;
  textTo: number;
  /** Where the emoji ends. */
  emojiTo: number;
};

function colouredHighlightsIn(state: EditorState): ColouredHighlight[] {
  const found: ColouredHighlight[] = [];
  // What is parsed so far when the whole note takes longer; the field looks
  // again as the parser goes on (`refindOnParse`).
  const tree =
    ensureSyntaxTree(state, state.doc.length, 50) ?? syntaxTree(state);
  tree.iterate({
    enter: (node) => {
      if (node.name !== "Highlight") {
        return;
      }
      const marks = node.node.getChildren("HighlightMark");
      if (marks.length < 2) {
        return;
      }
      const textFrom = marks[0].to;
      const textTo = marks[marks.length - 1].from;
      // Two code units are enough to hold any of the six emoji.
      const marker = highlightColourMarker(
        state.doc.sliceString(textFrom, Math.min(textFrom + 2, textTo)),
      );
      if (!marker) {
        return;
      }
      found.push({
        colour: marker.colour,
        textFrom,
        textTo,
        emojiTo: textFrom + marker.emoji.length,
      });
    },
  });
  return found;
}

function decorate(
  state: EditorState,
  highlights: ColouredHighlight[],
  focused: boolean,
): DecorationSet {
  const ranges = [];
  for (const highlight of highlights) {
    ranges.push(
      Decoration.mark({
        class: `live-editor-highlight live-editor-highlight-${highlight.colour}`,
      }).range(highlight.textFrom, highlight.textTo),
    );
    // The inline preview's rule for the `==`: shown while the focused
    // caret is on the line, hidden otherwise.
    const line = state.doc.lineAt(highlight.textFrom);
    if (!(focused && caretTouches(state, line.from, line.to))) {
      ranges.push(
        Decoration.replace({}).range(highlight.textFrom, highlight.emojiTo),
      );
    }
  }
  return Decoration.set(ranges, true);
}

const MARKER_CLASS = "live-editor-highlight-marker";

/** Colour the highlights the table widget drew in `root`'s cells. A cell
 * holding the caret keeps its colour but shows its emoji, as the cell shows
 * its source while being typed in; the emoji is wrapped once the caret
 * leaves, since splitting the text under the caret would move it. */
export function colourTableCells(root: ParentNode): void {
  const highlights = root.querySelectorAll<HTMLElement>(
    ".cm-atomic-table-cell-source .cm-atomic-highlight",
  );
  for (const highlight of highlights) {
    if (highlight.dataset.highlightColourDone !== undefined) {
      continue;
    }
    const first = highlight.firstChild;
    const marker =
      first?.nodeType === Node.TEXT_NODE
        ? highlightColourMarker(first.textContent ?? "")
        : null;
    if (!marker || !first) {
      highlight.dataset.highlightColourDone = "";
      continue;
    }
    highlight.classList.add(
      "live-editor-highlight",
      `live-editor-highlight-${marker.colour}`,
    );
    const source = highlight.closest(".cm-atomic-table-cell-source");
    if (source?.contains(source.ownerDocument.activeElement)) {
      continue;
    }
    const emoji = document.createElement("span");
    emoji.className = MARKER_CLASS;
    emoji.textContent = marker.emoji;
    first.textContent = (first.textContent ?? "").slice(marker.emoji.length);
    highlight.insertBefore(emoji, first);
    highlight.dataset.highlightColourDone = "";
  }
}

const tableCellColours = ViewPlugin.fromClass(
  class {
    observer: MutationObserver;
    view: EditorView;

    constructor(view: EditorView) {
      this.view = view;
      // The widget redraws a cell on each keystroke and on focus changes,
      // outside the editor's own updates, so the DOM is what says when.
      this.observer = new MutationObserver(() =>
        colourTableCells(view.contentDOM),
      );
      this.observer.observe(view.contentDOM, {
        childList: true,
        subtree: true,
      });
      view.contentDOM.addEventListener("focusout", this.onFocusOut);
      colourTableCells(view.contentDOM);
    }

    // The cell that held the caret is coloured once it has let go of it.
    onFocusOut = () => {
      queueMicrotask(() => colourTableCells(this.view.contentDOM));
    };

    destroy() {
      this.observer.disconnect();
      this.view.contentDOM.removeEventListener("focusout", this.onFocusOut);
    }
  },
);

export const highlightColours: Extension = [
  caretAwareField<ColouredHighlight[]>({
    find: colouredHighlightsIn,
    decorate,
    refindOnParse: true,
  }),
  tableCellColours,
];
