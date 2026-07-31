import {
  Children,
  cloneElement,
  isValidElement,
  useEffect,
  useRef,
  type KeyboardEvent,
  type MouseEvent,
  type PointerEvent,
  type ReactNode,
} from "react";

import { sourceOffsetForRenderedOffset } from "../../lib/caretMap";
import { blockRange, type LineRange } from "../../lib/sourceMap";
import { BlockInput, type UnitType } from "./BlockInput";
import { useInlineEditor } from "./inlineEditorContext";

function sameRange(a: LineRange | null, b: LineRange): boolean {
  return a !== null && a.startLine === b.startLine && a.endLine === b.endLine;
}

/**
 * The source offset under a pointer, or null when the browser cannot say.
 *
 * Approximate by design (D9): markdown syntax has no rendered counterpart, so
 * landing a few characters out is fine. Landing always at 0, or always at the
 * end, is not.
 */
function caretOffsetAtPoint(
  clientX: number,
  clientY: number,
  source: string,
): number | null {
  const doc = document as Document & {
    caretPositionFromPoint?: (
      x: number,
      y: number,
    ) => { offset: number } | null;
    caretRangeFromPoint?: (
      x: number,
      y: number,
    ) => { startOffset: number } | null;
  };

  const position = doc.caretPositionFromPoint?.(clientX, clientY);
  if (position) {
    return sourceOffsetForRenderedOffset(source, position.offset);
  }
  // WebKit spells it differently.
  const range = doc.caretRangeFromPoint?.(clientX, clientY);
  if (range) {
    return sourceOffsetForRenderedOffset(source, range.startOffset);
  }
  return null;
}

/**
 * How long after a tap a second one still counts as "edit this", not "read".
 *
 * Generous on purpose. A double-tap handler normally has to hold a single tap's
 * action hostage while it waits, so the window is a latency budget. Here a
 * single tap on prose does nothing, so there is nothing to delay and nothing to
 * disambiguate: the window costs only forgiveness.
 */
const DOUBLE_TAP_MS = 400;
/** Two taps further apart than this are aimed at different lines. */
const DOUBLE_TAP_SLOP_PX = 30;

type TapMark = { x: number; y: number; at: number };

/**
 * Whether this tap closes a double tap, recording it as the opening one if not.
 *
 * Module scope rather than a closure so the clock is read outside a component
 * body, where reading it is a rule violation rather than merely inelegant.
 */
function isSecondTap(
  lastTap: { current: TapMark | null },
  clientX: number,
  clientY: number,
): boolean {
  const previous = lastTap.current;
  const now = Date.now();
  const closes =
    previous !== null &&
    now - previous.at <= DOUBLE_TAP_MS &&
    Math.abs(clientX - previous.x) <= DOUBLE_TAP_SLOP_PX &&
    Math.abs(clientY - previous.y) <= DOUBLE_TAP_SLOP_PX;

  // A tap that closes a pair is consumed, so a third tap opens a new pair
  // rather than re-entering the block.
  lastTap.current = closes ? null : { x: clientX, y: clientY, at: now };
  return closes;
}

/**
 * Makes one rendered block editable in place.
 *
 * The rendered element is cloned rather than wrapped. A wrapper would sit
 * between a list and its items, breaking every `ul > li` rule (the bullet
 * markers) and putting a div directly inside a ul, and it would do the same to
 * table rows. Cloning keeps the tree exactly as the renderer built it and only
 * adds a class and a handler.
 *
 * Degrades to the untouched child whenever the block owns no lines: outside a
 * provider, in a read-only vault, or when the node carries no position at all
 * (display math, raw HTML, link reference definitions, generated footnotes).
 */
export function EditableBlock({
  node,
  range: explicitRange,
  unitType,
  children,
}: {
  node?: unknown;
  /**
   * For content the renderer rebuilds rather than passes through, where the
   * original node's position is no longer attached to what is on screen.
   * Given in rendered coordinates, exactly like a node position, so the
   * frontmatter offset still applies.
   */
  range?: LineRange;
  unitType: UnitType;
  children: ReactNode;
}) {
  const editor = useInlineEditor();
  const offset = editor?.frontmatterOffset ?? 0;
  const range = explicitRange
    ? {
        startLine: explicitRange.startLine + offset,
        endLine: explicitRange.endLine + offset,
      }
    : editor
      ? blockRange(node, offset)
      : null;
  const elementRef = useRef<HTMLElement | null>(null);
  const lastTapRef = useRef<{ x: number; y: number; at: number } | null>(null);
  const touchRef = useRef(false);

  const enterAt = (clientX: number, clientY: number) => {
    if (!editor || !range) {
      return;
    }
    editor.enterBlock(
      range,
      caretOffsetAtPoint(clientX, clientY, editor.sourceOf(range)),
    );
  };

  const registerBlock = editor?.registerBlock;
  const rangeKey = range ? `${range.startLine}:${range.endLine}` : null;
  useEffect(() => {
    if (!registerBlock || !rangeKey) {
      return;
    }
    const [startLine, endLine] = rangeKey.split(":").map(Number);
    return registerBlock({ startLine, endLine }, unitType);
  }, [registerBlock, rangeKey, unitType]);

  const isActive = sameRange(
    editor?.activeRange ?? null,
    range ?? { startLine: -1, endLine: -1 },
  );
  const wasActiveRef = useRef(false);
  useEffect(() => {
    // Leaving a block must hand focus back to it, or Escape strands a keyboard
    // user with nothing focused at all.
    // Only when nothing else took over: clicking straight into another block
    // must not have this one steal focus back and blur the new input.
    if (wasActiveRef.current && !isActive && editor?.activeRange == null) {
      elementRef.current?.focus();
    }
    wasActiveRef.current = isActive;
  }, [isActive, editor?.activeRange]);

  if (!editor || !editor.writeEnabled || !range || !isValidElement(children)) {
    return <>{children}</>;
  }

  const child = children as React.ReactElement<{
    className?: string;
    onClick?: (event: MouseEvent) => void;
    onPointerDown?: (event: PointerEvent) => void;
    onKeyDown?: (event: KeyboardEvent) => void;
    tabIndex?: number;
    "data-start-line"?: number;
    "data-end-line"?: number;
    ref?: unknown;
    children?: ReactNode;
  }>;

  if (isActive) {
    const input = (
      <BlockInput
        unitType={unitType}
        initialValue={editor.sourceOf(range)}
        initialCaret={editor.activeCaret}
        onCommit={(text) => editor.commitBlock(range, text)}
        onEdit={(text) => editor.previewBlock(range, text)}
        onSplit={(text, caret) => {
          // The op reads the document, so the in-progress text has to be part
          // of it first, or the split would run against the stale line.
          editor.commitBlock(range, text, { keepActive: true });
          editor.splitAt(range, caret);
        }}
        onMergeUp={(text) => {
          editor.commitBlock(range, text, { keepActive: true });
          return editor.mergeUp(range);
        }}
        onIndent={(text) => {
          editor.commitBlock(range, text, { keepActive: true });
          return editor.indent(range);
        }}
        onOutdent={(text) => {
          editor.commitBlock(range, text, { keepActive: true });
          return editor.outdent(range);
        }}
        onMove={(text, direction, column) => {
          editor.commitBlock(range, text, { keepActive: true });
          return editor.moveTo(range, direction, column);
        }}
      />
    );
    const activeClass = joinClass(
      child.props.className,
      "editable-block is-active",
    );
    // A tr may not contain a textarea, and swapping its cells for one would
    // let every column resize on entry and again on exit, because widths are
    // derived from content. So the cells stay exactly as they are, holding the
    // grid still, and the input is overlaid on the row from inside its first
    // cell, which is somewhere a textarea is allowed to live.
    if (child.type === "tr") {
      const cells = Children.toArray(child.props.children);
      const overlaid = cells.map((cell, index) =>
        index === 0 && isValidElement<{ children?: ReactNode }>(cell)
          ? cloneElement(
              cell,
              {},
              <>
                {cell.props.children}
                <span className="block-input-overlay">{input}</span>
              </>,
            )
          : cell,
      );
      return cloneElement(child, { className: activeClass }, overlaid);
    }

    // Same reason as below: a component child cannot receive replacement
    // children, so it gets a wrapper rather than being cloned into.
    return typeof child.type === "string" ? (
      cloneElement(child, { className: activeClass }, input)
    ) : (
      <div className={activeClass}>{input}</div>
    );
  }

  const claimsGesture = (target: EventTarget | null) => {
    // Links, checkboxes, and callout summaries keep their own behaviour.
    const el = target as HTMLElement | null;
    return !!el?.closest?.("a, input, summary, button");
  };

  // Only a real touch sequence arms the two-tap requirement. A screen reader
  // activating the focused block synthesizes a bare click with no pointer
  // sequence in front of it, so it takes the mouse path and enters on one
  // activation rather than needing a literal double-tap.
  const onPointerDown = (event: PointerEvent) => {
    touchRef.current =
      event.pointerType === "touch" || event.pointerType === "pen";
  };

  const onClick = (event: MouseEvent) => {
    if (event.defaultPrevented) {
      return;
    }
    // mdast-util-to-hast emits task checkboxes disabled, and disabled inputs
    // fire no click events, so the handler has to live on the li. The input
    // node carries no position either, so the line comes from this block.
    const clicked = event.target as HTMLElement;
    if (
      unitType === "list item" &&
      clicked instanceof HTMLInputElement &&
      clicked.type === "checkbox"
    ) {
      event.preventDefault();
      editor.toggleTask(range.startLine);
      return;
    }
    // The innermost block wins, so an ancestor never claims a click a
    // descendant already handled.
    if (claimsGesture(event.target)) {
      return;
    }
    // On a phone, reading is the dominant mode: entering on a single tap would
    // raise the keyboard on every stray touch. Entry is a deliberate double
    // tap, which unlike a hold does not race the OS text-selection gesture.
    if (
      touchRef.current &&
      !isSecondTap(lastTapRef, event.clientX, event.clientY)
    ) {
      return;
    }
    event.preventDefault();
    enterAt(event.clientX, event.clientY);
  };

  const onKeyDown = (event: KeyboardEvent) => {
    // Composition owns Enter while an IME candidate window is open.
    if (event.nativeEvent.isComposing || event.key !== "Enter") {
      return;
    }
    if (event.target !== event.currentTarget) {
      return;
    }
    event.preventDefault();
    editor.enterBlock(range, null);
  };

  const handlers = {
    className: joinClass(child.props.className, "editable-block"),
    "data-start-line": range.startLine,
    "data-end-line": range.endLine,
    onClick,
    onPointerDown,
    onKeyDown,
    ref: elementRef,
    // Every editable unit is reachable by Tab, so there is a keyboard-only
    // path to editing rather than a mouse-only one.
    tabIndex: 0,
  };

  // Props only reach the DOM when the child is an intrinsic element. A
  // component child (the fenced code block renders its own chrome) would
  // silently swallow them, so it gets a wrapper instead. That is safe here
  // precisely because a code block is not a list item or a table row, where a
  // wrapper would sit between a parent and children it must be adjacent to.
  if (typeof child.type !== "string") {
    const { ref, ...rest } = handlers;
    return (
      <div {...rest} ref={ref as React.RefObject<HTMLDivElement | null>}>
        {child}
      </div>
    );
  }

  return cloneElement(child, handlers);
}

function joinClass(existing: string | undefined, added: string): string {
  return existing ? `${existing} ${added}` : added;
}
