import { createContext, useContext } from "react";

import type { LineRange } from "../../lib/sourceMap";

export type InlineEditorValue = {
  writeEnabled: boolean;
  frontmatterOffset: number;
  /** The range currently being edited, or null when nothing is. */
  activeRange: LineRange | null;
  /** The source text of a range, sliced out of the whole document. */
  sourceOf: (range: LineRange) => string;
  enterBlock: (range: LineRange) => void;
  /** Writes `text` back over `range` and leaves the block. */
  commitBlock: (range: LineRange, text: string) => void;
  exitBlock: () => void;
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
