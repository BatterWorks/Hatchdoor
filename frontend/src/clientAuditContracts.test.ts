import { describe, expect, it } from "vitest";

import appCss from "./App.css?raw";
import graphPageSource from "./components/graph/GraphPage.tsx?raw";
import notePageSource from "./components/NotePage.tsx?raw";
import noteEditorSource from "./components/NoteEditor.tsx?raw";
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

  it("checks for service-worker updates during long-lived PWA sessions", () => {
    expect(mainSource).toContain("onRegisteredSW");
    expect(mainSource).toContain("registration.update()");
    expect(mainSource).toContain("visibilitychange");
    expect(mainSource).toContain("focus");
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
});
