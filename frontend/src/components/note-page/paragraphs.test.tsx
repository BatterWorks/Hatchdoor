import { describe, expect, it } from "vitest";

import { splitAtSoftBreaks } from "./paragraphs";

describe("splitAtSoftBreaks", () => {
  // Consecutive "> " lines parse as one paragraph joined by soft breaks, so
  // addressing a callout per line means splitting the rendered children back
  // apart at those breaks.
  it("splits a single string on its newlines", () => {
    expect(splitAtSoftBreaks(["one\ntwo\nthree"])).toEqual([
      ["one"],
      ["two"],
      ["three"],
    ]);
  });

  it("returns one line when there is no break", () => {
    expect(splitAtSoftBreaks(["just one"])).toEqual([["just one"]]);
  });

  it("keeps inline elements on the line they belong to", () => {
    const code = <code key="c">x</code>;
    expect(splitAtSoftBreaks(["use ", code, " here\nnext line"])).toEqual([
      ["use ", code, " here"],
      ["next line"],
    ]);
  });

  it("puts an element after a break on the following line", () => {
    const link = <a key="l">link</a>;
    expect(splitAtSoftBreaks(["before\n", link, " after"])).toEqual([
      ["before"],
      [link, " after"],
    ]);
  });

  it("drops nothing when a line is only an element", () => {
    const strong = <strong key="s">bold</strong>;
    expect(splitAtSoftBreaks(["a\n", strong, "\nb"])).toEqual([
      ["a"],
      [strong],
      ["b"],
    ]);
  });

  it("returns an empty list for no children", () => {
    expect(splitAtSoftBreaks([])).toEqual([]);
  });

  // A line's index is what maps it back to a source line, so an interior line
  // has to hold its place even when it carries nothing.
  it("keeps an interior empty line so later lines keep their index", () => {
    expect(splitAtSoftBreaks(["one\n\nthree"])).toEqual([
      ["one"],
      [],
      ["three"],
    ]);
  });

  it("drops a trailing empty line, which shifts nothing", () => {
    expect(splitAtSoftBreaks(["one\ntwo\n"])).toEqual([["one"], ["two"]]);
  });
});
