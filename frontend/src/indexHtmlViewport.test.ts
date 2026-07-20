import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it } from "vitest";

// The CSS in this app relies on env(safe-area-inset-*) in several files
// (topbar.css, responsive.css, note-content.css, noteEnhancements.css).
// WebKit/iOS only resolves those insets to non-zero values when the page
// opts into the full display area via viewport-fit=cover. Without it, every
// safe-area guard silently collapses on notched/Dynamic-Island iPhones,
// which is especially bad in the installed (apple-mobile-web-app-capable)
// PWA. Guard the viewport meta so the opt-in is never dropped.
describe("index.html viewport meta", () => {
  // vitest runs with the frontend package root as cwd.
  const indexHtml = readFileSync(resolve(process.cwd(), "index.html"), "utf8");

  it("declares viewport-fit=cover so env(safe-area-inset-*) resolves on iOS", () => {
    const viewportMeta = indexHtml.match(
      /<meta[^>]*name=["']viewport["'][^>]*>/i,
    );
    expect(viewportMeta).not.toBeNull();

    const content = viewportMeta![0].match(/content=["']([^"']*)["']/i)?.[1];
    expect(content).toBeDefined();

    const tokens = content!.split(",").map((part) => part.trim().toLowerCase());
    expect(tokens).toContain("viewport-fit=cover");
  });
});
