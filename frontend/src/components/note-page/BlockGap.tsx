import type { MouseEvent, ReactNode } from "react";

import { insertionLineForDrop } from "./attachmentDrop";
import { useInlineEditor } from "./inlineEditorContext";

/**
 * The space between blocks, and below the last one, as a place to start
 * writing.
 *
 * Blocks only cover the lines that have content, so the gaps between them
 * belong to nothing and swallow a click. That makes the end of a note the one
 * place a writer cannot simply click and type, which is the first thing anyone
 * tries.
 *
 * The click is only ours when it landed on this element itself. A click that
 * reached a block is that block's, and comparing target to currentTarget is
 * what tells the two apart without having to know anything about the tree in
 * between.
 */
export function BlockGap({ children }: { children: ReactNode }) {
  const editor = useInlineEditor();

  const onClick = (event: MouseEvent<HTMLDivElement>) => {
    if (!editor?.writeEnabled || event.defaultPrevented) {
      return;
    }
    if (event.target !== event.currentTarget) {
      return;
    }

    // Line ranges come off the rendered blocks rather than out of the
    // document, because the question being asked is a geometric one: which
    // block did this click land past. The dataset carries file coordinates
    // already, which is what insertParagraph addresses.
    const blocks = Array.from(
      event.currentTarget.querySelectorAll<HTMLElement>(".editable-block"),
    ).flatMap((el) => {
      const startLine = Number(el.dataset.startLine);
      const endLine = Number(el.dataset.endLine);
      if (!Number.isFinite(startLine) || !Number.isFinite(endLine)) {
        return [];
      }
      const rect = el.getBoundingClientRect();
      return [{ startLine, endLine, top: rect.top, bottom: rect.bottom }];
    });

    editor.insertParagraph(insertionLineForDrop(blocks, event.clientY));
  };

  return (
    <div className="block-gap" onClick={onClick}>
      {children}
    </div>
  );
}
