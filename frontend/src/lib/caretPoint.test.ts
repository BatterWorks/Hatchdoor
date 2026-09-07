import { describe, expect, it } from "vitest";

import { sourceOffsetForCaretPoint } from "./caretPoint";

/**
 * Builds a block whose rendered text is `See alias TAILWORD here.`, with
 * `alias` inside `wrapper`, and returns the block plus its text nodes.
 *
 * Every inline construct the map knows about renders to this same shape: three
 * child nodes where a plain paragraph has one. That is the whole defect, so one
 * builder covers every row of the table.
 */
function splitBlock(wrapper: HTMLElement): {
  root: HTMLElement;
  inner: Text;
  tail: Text;
} {
  const root = document.createElement("p");
  const lead = document.createTextNode("See ");
  const inner = document.createTextNode("alias");
  const tail = document.createTextNode(" TAILWORD here.");
  wrapper.append(inner);
  root.append(lead, wrapper, tail);
  return { root, inner, tail };
}

function anchor(): HTMLElement {
  const element = document.createElement("a");
  element.setAttribute("href", "/notes/some-note");
  return element;
}

describe("sourceOffsetForCaretPoint", () => {
  // Clicking between TAIL and WORD is offset 5 in the trailing text node and
  // offset 14 across the block. Feeding the map 5 lands inside the link's
  // target; 14 lands on the W that was clicked.
  const CLICK_IN_TAIL = 5;

  it("counts the text before a wikilink's own node", () => {
    const { root, tail } = splitBlock(anchor());
    expect(
      sourceOffsetForCaretPoint(
        root,
        tail,
        CLICK_IN_TAIL,
        "See [[Some Note|alias]] TAILWORD here.",
      ),
    ).toBe(28);
  });

  it("counts the text before an escaped wikilink's own node", () => {
    const { root, tail } = splitBlock(anchor());
    expect(
      sourceOffsetForCaretPoint(
        root,
        tail,
        CLICK_IN_TAIL,
        "See [[Some Note\\|alias]] TAILWORD here.",
      ),
    ).toBe(29);
  });

  it("counts the text before a markdown link's own node", () => {
    const { root, tail } = splitBlock(anchor());
    expect(
      sourceOffsetForCaretPoint(
        root,
        tail,
        CLICK_IN_TAIL,
        "See [alias](/target) TAILWORD here.",
      ),
    ).toBe(25);
  });

  it("counts the text before a bold run", () => {
    const { root, tail } = splitBlock(document.createElement("strong"));
    expect(
      sourceOffsetForCaretPoint(
        root,
        tail,
        CLICK_IN_TAIL,
        "See **alias** TAILWORD here.",
      ),
    ).toBe(18);
  });

  it("counts the text before an italic run", () => {
    const { root, tail } = splitBlock(document.createElement("em"));
    expect(
      sourceOffsetForCaretPoint(
        root,
        tail,
        CLICK_IN_TAIL,
        "See *alias* TAILWORD here.",
      ),
    ).toBe(16);
  });

  it("counts the text before an inline code span", () => {
    const { root, tail } = splitBlock(document.createElement("code"));
    expect(
      sourceOffsetForCaretPoint(
        root,
        tail,
        CLICK_IN_TAIL,
        "See `alias` TAILWORD here.",
      ),
    ).toBe(16);
  });

  it("descends through nested inline elements", () => {
    const strong = document.createElement("strong");
    const { root, tail } = splitBlock(strong);
    const em = document.createElement("em");
    em.append(document.createTextNode("alias"));
    strong.replaceChildren(em);
    expect(
      sourceOffsetForCaretPoint(
        root,
        tail,
        CLICK_IN_TAIL,
        "See ***alias*** TAILWORD here.",
      ),
    ).toBe(20);
  });

  it("maps a click inside a link's own text onto the alias, not the target", () => {
    const { root, inner } = splitBlock(anchor());
    // Rendered offset 2 within "alias" is the i.
    expect(
      sourceOffsetForCaretPoint(
        root,
        inner,
        2,
        "See [[Some Note|alias]] TAILWORD here.",
      ),
    ).toBe(18);
  });

  it("leaves a single-text-node block exactly as it was", () => {
    const root = document.createElement("p");
    const only = document.createTextNode("See alias TAILWORD here.");
    root.append(only);
    expect(
      sourceOffsetForCaretPoint(root, only, 14, "See alias TAILWORD here."),
    ).toBe(14);
  });

  it("has no preference when the node belongs to another block", () => {
    const { tail } = splitBlock(anchor());
    const other = document.createElement("p");
    other.append(document.createTextNode("Another block."));
    expect(
      sourceOffsetForCaretPoint(other, tail, CLICK_IN_TAIL, "Another block."),
    ).toBeNull();
  });

  it("has no preference when the reported node is an element", () => {
    const { root } = splitBlock(anchor());
    // On an element node the offset counts children, not characters.
    expect(
      sourceOffsetForCaretPoint(
        root,
        root,
        1,
        "See [[Some Note|alias]] TAILWORD here.",
      ),
    ).toBeNull();
  });

  it("has no preference without a block root or without a node", () => {
    const { root, tail } = splitBlock(anchor());
    expect(sourceOffsetForCaretPoint(null, tail, 0, "See alias")).toBeNull();
    expect(sourceOffsetForCaretPoint(root, null, 0, "See alias")).toBeNull();
  });
});
