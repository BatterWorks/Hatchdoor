import { describe, expect, it } from "vitest";

import baseCss from "./styles/base.css?raw";
import indexHtml from "../index.html?raw";
import { fillThemeColors, readThemeBackgrounds } from "../vite.config.ts";

// The browser bar colour in index.html and the installed app's splash in the
// manifest come from `--bg` in base.css at build time (#648), so a palette
// change reaches them and nothing restates the hex by hand.

const CSS = `
:root[data-theme="light"],
:root[data-theme="auto"] {
  --bg: #111111;
  --paper: #222222;
}
:root[data-theme="dark"] {
  --paper: #333333;
  --bg: #444444;
}
@media (prefers-color-scheme: dark) {
  :root[data-theme="auto"] {
    --bg: #444444;
  }
}
`;

describe("theme colours from base.css (#648)", () => {
  it("reads --bg from the light and the dark token blocks", () => {
    expect(readThemeBackgrounds(CSS)).toEqual({
      light: "#111111",
      dark: "#444444",
    });
  });

  it("follows a change to --bg", () => {
    expect(readThemeBackgrounds(CSS.replace("#111111", "#abcdef")).light).toBe(
      "#abcdef",
    );
  });

  it("refuses a stylesheet whose dark block has no --bg", () => {
    expect(() =>
      readThemeBackgrounds(CSS.replace("--bg: #444444;\n}", "}")),
    ).toThrow(/dark/);
  });

  it("reads the shipped stylesheet", () => {
    const { light, dark } = readThemeBackgrounds(baseCss);
    expect(light).toMatch(/^#[0-9a-f]{6}$/i);
    expect(dark).toMatch(/^#[0-9a-f]{6}$/i);
    expect(light).not.toBe(dark);
  });

  it("fills both theme-color tags in index.html and leaves no placeholder", () => {
    const html = fillThemeColors(indexHtml, {
      light: "#111111",
      dark: "#444444",
    });

    expect(html).toMatch(
      /<meta\s+name="theme-color"\s+content="#111111"\s+media="\(\s*prefers-color-scheme:\s*light\s*\)"/,
    );
    expect(html).toMatch(
      /<meta\s+name="theme-color"\s+content="#444444"\s+media="\(\s*prefers-color-scheme:\s*dark\s*\)"/,
    );
    expect(html).not.toContain("%THEME_BG");
  });

  it("refuses a page that lost a placeholder", () => {
    expect(() =>
      fillThemeColors("<html></html>", { light: "#111111", dark: "#444444" }),
    ).toThrow(/THEME_BG_LIGHT/);
  });
});
