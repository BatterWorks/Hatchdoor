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

  return function searchHighlightPlugin() {
    return function transform(tree: HastNode): void {
      if (!trimmedQuery) {
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
          trimmedQuery,
          () => hitIndex++,
        );
        if (highlighted.length > 1) {
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

function highlightTextNode(
  text: string,
  query: string,
  nextHitIndex: () => number,
): HastNode[] {
  const lower = text.toLowerCase();
  const queryLower = query.toLowerCase();
  if (!lower.includes(queryLower)) {
    return [{ type: "text", value: text }];
  }

  const nodes: HastNode[] = [];
  let start = 0;

  while (start < text.length) {
    const matchIndex = lower.indexOf(queryLower, start);
    if (matchIndex === -1) {
      nodes.push({ type: "text", value: text.slice(start) });
      break;
    }

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
          value: text.slice(matchIndex, matchIndex + query.length),
        },
      ],
    });

    start = matchIndex + query.length;
  }

  if (nodes.length === 0) {
    return [{ type: "text", value: text }];
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
