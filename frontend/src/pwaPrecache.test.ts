import { describe, expect, it } from "vitest";

import { findLazyChunks, type ChunkGraphNode } from "../vite.config.ts";

function chunk(
  fileName: string,
  fields: Partial<ChunkGraphNode> = {},
): ChunkGraphNode {
  return {
    fileName,
    imports: [],
    dynamicImports: [],
    isEntry: false,
    isDynamicEntry: false,
    facadeModuleId: null,
    ...fields,
  };
}

// The install-time precache is all-or-nothing, and Mermaid's diagram tree is
// most of what used to be in it (#332).
describe("service-worker precache boundary", () => {
  const bundle = [
    chunk("assets/index.js", {
      isEntry: true,
      imports: ["assets/react-vendor.js"],
      dynamicImports: [
        "assets/mermaid.core.js",
        "assets/pdf.js",
        "assets/workbox-window.js",
      ],
    }),
    chunk("assets/react-vendor.js"),
    chunk("assets/workbox-window.js", {
      isDynamicEntry: true,
      facadeModuleId: "/app/node_modules/workbox-window/build/index.mjs",
    }),
    chunk("assets/mermaid.core.js", {
      isDynamicEntry: true,
      facadeModuleId: "/app/node_modules/mermaid/dist/mermaid.core.mjs",
      // Shared helpers the shell also loads stay precached, and the walk does
      // not follow the shell's own dynamic imports from there.
      imports: ["assets/index.js", "assets/chunk-ABC.js"],
      dynamicImports: ["assets/flowDiagram.js"],
    }),
    chunk("assets/chunk-ABC.js"),
    chunk("assets/flowDiagram.js", {
      isDynamicEntry: true,
      facadeModuleId: "/app/node_modules/mermaid/dist/flowDiagram.mjs",
      imports: ["assets/cytoscape.esm.js", "assets/react-vendor.js"],
    }),
    chunk("assets/cytoscape.esm.js"),
    chunk("assets/pdf.js", {
      isDynamicEntry: true,
      facadeModuleId: "/app/node_modules/pdfjs-dist/build/pdf.mjs",
    }),
  ];

  it("leaves out everything only Mermaid or PDF.js reaches", () => {
    expect([...findLazyChunks(bundle)].sort()).toEqual([
      "assets/chunk-ABC.js",
      "assets/cytoscape.esm.js",
      "assets/flowDiagram.js",
      "assets/mermaid.core.js",
      "assets/pdf.js",
    ]);
  });

  it("keeps the app shell and its own lazy imports precached", () => {
    const lazy = findLazyChunks(bundle);
    expect(lazy.has("assets/index.js")).toBe(false);
    expect(lazy.has("assets/react-vendor.js")).toBe(false);
    expect(lazy.has("assets/workbox-window.js")).toBe(false);
  });
});

describe("service-worker precache wiring", () => {
  it("filters the precache by that boundary and caches those chunks on first use", async () => {
    const { default: viteConfig } = await import("../vite.config.ts?raw");
    expect(viteConfig).toMatch(
      /manifestTransforms:[\s\S]*!lazyChunks\.has\(entry\.url\)/,
    );
    expect(viteConfig).toMatch(
      /cacheName:\s*"hatchdoor-lazy-chunks"[\s\S]*handler|handler:\s*"CacheFirst"[\s\S]*cacheName:\s*"hatchdoor-lazy-chunks"/,
    );
  });
});
