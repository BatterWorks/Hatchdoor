import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import type { Plugin, Rolldown } from "vite";
import { VitePWA } from "vite-plugin-pwa";

// Packages the app loads only through a dynamic `import()`, for a note that
// needs them: Mermaid for a diagram fence, PDF.js for an embedded PDF.
const LAZY_PACKAGES = ["/node_modules/mermaid/", "/node_modules/pdfjs-dist/"];

/** The part of an emitted chunk `findLazyChunks` reads. */
export type ChunkGraphNode = {
  fileName: string;
  imports: string[];
  dynamicImports: string[];
  isEntry: boolean;
  isDynamicEntry: boolean;
  facadeModuleId: string | null;
};

/**
 * Every chunk those imports pull in that the app shell does not already load:
 * Mermaid alone is some ninety diagram, parser and layout chunks. The
 * install-time precache is all-or-nothing, so each one in it is one more
 * request that can fail the whole install, for code most notes never run
 * (#332). Found from the bundle graph rather than by file name, because
 * Mermaid's chunk names change with every release.
 */
export function findLazyChunks(chunks: Iterable<ChunkGraphNode>): Set<string> {
  const byName = new Map<string, ChunkGraphNode>();
  for (const chunk of chunks) {
    byName.set(chunk.fileName, chunk);
  }
  const reach = (
    roots: string[],
    next: (chunk: ChunkGraphNode) => string[],
    stopAt: ReadonlySet<string> = new Set(),
  ) => {
    const seen = new Set<string>();
    const stack = [...roots];
    while (stack.length > 0) {
      const fileName = stack.pop()!;
      const chunk = byName.get(fileName);
      if (!chunk || seen.has(fileName) || stopAt.has(fileName)) {
        continue;
      }
      seen.add(fileName);
      stack.push(...next(chunk));
    }
    return seen;
  };
  const all = [...byName.values()];
  const eager = reach(
    all.filter((chunk) => chunk.isEntry).map((chunk) => chunk.fileName),
    (chunk) => chunk.imports,
  );
  return reach(
    all
      .filter(
        (chunk) =>
          chunk.isDynamicEntry &&
          LAZY_PACKAGES.some((dir) => chunk.facadeModuleId?.includes(dir)),
      )
      .map((chunk) => chunk.fileName),
    (chunk) => [...chunk.imports, ...chunk.dynamicImports],
    // A Mermaid chunk can import a shared helper from the app shell; the walk
    // must not continue from there into the shell's own lazy imports.
    eager,
  );
}

// Filled at `generateBundle`, read when the worker is generated afterwards.
let lazyChunks = new Set<string>();

function trackLazyChunks(): Plugin {
  return {
    name: "hatchdoor:track-lazy-chunks",
    apply: "build",
    generateBundle(_options, bundle) {
      lazyChunks = findLazyChunks(
        Object.values(bundle).filter(
          (file): file is Rolldown.OutputChunk => file.type === "chunk",
        ),
      );
    },
  };
}

export default defineConfig({
  plugins: [
    react(),
    trackLazyChunks(),
    VitePWA({
      registerType: "autoUpdate",
      manifest: {
        name: "Hatchdoor",
        short_name: "Hatchdoor",
        description: "Self-hosted Markdown vault web app",
        start_url: "/",
        display: "standalone",
        // Android Chrome paints the installed app's splash screen and its
        // task-switcher card from these, and the manifest has no light/dark
        // form (index.html carries both for the in-browser bar). Dark is the
        // milder mismatch: a brief dark splash before a light UI, instead of
        // a cream flash on every cold start in dark mode (#332).
        background_color: "#0c0c0a",
        theme_color: "#0c0c0a",
        icons: [
          {
            src: "/android-chrome-192x192.png",
            sizes: "192x192",
            type: "image/png",
          },
          {
            src: "/android-chrome-512x512.png",
            sizes: "512x512",
            type: "image/png",
          },
          {
            src: "/android-chrome-512x512.png",
            sizes: "512x512",
            type: "image/png",
            purpose: "maskable",
          },
        ],
      },
      workbox: {
        globPatterns: ["**/*.{js,css,html,svg,png,ico,woff2}"],
        // PDF.js is loaded only for an embedded PDF. Keeping its renderer and
        // worker out of the install-time precache preserves that lazy boundary.
        globIgnores: ["**/pdf-*.js", "**/pdf.worker*.mjs"],
        // The same boundary for everything else those two imports pull in,
        // above all Mermaid's diagram tree (see `trackLazyChunks`).
        manifestTransforms: [
          (entries) => ({
            manifest: entries.filter((entry) => !lazyChunks.has(entry.url)),
            warnings: [],
          }),
        ],
        // Out of the precache, those chunks are fetched the first time a note
        // needs them and kept from then on, so a Mermaid note that rendered
        // once still renders offline. Their names carry a content hash, so a
        // cached copy is never stale.
        runtimeCaching: [
          {
            urlPattern: ({ sameOrigin, url }) =>
              sameOrigin &&
              url.pathname.startsWith("/assets/") &&
              /\.m?js$/.test(url.pathname),
            handler: "CacheFirst",
            options: {
              cacheName: "hatchdoor-lazy-chunks",
              expiration: {
                maxEntries: 300,
                maxAgeSeconds: 60 * 60 * 24 * 30,
                purgeOnQuotaError: true,
              },
            },
          },
        ],
        // Keep the SPA navigation fallback from swallowing server routes. On
        // iOS standalone PWAs the `download` attribute is ignored and the
        // anchor click becomes a navigation; without this denylist the service
        // worker serves the cached index.html, so a `.md` download arrives as
        // an HTML file. Let these requests reach the network instead.
        navigateFallbackDenylist: [
          /^\/api\//,
          /^\/vault-assets\//,
          /^\/health/,
        ],
      },
    }),
  ],
  server: {
    port: 5173,
    proxy: {
      "/api": "http://127.0.0.1:42824",
      "/health": "http://127.0.0.1:42824",
      "/vault-assets": "http://127.0.0.1:42824",
    },
  },
  test: {
    environment: "jsdom",
    setupFiles: "./src/test/setup.ts",
    // One jsdom per worker instead of one per file: about 20 s instead of 80 s
    // on 4 cores. The setup file resets what files would otherwise leak.
    isolate: false,
    css: true,
    testTimeout: 15_000,
    coverage: {
      provider: "v8",
      reporter: ["text", "json-summary", "html"],
      include: ["src/**/*.{ts,tsx}"],
      exclude: ["src/**/*.test.{ts,tsx}", "src/test/**", "src/main.tsx"],
      thresholds: {
        lines: 70,
        functions: 70,
        statements: 70,
        branches: 60,
      },
    },
  },
  build: {
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (!id.includes("node_modules")) {
            return;
          }

          if (
            id.includes("/react/") ||
            id.includes("/react-dom/") ||
            id.includes("/react-router")
          ) {
            return "react-vendor";
          }

          if (
            id.includes("/react-markdown/") ||
            id.includes("/remark-") ||
            id.includes("/rehype-") ||
            id.includes("/micromark") ||
            id.includes("/mdast-") ||
            id.includes("/hast-") ||
            id.includes("/unified/") ||
            id.includes("/unist-") ||
            id.includes("/vfile")
          ) {
            return "markdown-vendor";
          }

          if (id.includes("/katex/")) {
            return "katex-vendor";
          }
        },
      },
    },
  },
});
