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

describe("sourceOffsetForRenderedOffset inside an escaped wikilink", () => {
  it("places the caret inside the alias of a table-cell link", () => {
    // Rendered as "battergate": clicking before "gate" is rendered offset 6.
    const source = "[[Host\\|battergate]]";
    expect(sourceOffsetForRenderedOffset(source, 6)).toBe(
      source.indexOf("battergate") + 6,
    );
  });

  it("counts only the alias as visible text after an escaped link", () => {
    const source = "[[Host\\|battergate]] tail";
    expect(sourceOffsetForRenderedOffset(source, 11)).toBe(
      source.indexOf(" tail") + 1,
    );
  });
});

describe("blocks whose source spans more than one line (#284)", () => {
  // A wrapped list item is addressed one source line at a time (D25a), so its
  // continuation line reaches the map alone, indent and all.
  it("skips the indent of a continuation line that arrives on its own", () => {
    expect(sourceOffsetForRenderedOffset("  continues here", 0)).toBe(2);
  });

  it("skips a wider indent, which is what nesting a list produces", () => {
    expect(sourceOffsetForRenderedOffset("      continues here", 0)).toBe(6);
  });

  it("skips the indent of a continuation line within a whole item", () => {
    // Rendered "first line\ncontinues here"; offset 11 is the c of continues.
    expect(
      sourceOffsetForRenderedOffset("- first line\n  continues here", 11),
    ).toBe(15);
  });

  it("skips the wider indent of a nested item's continuation line", () => {
    expect(
      sourceOffsetForRenderedOffset("  - first line\n    continues here", 11),
    ).toBe(19);
  });

  it("leaves an unindented wrapped paragraph where it already landed", () => {
    // Correct before the change: a newline is one character on both sides and
    // there is no indent to skip.
    expect(
      sourceOffsetForRenderedOffset("one two three\nfour five six", 14),
    ).toBe(14);
  });

  it("skips the indent of a wrapped paragraph's continuation line", () => {
    expect(
      sourceOffsetForRenderedOffset("one two three\n    four five six", 14),
    ).toBe(18);
  });

  it("strips the quote arrow of every line, not only the first", () => {
    // Rendered "first line\nsecond line"; offset 11 is the s of second.
    expect(
      sourceOffsetForRenderedOffset("> first line\n> second line", 11),
    ).toBe(15);
  });

  it("strips nested quote arrows, which the map's own copy of the rule did not", () => {
    expect(sourceOffsetForRenderedOffset("> > deep", 0)).toBe(4);
  });

  it("puts a caret on the soft break at the end of the line it closes", () => {
    expect(sourceOffsetForRenderedOffset("one two\nnext", 7)).toBe(7);
  });
});

describe("fenced code blocks (#284)", () => {
  // Offset 4 in the rendered code is the x of `let x = 1;`.
  const CODE_CLICK = 4;

  it("skips the opening fence and its language", () => {
    expect(
      sourceOffsetForRenderedOffset("```javascript\nlet x = 1;\n```", CODE_CLICK),
    ).toBe(18);
  });

  it("skips a shorter language name by exactly its width", () => {
    expect(
      sourceOffsetForRenderedOffset("```rust\nlet x = 1;\n```", CODE_CLICK),
    ).toBe(12);
  });

  it("skips a tilde fence too", () => {
    expect(
      sourceOffsetForRenderedOffset("~~~js\nlet x = 1;\n~~~", CODE_CLICK),
    ).toBe(10);
  });

  it("counts the newline between two code lines", () => {
    // Rendered "first\nsecond"; offset 6 is the s of second.
    expect(sourceOffsetForRenderedOffset("```\nfirst\nsecond\n```", 6)).toBe(10);
  });

  it("treats a code line's indentation as text, because it is", () => {
    expect(sourceOffsetForRenderedOffset("```\n  indented\n```", 0)).toBe(4);
  });

  it("treats markdown syntax inside code as text, because it is", () => {
    // Rendered "**not bold**"; offset 2 is the n.
    expect(sourceOffsetForRenderedOffset("```\n**not bold**\n```", 2)).toBe(6);
  });

  it("lands at the end of the code when the click is past it", () => {
    expect(sourceOffsetForRenderedOffset("```js\nab\n```", 99)).toBe(8);
  });

  it("maps a block whose closing fence has not been typed yet", () => {
    expect(sourceOffsetForRenderedOffset("```js\nab", 1)).toBe(7);
  });

  it("strips the container's indent from a fence written inside one", () => {
    // Rendered "code"; the two spaces belong to the list item holding the
    // fence, not to the code, so offset 0 is the c.
    expect(sourceOffsetForRenderedOffset("  ```js\n  code\n  ```", 0)).toBe(10);
  });

  it("keeps a code line's own indent, past the container's", () => {
    // Rendered "  deeper"; offset 0 is the first of the two spaces that are
    // part of the code.
    expect(
      sourceOffsetForRenderedOffset("  ```js\n    deeper\n  ```", 0),
    ).toBe(10);
  });

  it("puts the caret between the fences when there is no code yet", () => {
    expect(sourceOffsetForRenderedOffset("```\n```", 0)).toBe(4);
  });
});
