import { describe, expect, it } from "vitest";

import {
  createSearchHighlightPlugin,
  normalizeSearchQuery,
  setActiveSearchHit,
} from "./noteSearch";

describe("noteSearch", () => {
  it("normalizes empty and trimmed query values", () => {
    expect(normalizeSearchQuery(null)).toBe("");
    expect(normalizeSearchQuery("  atlas  ")).toBe("atlas");
  });

  it("creates render-owned highlights and skips code blocks", () => {
    const tree: {
      type: string;
      children: Array<{
        type: string;
        tagName: string;
        properties: Record<string, unknown>;
        children: Array<{
          type: string;
          tagName?: string;
          properties?: Record<string, unknown>;
          value?: string;
          children?: Array<{ type: string; value: string }>;
        }>;
      }>;
    } = {
      type: "root",
      children: [
        {
          type: "element",
          tagName: "p",
          properties: {},
          children: [{ type: "text", value: "Atlas home atlas" }],
        },
        {
          type: "element",
          tagName: "pre",
          properties: {},
          children: [
            {
              type: "element",
              tagName: "code",
              properties: {},
              children: [{ type: "text", value: "atlas" }],
            },
          ],
        },
      ],
    };

    createSearchHighlightPlugin("atlas")()(tree);

    const paragraph = tree.children[0];
    expect(paragraph.children).toHaveLength(3);
    expect(paragraph.children[0]).toMatchObject({
      type: "element",
      tagName: "mark",
      properties: {
        className: ["search-hit"],
        "data-hit-index": "0",
      },
    });
    expect(paragraph.children[2]).toMatchObject({
      type: "element",
      tagName: "mark",
      properties: {
        className: ["search-hit"],
        "data-hit-index": "1",
      },
    });
    expect(tree.children[1].children[0].children).toEqual([
      { type: "text", value: "atlas" },
    ]);
  });

  it("assigns hit indexes in reading order across sibling text nodes", () => {
    const tree: {
      type: string;
      children: Array<{
        type: string;
        tagName: string;
        properties: Record<string, unknown>;
        children: Array<{
          type: string;
          tagName?: string;
          properties?: Record<string, unknown>;
          value?: string;
          children?: Array<{ type: string; value: string }>;
        }>;
      }>;
    } = {
      type: "root",
      children: [
        {
          type: "element",
          tagName: "p",
          properties: {},
          children: [
            { type: "text", value: "first token " },
            {
              type: "element",
              tagName: "strong",
              properties: {},
              children: [{ type: "text", value: "second token" }],
            },
          ],
        },
      ],
    };

    createSearchHighlightPlugin("token")()(tree);

    const paragraph = tree.children[0];
    expect(paragraph.children[1]).toMatchObject({
      type: "element",
      tagName: "mark",
      properties: {
        "data-hit-index": "0",
      },
    });
    expect(paragraph.children[3].children?.[1]).toMatchObject({
      type: "element",
      tagName: "mark",
      properties: {
        "data-hit-index": "1",
      },
    });
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
