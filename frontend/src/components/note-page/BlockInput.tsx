import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { KeyboardEvent } from "react";

export type UnitType =
  | "paragraph"
  | "heading"
  | "list item"
  | "quote"
  | "callout"
  | "code block"
  | "table row";

/**
 * The textarea that replaces one rendered block while it is being edited.
 *
 * Styling matches the metrics of the block it replaces but never its
 * transforms: `.block-input` resets text-transform, because three rendered
 * styles in this app uppercase their text and would otherwise show the typist a
 * different string than the one being saved.
 */
export function BlockInput({
  unitType,
  initialValue,
  onCommit,
  onCancel,
}: {
  unitType: UnitType;
  initialValue: string;
  onCommit: (text: string) => void;
  onCancel: () => void;
}) {
  const [value, setValue] = useState(initialValue);
  const ref = useRef<HTMLTextAreaElement | null>(null);
  const committedRef = useRef(false);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) {
      return;
    }
    el.focus();
    el.setSelectionRange(el.value.length, el.value.length);
  }, []);

  useEffect(() => {
    const el = ref.current;
    if (!el) {
      return;
    }
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight}px`;
  }, [value]);

  const commit = () => {
    if (committedRef.current) {
      return;
    }
    committedRef.current = true;
    onCommit(value);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    // Every intercepted key is gated on composition: with an IME active, Enter
    // and Escape belong to the candidate window, not to us.
    if (event.nativeEvent.isComposing) {
      return;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      committedRef.current = true;
      onCancel();
    }
  };

  return (
    <textarea
      ref={ref}
      className={`block-input block-input-${unitType.replace(/\s+/g, "-")}`}
      aria-label={`Editing ${unitType}`}
      value={value}
      rows={1}
      spellCheck
      onChange={(event) => setValue(event.target.value)}
      onBlur={commit}
      onKeyDown={onKeyDown}
    />
  );
}
