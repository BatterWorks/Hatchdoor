import { describe, expect, it } from "vitest";

import { sourceOffsetForRenderedOffset } from "./caretMap";

describe("sourceOffsetForRenderedOffset", () => {
  it("is an identity mapping for plain text", () => {
    expect(sourceOffsetForRenderedOffset("hello world", 6)).toBe(6);
  });

  it("skips a leading heading marker", () => {
    // Rendered "A heading"; clicking before "heading" is rendered offset 2.
    expect(sourceOffsetForRenderedOffset("## A heading", 2)).toBe(5);
  });

  it("skips a leading list marker", () => {
    expect(sourceOffsetForRenderedOffset("- two", 0)).toBe(2);
  });

  it("skips a leading task marker", () => {
    expect(sourceOffsetForRenderedOffset("- [ ] task", 0)).toBe(6);
  });

  it("skips a leading quote marker", () => {
    expect(sourceOffsetForRenderedOffset("> quoted", 0)).toBe(2);
  });

  it("steps over emphasis markers", () => {
    // Rendered "Second paragraph here."; offset 7 is the p of paragraph.
    expect(sourceOffsetForRenderedOffset("Second *paragraph* here.", 7)).toBe(
      8,
    );
  });

  it("steps over bold markers", () => {
    expect(sourceOffsetForRenderedOffset("a **bold** word", 2)).toBe(4);
  });

  it("steps over inline code ticks", () => {
    expect(sourceOffsetForRenderedOffset("use `code` now", 4)).toBe(5);
  });

  it("keeps a link's label offsets and skips its target", () => {
    // Rendered "see docs here"; offset 4 is the o of docs.
    expect(sourceOffsetForRenderedOffset("see [docs](/a/b) here", 5)).toBe(6);
  });

  it("lands at end of line when the click is past the text", () => {
    expect(sourceOffsetForRenderedOffset("- two", 99)).toBe(5);
  });

  it("never returns a negative offset", () => {
    expect(sourceOffsetForRenderedOffset("- two", -3)).toBe(2);
  });
});

describe("wikilinks", () => {
  // The dominant link syntax in this vault, and it was unhandled: a click after
  // a wikilink landed inside the brackets.
  it("maps past a wikilink's brackets", () => {
    // Rendered "see Note here"; offset 9 is the h of here.
    expect(sourceOffsetForRenderedOffset("see [[Note]] here", 9)).toBe(13);
  });

  it("keeps offsets inside a wikilink's label", () => {
    expect(sourceOffsetForRenderedOffset("see [[Note]] here", 4)).toBe(6);
  });

  it("maps past an aliased wikilink, which renders only its alias", () => {
    // Rendered "see Alias here"; offset 10 is the h of here.
    expect(sourceOffsetForRenderedOffset("see [[Note|Alias]] here", 10)).toBe(
      19,
    );
  });

  it("maps past an embed", () => {
    expect(sourceOffsetForRenderedOffset("![[a.png]] after", 1)).toBe(11);
  });
});
