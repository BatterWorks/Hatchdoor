import { useEffect, useRef } from "react";

import { defaultKeymap } from "@codemirror/commands";
import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
import { EditorState, Prec } from "@codemirror/state";
import { EditorView, keymap } from "@codemirror/view";

import { linePrefix } from "../../lib/linePrefix";
import {
  blockEditorTheme,
  blockMarkdownExtensions,
  markdownHighlighting,
} from "./blockEditorSetup";

export type UnitType =
  | "paragraph"
  | "heading"
  | "list item"
  | "quote"
  | "callout"
  | "code block"
  | "table row";

/**
 * The editor that replaces one rendered block while it is being edited.
 *
 * It is a CodeMirror instance rather than a textarea so markdown can be styled
 * as it is typed: `**word**` shows bold with its asterisks still visible. A
 * textarea cannot style part of its own text, and the usual workaround, a
 * styled mirror layer behind transparent text, does not survive contact with
 * bold, whose glyphs are wider than the regular ones the invisible text is
 * measured in. The caret would drift out of line with what is on screen.
 *
 * CodeMirror also owns the things that are easy to get wrong by hand and
 * invisible until someone hits them: IME composition, the mobile virtual
 * keyboard, selection, and undo inside a block.
 *
 * Structural editing is deliberately not CodeMirror's. Enter, Backspace at the
 * start, Tab and the arrow keys are bound to the same block operations as
 * before, which are pure functions over the whole document. CodeMirror only
 * ever edits the one block it holds.
 */
export function BlockInput({
  unitType,
  initialValue,
  initialCaret = null,
  onCommit,
  onEdit,
  onSplit,
  onMergeUp,
  onIndent,
  onOutdent,
  onMove,
}: {
  unitType: UnitType;
  initialValue: string;
  initialCaret?: number | null;
  onCommit: (text: string) => void;
  /** Fires on every keystroke so unflushed text is never only in this input. */
  onEdit?: (text: string) => void;
  onSplit?: (text: string, caret: number) => void;
  onMergeUp?: (text: string) => boolean;
  onIndent?: (text: string) => boolean;
  onOutdent?: (text: string) => boolean;
  onMove?: (text: string, direction: -1 | 1, column: number) => boolean;
}) {
  const hostRef = useRef<HTMLSpanElement | null>(null);
  const viewRef = useRef<EditorView | null>(null);
  const committedRef = useRef(false);
  const composingRef = useRef(false);

  // The handlers are rebuilt on every render of the parent, and the editor is
  // built once. Reading them through a ref keeps the keymap bound to the
  // current ones without tearing down the editor, which would lose the caret.
  const handlers = useRef({
    onCommit,
    onEdit,
    onSplit,
    onMergeUp,
    onIndent,
    onOutdent,
    onMove,
    unitType,
  });
  handlers.current = {
    onCommit,
    onEdit,
    onSplit,
    onMergeUp,
    onIndent,
    onOutdent,
    onMove,
    unitType,
  };

  useEffect(() => {
    const host = hostRef.current;
    if (!host) {
      return;
    }

    // Destroying the view blurs it, and a blur commits, which leaves the
    // block. Under StrictMode the first mount is immediately torn down and
    // remounted, so without this the block closes the instant it is opened.
    //
    // Nothing is lost by staying quiet here. A real unmount is always preceded
    // by a genuine blur: the pointer going down on another block moves focus
    // off this one before that block's click handler ever runs. Anything typed
    // has also already been reported through onEdit.
    let tearingDown = false;

    const commit = (view: EditorView) => {
      if (committedRef.current || tearingDown) {
        return;
      }
      committedRef.current = true;
      handlers.current.onCommit(view.state.doc.toString());
    };

    const view = new EditorView({
      parent: host,
      state: EditorState.create({
        doc: initialValue,
        selection: {
          anchor: Math.min(
            Math.max(initialCaret ?? initialValue.length, 0),
            initialValue.length,
          ),
        },
        extensions: [
          // GitHub-flavored, to match the remark-gfm the note is rendered
          // with. On the CommonMark default a task box, a strikethrough and a
          // table are not syntax at all, so the source would be styled as
          // something other than what it renders as.
          markdown({
            base: markdownLanguage,
            extensions: blockMarkdownExtensions,
          }),
          markdownHighlighting,
          blockEditorTheme,
          EditorView.lineWrapping,
          EditorView.contentAttributes.of({
            "aria-label": `Editing ${unitType}`,
            spellcheck: "true",
          }),
          // Highest precedence: these keys mean something structural here, and
          // the default keymap binds most of them to within-document motion.
          //
          // The keydown handler ahead of the keymap only records whether the
          // key belongs to an IME candidate window. CodeMirror tracks
          // composition through composition events, which is the accurate
          // signal for a real IME, but a key can report itself as composing
          // before that state is visible. Reading both and declining on either
          // keeps Enter and Escape belonging to the candidate window.
          Prec.highest([
            EditorView.domEventHandlers({
              keydown: (event) => {
                composingRef.current = event.isComposing;
                return false;
              },
            }),
            keymap.of(
              blockKeymap(handlers, commit, committedRef, composingRef),
            ),
          ]),
          keymap.of(defaultKeymap),
          EditorView.updateListener.of((update) => {
            if (update.docChanged) {
              handlers.current.onEdit?.(update.state.doc.toString());
              hangPrefix(view);
            }
          }),
          EditorView.domEventHandlers({
            blur: (_event, view) => {
              commit(view);
              return false;
            },
          }),
        ],
      }),
    });
    viewRef.current = view;

    view.focus();
    // Keeps the active line above the virtual keyboard on a phone.
    view.dom.scrollIntoView?.({ block: "nearest" });
    hangPrefix(view);

    return () => {
      tearingDown = true;
      view.destroy();
      viewRef.current = null;
    };
    // Deliberately mount-only: the document and caret are seeded once on
    // entry, and rerunning this would rebuild the editor mid-edit and yank the
    // caret back to where the block was opened.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // A span, not a div, because the block being replaced is often a p or a
  // heading, and those may only contain phrasing content. React validates what
  // it renders and would warn on a div there; the browser would be entitled to
  // reparent it. CodeMirror's own divs are appended to this host through DOM
  // APIs rather than by React, which is not subject to either. `.block-input`
  // carries display: block, so it lays out as the block it replaced.
  return (
    <span
      ref={hostRef}
      className={`block-input block-input-${unitType.replace(/\s+/g, "-")}`}
    />
  );
}

type Handlers = {
  onSplit?: (text: string, caret: number) => void;
  onMergeUp?: (text: string) => boolean;
  onIndent?: (text: string) => boolean;
  onOutdent?: (text: string) => boolean;
  onMove?: (text: string, direction: -1 | 1, column: number) => boolean;
  unitType: UnitType;
};

/**
 * The keys that mean something to the document rather than to this block.
 *
 * Each returns true only when it actually did something, so anything declined
 * falls through to CodeMirror's own binding: Tab outside a list still moves
 * focus, and an arrow key in the middle of a wrapped paragraph still just
 * moves the caret.
 */
function blockKeymap(
  handlers: { current: Handlers },
  commit: (view: EditorView) => void,
  committedRef: { current: boolean },
  composingRef: { current: boolean },
) {
  /**
   * Every structural key is gated on composition. With an IME active, Enter
   * and Escape belong to the candidate window, not to us: acting on them would
   * split a block in the middle of choosing a character.
   */
  const unlessComposing =
    (run: (view: EditorView) => boolean) => (view: EditorView) =>
      composingRef.current || view.composing ? false : run(view);
  // The op is about to rewrite the whole document, so this block's own blur
  // must not then write its stale text back over the result.
  const markCommitted = () => {
    committedRef.current = true;
  };
  const undoCommitted = () => {
    committedRef.current = false;
  };

  return [
    {
      key: "Escape",
      run: unlessComposing((view: EditorView) => {
        // Escape commits and leaves. Under autosave there is nothing to cancel
        // back to, so discarding here would throw away work with no way to get
        // it back. A deliberate break from Escape elsewhere in the app.
        commit(view);
        return true;
      }),
    },
    {
      key: "Enter",
      run: unlessComposing((view: EditorView) => {
        const { onSplit, unitType } = handlers.current;
        // Inside a fence or a table row a generic split would insert a line
        // before the closing fence or before the delimiter row and destroy the
        // block (D27), so Enter stays a plain line break there.
        if (unitType === "code block" || unitType === "table row" || !onSplit) {
          return false;
        }
        markCommitted();
        onSplit(view.state.doc.toString(), view.state.selection.main.head);
        return true;
      }),
    },
    {
      key: "Backspace",
      run: unlessComposing((view: EditorView) => {
        const { onMergeUp, unitType } = handlers.current;
        const range = view.state.selection.main;
        if (!range.empty || range.from !== 0) {
          return false;
        }
        if (
          unitType === "table row" ||
          unitType === "code block" ||
          !onMergeUp
        ) {
          return false;
        }
        markCommitted();
        if (onMergeUp(view.state.doc.toString())) {
          return true;
        }
        undoCommitted();
        return false;
      }),
    },
    {
      key: "Tab",
      run: unlessComposing((view: EditorView) =>
        runIndent(view, handlers, markCommitted, undoCommitted, true),
      ),
    },
    {
      key: "Shift-Tab",
      run: unlessComposing((view: EditorView) =>
        runIndent(view, handlers, markCommitted, undoCommitted, false),
      ),
    },
    {
      key: "ArrowUp",
      run: unlessComposing((view: EditorView) =>
        runMove(view, handlers, markCommitted, undoCommitted, -1),
      ),
    },
    {
      key: "ArrowDown",
      run: unlessComposing((view: EditorView) =>
        runMove(view, handlers, markCommitted, undoCommitted, 1),
      ),
    },
  ];
}

function runIndent(
  view: EditorView,
  handlers: { current: Handlers },
  markCommitted: () => void,
  undoCommitted: () => void,
  deeper: boolean,
): boolean {
  const { onIndent, onOutdent, unitType } = handlers.current;
  if (unitType !== "list item") {
    return false;
  }
  const handler = deeper ? onIndent : onOutdent;
  markCommitted();
  if (handler?.(view.state.doc.toString())) {
    return true;
  }
  undoCommitted();
  return false;
}

/**
 * Leave the block when the caret is already on its edge line, and only then.
 * Motion inside a block is CodeMirror's, including across the visual lines a
 * wrapped paragraph occupies.
 */
function runMove(
  view: EditorView,
  handlers: { current: Handlers },
  markCommitted: () => void,
  undoCommitted: () => void,
  direction: -1 | 1,
): boolean {
  const { onMove } = handlers.current;
  if (!onMove) {
    return false;
  }

  const { state } = view;
  const head = state.selection.main.head;
  const line = state.doc.lineAt(head);
  const leaving =
    direction === -1 ? line.number === 1 : line.number === state.doc.lines;
  if (!leaving) {
    return false;
  }

  // A wrapped line is several rows on screen but one line in the document, and
  // the caret should walk through those rows before leaving the block.
  const visual =
    direction === -1
      ? view.moveVertically(state.selection.main, false)
      : view.moveVertically(state.selection.main, true);
  if (visual.head !== head) {
    return false;
  }

  markCommitted();
  if (onMove(state.doc.toString(), direction, head - line.from)) {
    return true;
  }
  undoCommitted();
  return false;
}

/**
 * Hangs the line's markdown prefix into the gutter so the visible text does not
 * move when a block is entered.
 *
 * The prefix has to be *measured*: "- " is about 9px while the bullet marker it
 * replaces is inset 22.4px, so a fixed hang leaves the text 13px out. The
 * offset is clamped to the space the scroll pane actually has, which is 56px on
 * desktop but only 16px on a phone, so a long prefix never gets clipped.
 */
function hangPrefix(view: EditorView) {
  const el = view.dom;
  const prefix = linePrefix(view.state.doc.lineAt(0).text);
  if (!prefix) {
    el.style.marginLeft = "";
    el.style.width = "";
    return;
  }

  const content = view.contentDOM;
  const style = getComputedStyle(content);
  const width = measureText(prefix, style.font);
  if (width === null) {
    return;
  }

  const pane = el.closest("main");
  const available = pane
    ? parseFloat(getComputedStyle(pane).paddingLeft) || 0
    : 0;
  const existingInset = parseFloat(style.paddingLeft) || 0;
  const hang = Math.min(width, available + existingInset);

  el.style.marginLeft = `${-hang}px`;
  el.style.width = `calc(100% + ${hang}px)`;
}

let measureContext: CanvasRenderingContext2D | null | undefined;

function measureText(text: string, font: string): number | null {
  if (measureContext === undefined) {
    measureContext = document.createElement("canvas").getContext("2d");
  }
  if (!measureContext) {
    return null;
  }
  measureContext.font = font;
  return measureContext.measureText(text).width;
}
