import { useCallback, useMemo, useRef, useState, type ReactNode } from "react";

import { replaceLines, sliceLines, type LineRange } from "../../lib/sourceMap";
import {
  indentListItem,
  mergeBlockUp,
  outdentListItem,
  splitBlock,
} from "../../lib/blockOps";
import { InlineEditorContext } from "./inlineEditorContext";

export function InlineEditorProvider({
  content,
  frontmatterOffset,
  writeEnabled,
  settling = false,
  onChange,
  children,
}: {
  content: string;
  frontmatterOffset: number;
  writeEnabled: boolean;
  settling?: boolean;
  onChange: (next: string) => void;
  children: ReactNode;
}) {
  const [activeRange, setActiveRange] = useState<LineRange | null>(null);
  const [activeCaret, setActiveCaret] = useState<number | null>(null);
  // Ops chain off a commit in the same tick, so they read the text just
  // written rather than the content prop from the previous render.
  const latestRef = useRef(content);
  latestRef.current = content;
  const settlingRef = useRef(settling);
  settlingRef.current = settling;

  const sourceOf = useCallback(
    (range: LineRange) => sliceLines(content, range.startLine, range.endLine),
    [content],
  );

  const enterBlock = useCallback(
    (range: LineRange, caret: number | null) => {
      // Refusing beats guessing: entering a block while the rendered tree is
      // stale would address the wrong lines and silently edit the wrong text.
      if (!writeEnabled || settlingRef.current) {
        return;
      }
      setActiveRange(range);
      setActiveCaret(caret);
    },
    [writeEnabled],
  );

  // Navigation and merging follow source order, not DOM order. With remark-gfm
  // a footnote definition renders inside a generated section at the end of the
  // document while carrying the position of wherever it was written, so the two
  // orders genuinely diverge.
  const blocksRef = useRef<LineRange[]>([]);
  const registerBlock = useCallback((range: LineRange) => {
    blocksRef.current = [...blocksRef.current, range].sort(
      (a, b) => a.startLine - b.startLine,
    );
    return () => {
      const at = blocksRef.current.findIndex(
        (r) => r.startLine === range.startLine && r.endLine === range.endLine,
      );
      if (at >= 0) {
        blocksRef.current = [
          ...blocksRef.current.slice(0, at),
          ...blocksRef.current.slice(at + 1),
        ];
      }
    };
  }, []);

  const previousOf = useCallback(
    (range: LineRange) =>
      [...blocksRef.current].filter((r) => r.endLine < range.startLine).pop() ??
      null,
    [],
  );

  const splitAt = useCallback(
    (range: LineRange, caret: number) => {
      const result = splitBlock(latestRef.current, range, caret);
      setActiveRange({
        startLine: result.caretLine,
        endLine: result.caretLine,
      });
      setActiveCaret(result.caretOffset);
      onChange(result.content);
    },
    [onChange],
  );

  const mergeUp = useCallback(
    (range: LineRange) => {
      const result = mergeBlockUp(latestRef.current, range, previousOf(range));
      if (!result) {
        return false;
      }
      setActiveRange({
        startLine: result.caretLine,
        endLine: result.caretLine,
      });
      setActiveCaret(result.caretOffset);
      onChange(result.content);
      return true;
    },
    [onChange, previousOf],
  );

  const indent = useCallback(
    (range: LineRange) => {
      const result = indentListItem(latestRef.current, range);
      if (!result) {
        return false;
      }
      setActiveRange(null);
      setActiveCaret(null);
      onChange(result.content);
      return true;
    },
    [onChange],
  );

  const outdent = useCallback(
    (range: LineRange) => {
      const result = outdentListItem(latestRef.current, range);
      if (!result) {
        return false;
      }
      setActiveRange(null);
      setActiveCaret(null);
      onChange(result.content);
      return true;
    },
    [onChange],
  );

  const exitBlock = useCallback(() => {
    setActiveRange(null);
    setActiveCaret(null);
  }, []);

  const commitBlock = useCallback(
    (range: LineRange, text: string, opts?: { keepActive?: boolean }) => {
      if (!opts?.keepActive) {
        setActiveRange(null);
        setActiveCaret(null);
      }
      if (text === sliceLines(content, range.startLine, range.endLine)) {
        return;
      }
      const next = replaceLines(content, range.startLine, range.endLine, text);
      latestRef.current = next;
      onChange(next);
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
      registerBlock,
      splitAt,
      mergeUp,
      indent,
      outdent,
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
      registerBlock,
      splitAt,
      mergeUp,
      indent,
      outdent,
    ],
  );

  return (
    <InlineEditorContext.Provider value={value}>
      {children}
    </InlineEditorContext.Provider>
  );
}
