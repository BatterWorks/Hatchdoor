export function scrollElementIntoView(
  element: Element | null,
  options: ScrollIntoViewOptions,
): void {
  if (!element) {
    return;
  }
  const maybeScrollable = element as Element & {
    scrollIntoView?: (opts?: ScrollIntoViewOptions) => void;
  };
  if (typeof maybeScrollable.scrollIntoView === "function") {
    maybeScrollable.scrollIntoView(options);
  }
}

export function jumpToHeading(id: string): void {
  const heading = document.getElementById(id);
  scrollElementIntoView(heading, {
    behavior: "smooth",
    block: "start",
    inline: "nearest",
  });
}
