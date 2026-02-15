const SEARCH_MARK_SELECTOR = "mark.search-hit";

export function normalizeSearchQuery(raw: string | null): string {
  if (!raw) {
    return "";
  }
  return raw.trim();
}

export function clearSearchHighlights(root: HTMLElement): void {
  const marks = root.querySelectorAll<HTMLSpanElement>(SEARCH_MARK_SELECTOR);
  for (const mark of marks) {
    const parent = mark.parentNode;
    if (!parent) {
      continue;
    }

    parent.replaceChild(document.createTextNode(mark.textContent ?? ""), mark);
    parent.normalize();
  }
}

export function applySearchHighlights(
  root: HTMLElement,
  query: string,
): HTMLSpanElement[] {
  clearSearchHighlights(root);

  const trimmedQuery = normalizeSearchQuery(query);
  if (!trimmedQuery) {
    return [];
  }

  const queryLower = trimmedQuery.toLowerCase();
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
    acceptNode(node) {
      if (!node.nodeValue || !node.nodeValue.trim()) {
        return NodeFilter.FILTER_REJECT;
      }

      const parent = node.parentElement;
      if (!parent) {
        return NodeFilter.FILTER_REJECT;
      }

      if (
        parent.closest(
          "pre, code, .code-block, .search-match, .note-links-panel, .note-toc, .tag-chip",
        )
      ) {
        return NodeFilter.FILTER_REJECT;
      }

      return NodeFilter.FILTER_ACCEPT;
    },
  });

  const nodes: Text[] = [];
  let current = walker.nextNode();
  while (current) {
    if (current.nodeType === Node.TEXT_NODE) {
      nodes.push(current as Text);
    }
    current = walker.nextNode();
  }

  const marks: HTMLSpanElement[] = [];

  for (const node of nodes) {
    const text = node.nodeValue ?? "";
    const lower = text.toLowerCase();
    if (!lower.includes(queryLower)) {
      continue;
    }

    const fragment = document.createDocumentFragment();
    let start = 0;

    while (start < text.length) {
      const matchIndex = lower.indexOf(queryLower, start);
      if (matchIndex === -1) {
        fragment.appendChild(document.createTextNode(text.slice(start)));
        break;
      }

      if (matchIndex > start) {
        fragment.appendChild(
          document.createTextNode(text.slice(start, matchIndex)),
        );
      }

      const mark = document.createElement("mark");
      mark.className = "search-hit";
      mark.textContent = text.slice(
        matchIndex,
        matchIndex + trimmedQuery.length,
      );
      marks.push(mark);
      fragment.appendChild(mark);

      start = matchIndex + trimmedQuery.length;
    }

    node.parentNode?.replaceChild(fragment, node);
  }

  for (const [index, mark] of marks.entries()) {
    mark.setAttribute("data-hit-index", String(index));
  }

  return marks;
}

export function setActiveSearchHit(
  hits: HTMLSpanElement[],
  activeIndex: number,
): void {
  for (const [index, hit] of hits.entries()) {
    hit.classList.toggle("active-hit", index === activeIndex);
  }
}
