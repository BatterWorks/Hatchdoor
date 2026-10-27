import { describe, expect, it } from "vitest";

import {
  createSearchHighlightPlugin,
  normalizeSearchQuery,
  setActiveSearchHit,
} from "./noteSearch";

type TextTree = {
  type: string;
  tagName?: string;
  value?: string;
  children?: TextTree[];
};

// Highlights `text` as one paragraph and writes each mark as `[...]`.
function markedText(text: string, query: string): string {
  const tree: TextTree = {
    type: "root",
    children: [
      {
        type: "element",
        tagName: "p",
        children: [{ type: "text", value: text }],
      },
    ],
  };

  createSearchHighlightPlugin(query)()(tree);

  return (tree.children?.[0].children ?? [])
    .map((node) =>
      node.tagName === "mark"
        ? `[${node.children?.[0].value ?? ""}]`
        : (node.value ?? ""),
    )
    .join("");
}

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

  it("marks a match that fills its whole text node and numbers hits without gaps", () => {
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
          children?: Array<{
            type: string;
            tagName?: string;
            properties?: Record<string, unknown>;
            value?: string;
            children?: Array<{ type: string; value: string }>;
          }>;
        }>;
      }>;
    } = {
      type: "root",
      children: [
        {
          type: "element",
          tagName: "h1",
          properties: {},
          children: [{ type: "text", value: "Homelab" }],
        },
        {
          type: "element",
          tagName: "p",
          properties: {},
          children: [
            {
              type: "element",
              tagName: "strong",
              properties: {},
              children: [{ type: "text", value: "homelab" }],
            },
          ],
        },
        {
          type: "element",
          tagName: "p",
          properties: {},
          children: [
            { type: "text", value: "my homelab rack" },
            {
              type: "element",
              tagName: "em",
              properties: {},
              children: [{ type: "text", value: "no match here" }],
            },
          ],
        },
      ],
    };

    createSearchHighlightPlugin("homelab")()(tree);

    const [heading, boldParagraph, plainParagraph] = tree.children;
    expect(heading.children).toEqual([
      {
        type: "element",
        tagName: "mark",
        properties: { className: ["search-hit"], "data-hit-index": "0" },
        children: [{ type: "text", value: "Homelab" }],
      },
    ]);
    expect(boldParagraph.children[0].children).toEqual([
      {
        type: "element",
        tagName: "mark",
        properties: { className: ["search-hit"], "data-hit-index": "1" },
        children: [{ type: "text", value: "homelab" }],
      },
    ]);
    expect(plainParagraph.children[1]).toMatchObject({
      tagName: "mark",
      properties: { "data-hit-index": "2" },
    });
    expect(plainParagraph.children[3].children).toEqual([
      { type: "text", value: "no match here" },
    ]);
  });

  it("marks exactly the match after a character that grows when lowercased", () => {
    // `İ` lowercases to `i` plus a combining dot, two characters for one.
    expect(markedText("İhome", "home")).toBe("İ[home]");
    expect(markedText("İstanbul home", "home")).toBe("İstanbul [home]");
    expect(markedText("İİ home", "home")).toBe("İİ [home]");
    expect(markedText("İ İstanbul", "İstanbul")).toBe("İ [İstanbul]");
  });

  it("matches without case and reads the query literally", () => {
    expect(markedText("Atlas, atlas, ATLAS", "atlas")).toBe(
      "[Atlas], [atlas], [ATLAS]",
    );
    expect(markedText("a.b axb", "a.b")).toBe("[a.b] axb");
    expect(markedText("(x) x", "(x)")).toBe("[(x)] x");
    expect(markedText("[x]* x", "[x]*")).toBe("[[x]*] x");
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
