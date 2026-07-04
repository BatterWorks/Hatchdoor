import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import { VitePWA } from "vite-plugin-pwa";

export default defineConfig({
  plugins: [
    react(),
    VitePWA({
      registerType: "autoUpdate",
      manifest: {
        name: "Hatchdoor",
        short_name: "Hatchdoor",
        description: "Read-only Obsidian vault web frontend",
        start_url: "/",
        display: "standalone",
        background_color: "#f4f1e8",
        theme_color: "#f4f1e8",
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
        runtimeCaching: [
          {
            urlPattern: ({ request, url }) =>
              request.method === "GET" && url.pathname === "/api/tree",
            handler: "NetworkFirst",
            options: {
              cacheName: "hatchdoor-api-tree",
              networkTimeoutSeconds: 4,
              expiration: {
                maxEntries: 1,
                maxAgeSeconds: 7 * 24 * 60 * 60,
              },
            },
          },
          {
            urlPattern: ({ request, url }) =>
              request.method === "GET" &&
              /^\/api\/note\/[^/]+$/.test(url.pathname),
            handler: "NetworkFirst",
            options: {
              cacheName: "hatchdoor-api-note",
              networkTimeoutSeconds: 4,
              expiration: {
                maxEntries: 80,
                maxAgeSeconds: 7 * 24 * 60 * 60,
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
    },
  },
  test: {
    environment: "jsdom",
    setupFiles: "./src/test/setup.ts",
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
