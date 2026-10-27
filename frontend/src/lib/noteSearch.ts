type HastNode = {
  type: string;
  tagName?: string;
  value?: string;
  properties?: Record<string, unknown>;
  children?: HastNode[];
};

export function normalizeSearchQuery(raw: string | null): string {
  if (!raw) {
    return "";
  }
  return raw.trim();
}

export function createSearchHighlightPlugin(query: string) {
  const trimmedQuery = normalizeSearchQuery(query);
  // Match on the original text: lowercasing a copy can change its length
  // (`İ` becomes two characters), so the copy's offsets would not line up.
  const pattern = trimmedQuery
    ? new RegExp(escapeRegExp(trimmedQuery), "giu")
    : null;

  return function searchHighlightPlugin() {
    return function transform(tree: HastNode): void {
      if (!pattern) {
        return;
      }

      let hitIndex = 0;
      visit(tree, false, (node, index, parent) => {
        if (
          !parent ||
          index === null ||
          node.type !== "text" ||
          typeof node.value !== "string" ||
          !node.value.trim()
        ) {
          return;
        }

        const highlighted = highlightTextNode(
          node.value,
          pattern,
          () => hitIndex++,
        );
        if (highlighted) {
          parent.children?.splice(index, 1, ...highlighted);
        }
      });
    };
  };
}

function visit(
  node: HastNode,
  skipped: boolean,
  visitor: (
    node: HastNode,
    index: number | null,
    parent: HastNode | null,
  ) => void,
  index: number | null = null,
  parent: HastNode | null = null,
): void {
  visitor(node, index, parent);

  const nextSkipped = skipped || shouldSkipHighlighting(node);
  if (nextSkipped || !node.children) {
    return;
  }

  for (let childIndex = 0; childIndex < node.children.length; childIndex += 1) {
    visit(node.children[childIndex], nextSkipped, visitor, childIndex, node);
  }
}

function shouldSkipHighlighting(node: HastNode): boolean {
  if (node.type !== "element") {
    return false;
  }

  if (node.tagName === "pre" || node.tagName === "code") {
    return true;
  }

  const className = node.properties?.className;
  const classes = Array.isArray(className)
    ? className
    : typeof className === "string"
      ? className.split(/\s+/)
      : [];

  return classes.some((item) =>
    [
      "search-hit",
      "katex",
      "katex-display",
      "math",
      "math-inline",
      "math-display",
    ].includes(String(item)),
  );
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function highlightTextNode(
  text: string,
  pattern: RegExp,
  nextHitIndex: () => number,
): HastNode[] | null {
  const matches = Array.from(text.matchAll(pattern));
  if (matches.length === 0) {
    return null;
  }

  const nodes: HastNode[] = [];
  let start = 0;

  for (const match of matches) {
    const matchIndex = match.index;
    if (matchIndex > start) {
      nodes.push({ type: "text", value: text.slice(start, matchIndex) });
    }

    nodes.push({
      type: "element",
      tagName: "mark",
      properties: {
        className: ["search-hit"],
        "data-hit-index": String(nextHitIndex()),
      },
      children: [
        {
          type: "text",
          value: match[0],
        },
      ],
    });

    start = matchIndex + match[0].length;
  }

  if (start < text.length) {
    nodes.push({ type: "text", value: text.slice(start) });
  }

  return nodes;
}

export function setActiveSearchHit(
  hits: HTMLSpanElement[],
  activeIndex: number,
): void {
  for (const [index, hit] of hits.entries()) {
    hit.classList.toggle("active-hit", index === activeIndex);
  }
}
