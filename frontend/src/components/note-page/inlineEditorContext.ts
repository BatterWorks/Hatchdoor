import { createContext, useContext } from "react";

import type { LineRange } from "../../lib/sourceMap";

export type InlineEditorValue = {
  writeEnabled: boolean;
  frontmatterOffset: number;
  /** The range currently being edited, or null when nothing is. */
  activeRange: LineRange | null;
  /** Caret offset within the active block's source, or null for end-of-block. */
  activeCaret: number | null;
  /** The source text of a range, sliced out of the whole document. */
  sourceOf: (range: LineRange) => string;
  enterBlock: (range: LineRange, caret: number | null) => void;
  /** Writes `text` back over `range` and leaves the block. */
  commitBlock: (
    range: LineRange,
    text: string,
    opts?: { keepActive?: boolean },
  ) => void;
  exitBlock: () => void;
  /** Registers a block so ops can find its neighbours in source order. */
  registerBlock: (range: LineRange) => () => void;
  /** Structural ops, each a no-op where the design disables it. */
  splitAt: (range: LineRange, caret: number) => void;
  mergeUp: (range: LineRange) => boolean;
  indent: (range: LineRange) => boolean;
  outdent: (range: LineRange) => boolean;
  /** Move to the unit before or after this one, keeping the column. */
  moveTo: (range: LineRange, direction: -1 | 1, column: number) => boolean;
};

export const InlineEditorContext = createContext<InlineEditorValue | null>(
  null,
);

/**
 * Null outside a provider, which is how every renderer degrades to read-only
 * without knowing anything about inline editing.
 */
export function useInlineEditor(): InlineEditorValue | null {
  return useContext(InlineEditorContext);
}
