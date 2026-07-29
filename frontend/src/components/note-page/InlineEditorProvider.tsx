import { useCallback, useMemo, useState, type ReactNode } from "react";

import { replaceLines, sliceLines, type LineRange } from "../../lib/sourceMap";
import { InlineEditorContext } from "./inlineEditorContext";

export function InlineEditorProvider({
  content,
  frontmatterOffset,
  writeEnabled,
  onChange,
  children,
}: {
  content: string;
  frontmatterOffset: number;
  writeEnabled: boolean;
  onChange: (next: string) => void;
  children: ReactNode;
}) {
  const [activeRange, setActiveRange] = useState<LineRange | null>(null);
  const [activeCaret, setActiveCaret] = useState<number | null>(null);

  const sourceOf = useCallback(
    (range: LineRange) => sliceLines(content, range.startLine, range.endLine),
    [content],
  );

  const enterBlock = useCallback(
    (range: LineRange, caret: number | null) => {
      if (!writeEnabled) {
        return;
      }
      setActiveRange(range);
      setActiveCaret(caret);
    },
    [writeEnabled],
  );

  const exitBlock = useCallback(() => {
    setActiveRange(null);
    setActiveCaret(null);
  }, []);

  const commitBlock = useCallback(
    (range: LineRange, text: string) => {
      setActiveRange(null);
      setActiveCaret(null);
      if (text === sliceLines(content, range.startLine, range.endLine)) {
        return;
      }
      onChange(replaceLines(content, range.startLine, range.endLine, text));
    },
    [content, onChange],
  );

  const value = useMemo(
    () => ({
      writeEnabled,
      frontmatterOffset,
      activeRange,
      activeCaret,
      sourceOf,
      enterBlock,
      commitBlock,
      exitBlock,
    }),
    [
      writeEnabled,
      frontmatterOffset,
      activeRange,
      activeCaret,
      sourceOf,
      enterBlock,
      commitBlock,
      exitBlock,
    ],
  );

  return (
    <InlineEditorContext.Provider value={value}>
      {children}
    </InlineEditorContext.Provider>
  );
}
