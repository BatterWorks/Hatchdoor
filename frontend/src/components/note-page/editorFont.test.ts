import { describe, expect, it } from "vitest";

import { resolveFont } from "./editorFont";

const HEADING = {
  // What Chromium reports for a heading block: the shorthand collapses to the
  // empty string because font-variation-settings cannot fold back into it.
  font: "",
  fontStyle: "normal",
  fontWeight: "700",
  fontSize: "27.2px",
  lineHeight: "39.44px",
  fontFamily: '"Bricolage Grotesque", system-ui, sans-serif',
};

describe("resolveFont", () => {
  it("uses the shorthand when the browser gives one", () => {
    expect(
      resolveFont({
        ...HEADING,
        font: "italic 500 23.2px / 27.376px Newsreader, Georgia, serif",
      }),
    ).toBe("italic 500 23.2px / 27.376px Newsreader, Georgia, serif");
  });

  // #286: an empty shorthand was passed straight to the canvas, which ignores
  // it, so a heading was silently measured in whatever font the shared context
  // held last and its prefix hung far too little.
  it("composes the longhands when the shorthand is empty", () => {
    expect(resolveFont(HEADING)).toBe(
      'normal 700 27.2px/39.44px "Bricolage Grotesque", system-ui, sans-serif',
    );
  });

  it("omits a line height of normal, which the shorthand cannot carry alone", () => {
    expect(resolveFont({ ...HEADING, lineHeight: "normal" })).toBe(
      'normal 700 27.2px "Bricolage Grotesque", system-ui, sans-serif',
    );
  });

  it("produces a font string a canvas actually accepts", () => {
    const ctx = document.createElement("canvas").getContext("2d");
    if (!ctx) {
      return;
    }
    ctx.font = "10px sans-serif";
    ctx.font = resolveFont(HEADING);
    expect(ctx.font).not.toBe("10px sans-serif");
  });
});
