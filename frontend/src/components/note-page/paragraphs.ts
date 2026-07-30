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

/**
 * Split a paragraph's rendered children back into one array per source line.
 *
 * Consecutive `> ` lines parse as a single paragraph joined by soft line
 * breaks, which arrive here as newlines inside string children. Callouts are
 * addressed per line, so the run has to be taken apart again.
 *
 * The index of a returned line is what maps it back to a source line, so an
 * interior line is never dropped however empty it looks: dropping one would
 * shift every line below it and point its block at the wrong line of the file.
 * Only a trailing empty line is discarded, which shifts nothing.
 */
export function splitAtSoftBreaks(children: ReactNode[]): ReactNode[][] {
  if (children.length === 0) {
    return [];
  }

  const lines: ReactNode[][] = [];
  let current: ReactNode[] = [];

  for (const child of children) {
    if (typeof child !== "string" || !child.includes("\n")) {
      current.push(child);
      continue;
    }

    const parts = child.split("\n");
    parts.forEach((part, index) => {
      if (index > 0) {
        lines.push(current);
        current = [];
      }
      if (part !== "") {
        current.push(part);
      }
    });
  }

  lines.push(current);

  while (lines.length > 0 && lines[lines.length - 1].length === 0) {
    lines.pop();
  }

  return lines;
}
