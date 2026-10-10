import { describe, expect, it } from "vitest";

import appCss from "./App.css?raw";
import appSource from "./App.tsx?raw";
import graphPageSource from "./components/graph/GraphPage.tsx?raw";
import notePageSource from "./components/NotePage.tsx?raw";
import noteContentCss from "./styles/note-content.css?raw";
import noteEditorSource from "./components/NoteEditor.tsx?raw";
import tokenPromptSource from "./components/TokenPrompt.tsx?raw";
import wikilinksSource from "./components/note-page/wikilinks.ts?raw";
import mainSource from "./main.tsx?raw";
import graphCss from "./styles/graph.css?raw";
import explorerCss from "./styles/layout-explorer.css?raw";
import responsiveCss from "./styles/responsive.css?raw";
import searchCss from "./features/search/search.css?raw";
import topbarCss from "./styles/topbar.css?raw";
import uiCss from "./styles/ui-common.css?raw";
import indexHtml from "../index.html?raw";
import viteConfig from "../vite.config.ts?raw";

describe("client audit launch contracts", () => {
  it("opts the installed PWA into safe-area viewport insets", () => {
    expect(indexHtml).toMatch(
      /<meta\s+name="viewport"\s+content="[^"]*viewport-fit=cover[^"]*"/,
    );
  });

  it("ships separate light and dark theme-color metadata", () => {
    expect(indexHtml).toMatch(
      /<meta\s+name="theme-color"\s+content="#f4f1e8"\s+media="\(\s*prefers-color-scheme:\s*light\s*\)"/,
    );
    expect(indexHtml).toMatch(
      /<meta\s+name="theme-color"\s+content="#0c0c0a"\s+media="\(\s*prefers-color-scheme:\s*dark\s*\)"/,
    );
    expect(indexHtml).toMatch(
      /<meta\s+name="apple-mobile-web-app-status-bar-style"\s+content="black-translucent"/,
    );
  });

  // WebKit throws from the `localStorage` accessor itself when site data is
  // blocked (#339). The pre-paint theme script runs before React, outside any
  // boundary, so it has to guard its own read and still pick a theme.
  it("applies the auto theme pre-paint when reading storage throws", () => {
    const script = /<script>([\s\S]*?)<\/script>/.exec(indexHtml)?.[1];
    expect(script).toBeTruthy();
    const blockedStorage = {
      getItem() {
        throw new DOMException("The operation is insecure.", "SecurityError");
      },
    };
    const root = { dataset: {} as Record<string, string> };
    const run = new Function("localStorage", "document", script!);
    expect(() => run(blockedStorage, { documentElement: root })).not.toThrow();
    expect(root.dataset.theme).toBe("auto");
  });

  it("checks for service-worker updates during long-lived PWA sessions", () => {
    expect(mainSource).toContain("onRegisteredSW");
    expect(mainSource).toContain("registration.update()");
    expect(mainSource).toContain("visibilitychange");
  });

  // `update()` rejects whenever the worker script cannot be fetched, which is
  // every tick while offline (#332). Coming back to the tab fires both `focus`
  // and `visibilitychange`, so listening to both doubled every check.
  it("swallows an offline update check and checks once per return to the tab", () => {
    expect(mainSource).toMatch(/registration\s*\.update\(\)\s*\.catch\(/);
    expect(mainSource).not.toMatch(/void\s+registration\.update\(\)/);
    expect(mainSource).not.toMatch(/addEventListener\(\s*"focus"/);
  });

  // Android Chrome paints the installed app's splash and task-switcher card
  // from the manifest, which has no media-query form (#332). A dark splash
  // under a light UI is the milder flash of the two.
  it("gives the installed PWA a dark splash rather than a light one", () => {
    expect(viteConfig).toMatch(/background_color:\s*"#0c0c0a"/);
    expect(viteConfig).toMatch(/theme_color:\s*"#0c0c0a"/);
  });

  // `autoUpdate` activates a new worker the moment it installs and reloads the
  // page with no prompt, between keystrokes (#330). Both the check for an
  // update and the reload itself go through the editor's hold.
  it("does not reload for a service-worker update while an edit is unsaved", () => {
    expect(mainSource).toContain("isAppReloadHeld");
    expect(mainSource).toContain("onNeedReload");
    expect(mainSource).toMatch(
      /onNeedReload\(\)\s*{\s*whenAppReloadReleased\(\(\)\s*=>\s*window\.location\.reload\(\)\)/s,
    );
    expect(mainSource).not.toMatch(
      /onNeedRefresh\(\)\s*{\s*window\.location\.reload\(\)/s,
    );
  });

  it("does not runtime-cache authenticated API data in the service worker", () => {
    expect(viteConfig).not.toContain("hatchdoor-api-tree");
    expect(viteConfig).not.toContain("hatchdoor-api-note");
    expect(viteConfig).not.toContain("/api/tree");
    expect(viteConfig).not.toContain("api\\/note");
  });

  it("does not ship runtime .at() calls below the configured Safari floor", () => {
    expect(wikilinksSource).not.toContain(".at(");
    expect(notePageSource).not.toContain(".at(");
  });

  it("lets note prose and editor fields resolve RTL direction automatically", () => {
    expect(notePageSource).toMatch(/className="note-body"[^>]*dir="auto"/s);
    expect(noteEditorSource).toMatch(
      /className="note-editor-textarea"[^>]*dir="auto"/s,
    );
  });

  it("adds trailing scroll space only once the reader jumps to a heading", () => {
    // Plain reading ends where the note's text ends; the space exists solely
    // so an end-of-note heading can reach the top of the pane, and only a
    // heading jump ever needs that.
    expect(noteContentCss).toMatch(
      /\.note-content\s*{[^}]*padding-bottom:\s*3rem/s,
    );
    expect(noteContentCss).toMatch(
      /\.note-content\[data-tail="true"\]\s*{[^}]*padding-bottom:\s*max\(3rem,\s*calc\(100dvh/s,
    );
    expect(notePageSource).toMatch(/data-tail=\{tailArmed\}/);
  });

  it("lets KaTeX display equations scroll horizontally in read view", () => {
    expect(appCss).toMatch(/\.note-body\s+:where\([^)]*\.katex-display/);
    expect(appCss).toMatch(
      /\.note-body\s+:where\([^)]*\.katex-display[^}]*overflow-x:\s*auto/s,
    );
  });

  it("declares graph canvas touch gestures to the browser compositor", () => {
    expect(graphCss).toMatch(
      /\.graph-canvas\s*{[^}]*touch-action:\s*none[^}]*overscroll-behavior:\s*contain/s,
    );
  });

  it("does not recenter the graph transform during canvas buffer resizes", () => {
    expect(graphPageSource).not.toMatch(
      /transformRef\.current\s*=\s*{\s*x:\s*cssW\s*\/\s*2,\s*y:\s*cssH\s*\/\s*2/s,
    );
  });

  it("uses the hotbar as the sole top safe-area spacer on mobile", () => {
    expect(responsiveCss).not.toMatch(
      /\.app-topbar\s*{[^}]*env\(safe-area-inset-top\)/s,
    );
  });

  it("sizes modal dialogs against the visual viewport instead of static 100vh", () => {
    expect(appCss).toContain("--visual-viewport-height");
    expect(appCss).toMatch(
      /\.modal-backdrop\s*{[^}]*align-items:\s*flex-start/s,
    );
    expect(appCss).toMatch(
      /\.modal-panel\s*{[^}]*max-height:\s*min\(720px,\s*calc\(var\(--visual-viewport-height,\s*100dvh\)/s,
    );
  });

  it("lets the sidebar grid read the live --sidebar-width off the shell", () => {
    // App.tsx sets --sidebar-width inline on `.app-shell`. A local
    // declaration on `.app-layout` shadows that inherited value, so the
    // resizer moved only the topbar column (which reads the inherited one)
    // while the pane stayed pinned at the shadowed default.
    expect(appSource).toMatch(
      /app-shell[\s\S]{0,400}?"--sidebar-width":\s*`\$\{sidebarWidth\}px`/,
    );
    expect(explorerCss).not.toMatch(
      /\.app-layout\s*{[^}]*--sidebar-width:\s*\d/s,
    );
    expect(explorerCss).toMatch(
      /\.app-layout\s*{[^}]*grid-template-columns:\s*var\(--sidebar-width,\s*280px\)/s,
    );
  });

  it("guards touch-sticky hover styles behind hover-capable media queries", () => {
    expect(topbarCss).toMatch(
      /@media\s*\(hover:\s*hover\)\s*{[^}]*\.topbar-scope-trigger:hover/s,
    );
    expect(explorerCss).toMatch(
      /@media\s*\(hover:\s*hover\)\s*{[^}]*\.note-link:hover/s,
    );
    expect(explorerCss).toMatch(
      /@media\s*\(hover:\s*hover\)\s*{[^}]*\.folder-item summary:hover/s,
    );
    expect(searchCss).toMatch(
      /@media\s*\(hover:\s*hover\)\s*{[^}]*\.search-result--primary:hover/s,
    );
    expect(searchCss).toMatch(
      /@media\s*\(hover:\s*hover\)\s*{[^}]*\.search-result--chunk:hover/s,
    );
    expect(searchCss).toMatch(
      /@media\s*\(hover:\s*hover\)\s*{[^}]*\.search-group-toggle:hover/s,
    );
    expect(uiCss).toMatch(
      /@media\s*\(hover:\s*hover\)\s*{[^}]*\.ui-button:hover,\s*\.close-note:hover/s,
    );
  });

  // The bullet inset used to be declared on `:root` in a stylesheet section
  // that belonged to something else, and went when that section did (#547).
  // A plain bullet then padded by an undefined property, which is zero, and
  // its dash landed on the text.
  it("declares the bullet list inset beside the list rules that read it", () => {
    expect(noteContentCss).toMatch(
      /\.note-body ul li,\s*\.note-body ol li\s*{[^}]*padding-left:\s*var\(--li-inset\)/s,
    );
    expect(noteContentCss).toMatch(
      /\.note-body ul\s*{[^}]*--li-inset:\s*1\.4rem/s,
    );
  });

  // None of these five names is a Forge token. A rule that reads one always
  // renders its fallback, so it ignores the theme (#547).
  it("reads none of the five undefined properties #547 removed", () => {
    const sources = import.meta.glob(
      ["./**/*.{css,ts,tsx}", "!./**/*.test.*"],
      {
        query: "?raw",
        import: "default",
        eager: true,
      },
    ) as Record<string, string>;
    expect(Object.keys(sources).length).toBeGreaterThan(50);
    const offenders = Object.entries(sources)
      .filter(([, source]) =>
        /var\(--(accent|surface|hover|border|text)\s*[,)]/.test(source),
      )
      .map(([path]) => path);
    expect(offenders).toEqual([]);
  });

  it("keeps raw colours and radii out of the token prompt and the app stylesheet", () => {
    // Comments cite issues as "#547", which reads as a hex colour.
    const withoutComments = (source: string) =>
      source.replace(/\/\*[\s\S]*?\*\//g, "");
    for (const source of [appCss, tokenPromptSource].map(withoutComments)) {
      expect(source).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
      expect(source).not.toMatch(/\brgba?\(/);
      const radii = [
        ...source.matchAll(/border-?[rR]adius"?\s*:\s*"?([^;",]+)/g),
      ].map((match) => match[1].trim());
      expect(
        radii.filter((radius) => !/^var\(--radius-[a-z]+\)$/.test(radius)),
      ).toEqual([]);
    }
  });

  it("tints the code block header from a token, squares the graph badge, and has no fixed grey for an untagged node", () => {
    expect(noteContentCss).not.toMatch(
      /\.code-block-head\s*{[^}]*background:\s*rgba?\(/s,
    );
    expect(graphCss).toMatch(
      /\.graph-filter-badge\s*{[^}]*border-radius:\s*var\(--radius-none\)/s,
    );
    expect(graphPageSource).not.toMatch(/rgba\(138, 134, 120/);
  });
});
