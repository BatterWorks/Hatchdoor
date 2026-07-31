// The CodeMirror configuration behind an open block: how markdown source is
// coloured while it is being typed, and how the editor is stripped back so it
// looks like the paragraph it replaced rather than like a code editor.

import type { MarkdownConfig } from "@lezer/markdown";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { EditorView } from "@codemirror/view";
import { Tag, styleTags, tags } from "@lezer/highlight";

/**
 * List markers get their own tags so they can be styled apart from the rest of
 * the markdown syntax.
 *
 * The parser files every marker under one tag, which is the right default:
 * `**` and `#` are redundant with the bold and the heading size they produce,
 * so they can recede. A list marker is not. The drawn bullet is hidden while
 * its line is being edited, precisely so the typed marker can stand in its
 * place, which leaves the typed `-` as the only thing saying "this is still a
 * bullet". Dimmed to the same 65% as an asterisk, it reads as an empty line.
 */
const listMark = Tag.define();
const taskMark = Tag.define();

const listMarkerTags: MarkdownConfig = {
  props: [styleTags({ ListMark: listMark, TaskMarker: taskMark })],
};

/**
 * An indented list line is not an indented code block.
 *
 * The editor holds one block at a time, so a nested list item arrives as the
 * bare line `      2. Another sub-step`. Read on its own, four or more leading
 * spaces is CommonMark's indented code block, and the whole item would be
 * styled as code while it is being edited, in a note where it is plainly a
 * list. The parser cannot know better, because the list it belongs to is not
 * in the document it was given.
 *
 * Dropping the rule costs nothing here. A real indented code block is its own
 * unit and is edited as one, and fenced blocks are a different parser.
 */
const noIndentedCode: MarkdownConfig = { remove: ["IndentedCode"] };

/** The markdown language, with list markers tagged apart from other syntax. */
export const blockMarkdownExtensions = [listMarkerTags, noIndentedCode];

/**
 * Markdown syntax, styled as you type it.
 *
 * The markers stay on screen rather than being hidden or replaced. This is a
 * markdown editor over plain files, so the syntax is the thing being edited:
 * hiding `**` would mean the user could not see where emphasis starts, could
 * not delete it deliberately, and would have to guess why a stray asterisk
 * changed the look of a whole paragraph. Dimming them keeps them legible while
 * letting the prose sit in front.
 *
 * Everything here is a colour, a weight, or a family. Nothing changes the size
 * or spacing of text, because an open block sits inline with rendered blocks
 * above and below it, and reflowing on entry would shift the page under the
 * cursor.
 */
export const markdownHighlighting = syntaxHighlighting(
  HighlightStyle.define([
    // The syntax itself: asterisks, backticks, hashes, the brackets around a
    // link. Present, but behind the prose.
    { tag: tags.processingInstruction, color: "var(--muted)", opacity: "0.65" },
    // The list marker stands in for the bullet that is hidden while this line
    // is edited, so it carries that bullet's weight and its accent colour
    // rather than receding like the other syntax.
    { tag: listMark, color: "var(--hot)", opacity: "1", fontWeight: "600" },
    { tag: taskMark, color: "var(--hot)", opacity: "1", fontWeight: "600" },
    { tag: tags.strong, fontWeight: "700" },
    { tag: tags.emphasis, fontStyle: "italic" },
    { tag: tags.strikethrough, textDecoration: "line-through" },
    { tag: tags.heading, fontWeight: "700" },
    { tag: tags.monospace, fontFamily: "var(--font-mono)" },
    { tag: tags.link, color: "var(--hot)" },
    { tag: tags.url, color: "var(--muted)", opacity: "0.75" },
    { tag: tags.quote, color: "var(--muted)" },
  ]),
);

/**
 * Strips CodeMirror back to nothing so the block keeps the typography of what
 * it replaced.
 *
 * Metrics are inherited rather than set: a heading edits at heading size, a
 * list item at body size, and neither is named here. Transforms are the one
 * thing deliberately not inherited, because h1, h3, h6, callout titles and
 * table headers all uppercase their text, and an editor that inherited that
 * would show the typist a different string than the one being written to the
 * file.
 */
export const blockEditorTheme = EditorView.theme({
  "&": {
    backgroundColor: "transparent",
    color: "inherit",
    font: "inherit",
    letterSpacing: "inherit",
    textAlign: "inherit",
    // An opened block is scrolled into view by the least amount that makes it
    // visible, which lands it flush against the edge it came in from. A line
    // added at the end of a note is always just below the fold, so without
    // this it always arrives pinned to the very bottom of the screen with
    // nothing beneath it to write into. The larger figure is at the end, where
    // a phone's keyboard takes the bottom of the screen.
    scrollMarginBlock: "5rem 8rem",
  },
  "&.cm-focused": { outline: "none" },
  // overflow visible, so the block grows with its content instead of becoming
  // a small scrolling pane inside the note.
  ".cm-scroller": {
    font: "inherit",
    lineHeight: "inherit",
    overflow: "visible",
    alignItems: "stretch",
  },
  ".cm-content": {
    padding: "0",
    textTransform: "none",
    textIndent: "0",
    caretColor: "var(--hot)",
  },
  ".cm-line": { padding: "0" },
  ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--hot)" },
  "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection":
    {
      backgroundColor: "var(--sel, rgba(125, 125, 125, 0.3))",
    },
});
