import { describe, expect, it } from "vitest";

import {
  applySearchHighlights,
  clearSearchHighlights,
  normalizeSearchQuery,
  setActiveSearchHit,
} from "./noteSearch";

describe("noteSearch", () => {
  it("normalizes empty and trimmed query values", () => {
    expect(normalizeSearchQuery(null)).toBe("");
    expect(normalizeSearchQuery("  atlas  ")).toBe("atlas");
  });

  it("highlights matches and skips code blocks", () => {
    const root = document.createElement("div");
    root.innerHTML = "<p>Atlas home atlas</p><pre><code>atlas</code></pre>";

    const hits = applySearchHighlights(root, "atlas");

    expect(hits).toHaveLength(2);
    expect(root.querySelectorAll("pre mark.search-hit")).toHaveLength(0);
  });

  it("clearSearchHighlights unwraps marks", () => {
    const root = document.createElement("div");
    root.innerHTML = '<p>A <mark class="search-hit">B</mark> C</p>';

    clearSearchHighlights(root);

    expect(root.querySelectorAll("mark.search-hit")).toHaveLength(0);
    expect(root.textContent).toContain("A B C");
  });

  it("setActiveSearchHit toggles the active marker", () => {
    const root = document.createElement("div");
    root.innerHTML =
      '<mark class="search-hit">A</mark><mark class="search-hit">B</mark>';
    const hits = Array.from(
      root.querySelectorAll("mark.search-hit"),
    ) as HTMLSpanElement[];

    setActiveSearchHit(hits, 1);

    expect(hits[0].classList.contains("active-hit")).toBe(false);
    expect(hits[1].classList.contains("active-hit")).toBe(true);
  });
});
