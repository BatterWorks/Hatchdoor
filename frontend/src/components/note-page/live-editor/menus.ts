// The three pop-ups the editor shows: the `/` menu at the start of a line,
// `[[` completion over the Vault's note titles, and the floating toolbar over
// a selection. The first two are one CodeMirror completion source each; the
// toolbar is a tooltip the selection drives.

import {
  autocompletion,
  completionKeymap,
  type Completion,
  type CompletionContext,
  type CompletionResult,
} from "@codemirror/autocomplete";
import {
  EditorState,
  Prec,
  StateField,
  type Extension,
} from "@codemirror/state";
import {
  EditorView,
  keymap,
  showTooltip,
  type Tooltip,
} from "@codemirror/view";

import { searchPanelOpen } from "@codemirror/search";
import type { ExplorerNote } from "../../../types";
import {
  CHECKLIST_PATH,
  CODE_PATH,
  FORMAT_BOLD_PATH,
  FORMAT_H2_PATH,
  FORMAT_INK_HIGHLIGHTER_PATH,
  FORMAT_ITALIC_PATH,
  FORMAT_LIST_BULLETED_PATH,
  FORMAT_STRIKETHROUGH_PATH,
  LINK_PATH,
  createIconElement,
} from "../../iconPaths";
import { insertLink, setLinePrefix, wrapSelection } from "./commands";

/* ── `/` menu ─────────────────────────────────────────────────────────── */

type SlashCommand = {
  label: string;
  detail: string;
  /** A line prefix the command sets, or */
  prefix?: string;
  /** a whole block it inserts in place of the line. */
  block?: string;
  /** Caret offset into `block`; end of it when absent. */
  caret?: number;
};

const SLASH_COMMANDS: SlashCommand[] = [
  { label: "Text", detail: "plain paragraph", prefix: "" },
  { label: "Heading 1", detail: "#", prefix: "# " },
  { label: "Heading 2", detail: "##", prefix: "## " },
  { label: "Heading 3", detail: "###", prefix: "### " },
  { label: "Bulleted list", detail: "-", prefix: "- " },
  { label: "Numbered list", detail: "1.", prefix: "1. " },
  { label: "To-do", detail: "- [ ]", prefix: "- [ ] " },
  { label: "Quote", detail: ">", prefix: "> " },
  { label: "Callout", detail: "> [!note]", prefix: "> [!note] " },
  { label: "Code block", detail: "```", block: "```\n\n```", caret: 4 },
  { label: "Divider", detail: "---", block: "---\n" },
  {
    label: "Table",
    detail: "3 columns",
    block: "| Column | Column | Column |\n| --- | --- | --- |\n|  |  |  |\n",
    caret: 2,
  },
];

function slashSource(context: CompletionContext): CompletionResult | null {
  const line = context.state.doc.lineAt(context.pos);
  const before = line.text.slice(0, context.pos - line.from);
  // Only a slash that opens the line is a command; one inside prose is text.
  if (!/^\s*\/[\w ]*$/.test(before)) {
    return null;
  }
  const slashAt = line.from + before.indexOf("/");
  return {
    // The text CodeMirror filters on starts after the slash.
    from: slashAt + 1,
    to: context.pos,
    filter: true,
    validFor: /^[\w ]*$/,
    options: SLASH_COMMANDS.map((command, index) => ({
      label: command.label,
      detail: command.detail,
      type: "keyword",
      // Keep the written order rather than the alphabetical one.
      boost: -index,
      apply: (view: EditorView, _c: Completion, _from: number, to: number) => {
        const insert = command.block ?? command.prefix ?? "";
        view.dispatch({
          changes: { from: line.from, to, insert },
          selection: { anchor: line.from + (command.caret ?? insert.length) },
          userEvent: "input",
        });
      },
    })),
  };
}

/* ── `[[` completion ──────────────────────────────────────────────────── */

/**
 * Typing `[[` offers the Vault's note titles. Choosing one writes the link in
 * the Vault's own style through `formatNoteLink` (ADR-33), replacing the
 * brackets typed so far and the closing pair the bracket closer added.
 */
function wikiSource(
  candidates: () => ExplorerNote[],
  formatNoteLink: (note: ExplorerNote) => string,
) {
  return (context: CompletionContext): CompletionResult | null => {
    const match = context.matchBefore(/\[\[([^\]]*)$/);
    if (!match) {
      return null;
    }
    const query = match.text.slice(2).toLowerCase();
    const options = candidates()
      .filter((note) => note.title.toLowerCase().includes(query))
      .slice(0, 12)
      .map((note) => ({
        label: note.title,
        type: "text",
        apply: (view: EditorView) => {
          const closing = view.state.sliceDoc(context.pos, context.pos + 2);
          const to = closing === "]]" ? context.pos + 2 : context.pos;
          const insert = formatNoteLink(note);
          view.dispatch({
            changes: { from: match.from, to, insert },
            selection: { anchor: match.from + insert.length },
            userEvent: "input",
          });
        },
      }));
    return { from: match.from + 2, to: context.pos, options, filter: false };
  };
}

export function completionMenus(
  candidates: () => ExplorerNote[],
  formatNoteLink: (note: ExplorerNote) => string,
): Extension {
  return [
    autocompletion({
      activateOnTyping: true,
      icons: false,
      // The list-continuation keymap binds Enter ahead of the completion's,
      // so the completion keys go in at the top instead.
      defaultKeymap: false,
      override: [slashSource, wikiSource(candidates, formatNoteLink)],
    }),
    Prec.highest(keymap.of(completionKeymap)),
  ];
}

/* ── Floating toolbar ─────────────────────────────────────────────────── */

const TOOLBAR_ITEMS: Array<
  | {
      icon: string;
      title: string;
      run: (view: EditorView) => boolean;
    }
  | "gap"
> = [
  {
    icon: FORMAT_BOLD_PATH,
    title: "Bold (Ctrl+B)",
    run: (v) => wrapSelection(v, "**"),
  },
  {
    icon: FORMAT_ITALIC_PATH,
    title: "Italic (Ctrl+I)",
    run: (v) => wrapSelection(v, "*"),
  },
  {
    icon: FORMAT_STRIKETHROUGH_PATH,
    title: "Strikethrough",
    run: (v) => wrapSelection(v, "~~"),
  },
  {
    icon: CODE_PATH,
    title: "Code (Ctrl+E)",
    run: (v) => wrapSelection(v, "`"),
  },
  {
    icon: FORMAT_INK_HIGHLIGHTER_PATH,
    title: "Highlight",
    run: (v) => wrapSelection(v, "=="),
  },
  { icon: LINK_PATH, title: "Link (Ctrl+K)", run: insertLink },
  // What the line is, after what the selection is. The `/` menu only opens
  // on a line being started, so a line already written had no way to become
  // a heading or a to-do short of typing the marker (the keyboard bar on a
  // phone has had these three all along).
  "gap",
  {
    icon: FORMAT_H2_PATH,
    title: "Heading",
    run: (v) => setLinePrefix(v, "## "),
  },
  {
    icon: FORMAT_LIST_BULLETED_PATH,
    title: "Bullet",
    run: (v) => setLinePrefix(v, "- "),
  },
  {
    icon: CHECKLIST_PATH,
    title: "To-do",
    run: (v) => setLinePrefix(v, "- [ ] "),
  },
];

function selectionTooltip(state: EditorState): Tooltip | null {
  const range = state.selection.main;
  if (range.empty) {
    return null;
  }
  // Stepping through matches selects each one; that is finding, not a
  // selection to format.
  if (searchPanelOpen(state)) {
    return null;
  }
  // A selection across lines is usually a cut or a move, not a format.
  if (
    state.doc.lineAt(range.from).number !== state.doc.lineAt(range.to).number
  ) {
    return null;
  }
  return {
    pos: range.from,
    end: range.to,
    above: true,
    strictSide: true,
    arrow: false,
    create: (view) => {
      const dom = document.createElement("div");
      dom.className = "live-editor-toolbar";
      dom.setAttribute("role", "toolbar");
      dom.setAttribute("aria-label", "Formatting");
      for (const item of TOOLBAR_ITEMS) {
        if (item === "gap") {
          const gap = document.createElement("span");
          gap.className = "live-editor-toolbar-gap";
          gap.setAttribute("aria-hidden", "true");
          dom.appendChild(gap);
          continue;
        }
        const button = document.createElement("button");
        button.type = "button";
        button.appendChild(createIconElement(item.icon));
        button.title = item.title;
        button.setAttribute("aria-label", item.title);
        // mousedown, so the click does not first collapse the selection.
        button.addEventListener("mousedown", (event) => {
          event.preventDefault();
          item.run(view);
        });
        dom.appendChild(button);
      }
      // Clear of the line it formats: flush against the selection it hid
      // the ascenders of the words either side.
      return { dom, offset: { x: 0, y: 10 } };
    },
  };
}

/**
 * Desktop only (#540): on touch the OS draws its own Cut/Copy/Paste callout
 * in the same spot, and the keyboard bar carries the formatting instead.
 */
export const selectionToolbar: Extension = StateField.define<Tooltip | null>({
  create: selectionTooltip,
  update(value, tr) {
    if (!tr.docChanged && !tr.selection) {
      return value;
    }
    return selectionTooltip(tr.state);
  },
  provide: (field) => showTooltip.from(field),
});
