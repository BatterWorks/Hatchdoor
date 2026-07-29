import { isValidElement, type ReactNode } from "react";

// Callout detection needs to know whether the first child of a blockquote is a
// paragraph. That is not just `type === "p"`: the components map supplies its
// own paragraph component, in which case the element's type is that function.
// Marking it keeps the check exact rather than loosening it to "any element".
const PARAGRAPH_MARKER = "isNoteParagraph";

export function markAsParagraph<T extends object>(component: T): T {
  (component as Record<string, unknown>)[PARAGRAPH_MARKER] = true;
  return component;
}

export function isParagraphElement(node: ReactNode): boolean {
  if (!isValidElement(node)) {
    return false;
  }
  const { type } = node;
  if (type === "p") {
    return true;
  }
  return (
    typeof type === "function" &&
    (type as unknown as Record<string, unknown>)[PARAGRAPH_MARKER] === true
  );
}
