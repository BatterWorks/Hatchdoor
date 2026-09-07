// Turns a browser caret position into an offset in the markdown source.
//
// `caretPositionFromPoint` and `caretRangeFromPoint` both report a position
// inside the text node under the pointer, while `caretMap` maps an offset
// measured across a block's whole rendered text. The two readings coincide only
// when a block renders as a single text node, which is why plain prose has
// always worked and anything holding a link, bold, italic, or inline code has
// not. This module is the conversion between them.

import { sourceOffsetForRenderedOffset } from "./caretMap";

/**
 * The source offset for a caret the browser reported at `offsetInNode`
 * characters into `node`, or null when no offset can be trusted.
 *
 * Null rather than a guess in the three cases where the reported position says
 * nothing about this block: no node at all, an element node, where the offset
 * counts children rather than characters, and a node this block does not
 * contain, which both browser APIs can report because they answer for the whole
 * document and a click can land on padding or on a neighbour. The caller opens
 * the block with its default caret instead.
 */
export function sourceOffsetForCaretPoint(
  root: Element | null,
  node: Node | null,
  offsetInNode: number,
  source: string,
): number | null {
  if (!root || !node || node.nodeType !== Node.TEXT_NODE) {
    return null;
  }

  const walker = root.ownerDocument.createTreeWalker(
    root,
    NodeFilter.SHOW_TEXT,
  );
  let before = 0;
  for (let text = walker.nextNode(); text; text = walker.nextNode()) {
    if (text === node) {
      return sourceOffsetForRenderedOffset(source, before + offsetInNode);
    }
    before += text.textContent?.length ?? 0;
  }
  return null;
}
