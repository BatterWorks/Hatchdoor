// The note width's caps (#698), read from the stylesheets themselves: jsdom
// lays nothing out, so the live check measures the drawn column and this
// keeps each width tied to its token.
import { describe, expect, it } from "vitest";

import baseCss from "./base.css?raw";
import noteContentCss from "./note-content.css?raw";

/** The declarations of the first rule whose selector is exactly `selector`. */
function rule(css: string, selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = new RegExp(`(?:^|})\\s*${escaped}\\s*\\{([^}]*)\\}`, "m").exec(
    css,
  );
  if (!match) throw new Error(`no rule for ${selector}`);
  return match[1];
}

describe("note width caps (#698)", () => {
  it("keeps Default at today's 38rem and sets Wide at 48rem", () => {
    const root = rule(baseCss, ":root");
    expect(root).toMatch(/--note-measure-default:\s*38rem;/);
    expect(root).toMatch(/--note-measure-wide:\s*48rem;/);
  });

  it("measures the note column from the Default token unless a width says otherwise", () => {
    const layout = rule(noteContentCss, ".note-page-layout");
    expect(layout).toMatch(/--note-measure:\s*var\(--note-measure-default\);/);
    expect(layout).toMatch(
      /max-width:\s*calc\(var\(--note-measure\) \+ 1\.4rem \+ 220px\);/,
    );
  });

  it("gives Wide its token outside Source, and Full no cap at all", () => {
    expect(
      rule(
        noteContentCss,
        '.note-page-layout[data-note-width="wide"]:not(:has(.note-editor))',
      ),
    ).toMatch(/--note-measure:\s*var\(--note-measure-wide\);/);
    expect(
      rule(noteContentCss, '.note-page-layout[data-note-width="full"]'),
    ).toMatch(/max-width:\s*none;/);
  });
});
