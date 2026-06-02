# Logo Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace all existing brand marks (topbar text/dot, favicons, PWA icons) with the new SVG logos, with full light/dark theme support.

**Architecture:** The wordmark is inlined as SVG in `AppTopbar.tsx` using `currentColor` and `var(--hot)` so it inherits theme colours from CSS without any JS logic. Favicons and PWA icons are rasterised from the icon SVG at build time via a one-off Node script using `@resvg/resvg-js`, with a `#f4f1e8` background baked in (light theme — standard for app icons).

**Tech Stack:** React/TSX, inline SVG, CSS custom properties, `@resvg/resvg-js`, `png-to-ico`

---

## File Map

| File | Change |
|------|--------|
| `frontend/src/app/AppTopbar.tsx` | Replace dot + text brand with inline SVG wordmark |
| `frontend/src/styles/topbar.css` | Remove `.topbar-brand-dot` rule, add `.brand-wordmark` sizing |
| `frontend/scripts/gen-icons.mjs` | New: one-off script to generate all PNG icon sizes + favicon.ico |
| `frontend/public/favicon-16x16.png` | Replaced by script |
| `frontend/public/favicon-32x32.png` | Replaced by script |
| `frontend/public/favicon-64x64.png` | Replaced by script |
| `frontend/public/favicon.ico` | Replaced by script |
| `frontend/public/apple-touch-icon.png` | Replaced by script |
| `frontend/public/android-chrome-192x192.png` | Replaced by script |
| `frontend/public/android-chrome-512x512.png` | Replaced by script |
| `frontend/public/icons/icon-*.png` | Replaced by script |
| `frontend/vite.config.ts` | Fix PWA manifest `background_color` + `theme_color` to match brand tokens |

---

## Task 1: Inline SVG wordmark in the topbar

**Files:**
- Modify: `frontend/src/app/AppTopbar.tsx:62-76`
- Modify: `frontend/src/styles/topbar.css:34-40`

The wordmark SVG uses `fill="currentColor"` for the bracket mark and text so they inherit `color: var(--ink)` from `.topbar-brand`. The orange accent uses `fill="var(--hot)"` directly. No JS, no theme prop needed.

- [ ] **Step 1: Replace the brand mark in AppTopbar.tsx**

In `frontend/src/app/AppTopbar.tsx`, replace the brand content (lines ~62–76):

```tsx
{/* Col 1 — Brand */}
<div className="topbar-brand">
  {isMobile ? (
    <button
      type="button"
      className="icon-button"
      onClick={onToggleDrawer}
      aria-label="Toggle explorer"
      style={{ marginRight: "0.5rem" }}
    >
      ☰
    </button>
  ) : null}
  <span className="topbar-brand-dot" aria-hidden="true" />
  HATCHDOOR
</div>
```

Replace with:

```tsx
{/* Col 1 — Brand */}
<div className="topbar-brand">
  {isMobile ? (
    <button
      type="button"
      className="icon-button"
      onClick={onToggleDrawer}
      aria-label="Toggle explorer"
      style={{ marginRight: "0.5rem" }}
    >
      ☰
    </button>
  ) : null}
  <svg
    className="brand-wordmark"
    viewBox="0 0 340 60"
    aria-label="Hatchdoor"
    role="img"
    focusable="false"
  >
    {/* Left bracket */}
    <rect x="4" y="4" width="9" height="52" fill="currentColor" />
    <rect x="4" y="4" width="16" height="9" fill="currentColor" />
    <rect x="4" y="47" width="16" height="9" fill="currentColor" />
    {/* Right bracket */}
    <rect x="47" y="4" width="9" height="52" fill="currentColor" />
    <rect x="40" y="4" width="16" height="9" fill="currentColor" />
    <rect x="40" y="47" width="16" height="9" fill="currentColor" />
    {/* Accent square */}
    <rect x="24" y="24" width="12" height="12" fill="var(--hot)" />
    {/* Wordmark text */}
    <text className="brand-wordmark-text" x="76" y="47">
      HATCHDOOR
    </text>
  </svg>
</div>
```

- [ ] **Step 2: Update topbar.css**

In `frontend/src/styles/topbar.css`, remove the `.topbar-brand-dot` rule (lines ~34–40):

```css
.topbar-brand-dot {
  width: 9px;
  height: 9px;
  background: var(--hot);
  display: inline-block;
  flex-shrink: 0;
}
```

And replace with:

```css
.brand-wordmark {
  height: 1.45rem;
  width: auto;
  display: block;
  flex-shrink: 0;
  overflow: visible;
}

.brand-wordmark-text {
  fill: currentColor;
  font-family: var(--font-display);
  font-size: 46px;
  font-weight: 800;
  font-variation-settings: "wdth" 78, "wght" 800;
  letter-spacing: 0.04em;
}
```

- [ ] **Step 3: Run the app and check visually**

```bash
cd frontend && npm run dev
```

Open http://localhost:5173 and verify:
- Desktop: wordmark visible in topbar, bracket mark on left, "HATCHDOOR" text on right
- Toggle dark theme (◑ button): brackets and text go cream, orange stays orange
- Mobile (resize below 920px): hamburger button + wordmark fit, nothing overflows

- [ ] **Step 4: Commit**

```bash
git add frontend/src/app/AppTopbar.tsx frontend/src/styles/topbar.css
git commit -m "feat(brand): replace topbar dot+text with inline SVG wordmark"
```

---

## Task 2: Generate PNG icon set

**Files:**
- Create: `frontend/scripts/gen-icons.mjs`
- Replace: all files in `frontend/public/` and `frontend/public/icons/` listed in the file map

This script rasterises the bracket icon mark SVG at all required sizes with a `#f4f1e8` background baked in. Run it once; the output PNGs are committed to the repo.

- [ ] **Step 1: Install dependencies**

```bash
cd frontend && npm install -D @resvg/resvg-js png-to-ico
```

Expected: packages added to `devDependencies`, no errors.

- [ ] **Step 2: Create the generation script**

Create `frontend/scripts/gen-icons.mjs`:

```js
import { Resvg } from "@resvg/resvg-js";
import pngToIco from "png-to-ico";
import { writeFileSync, mkdirSync, readFileSync } from "fs";
import { resolve, dirname } from "path";
import { fileURLToPath } from "url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const publicDir = resolve(__dirname, "../public");
const iconsDir = resolve(__dirname, "../public/icons");

mkdirSync(iconsDir, { recursive: true });

// Icon SVG with light-theme background baked in (for all raster outputs)
const ICON_SVG = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 60 60">
  <rect width="60" height="60" fill="#f4f1e8"/>
  <rect x="4" y="4" width="9" height="52" fill="#0c0c0a"/>
  <rect x="4" y="4" width="16" height="9" fill="#0c0c0a"/>
  <rect x="4" y="47" width="16" height="9" fill="#0c0c0a"/>
  <rect x="47" y="4" width="9" height="52" fill="#0c0c0a"/>
  <rect x="40" y="4" width="16" height="9" fill="#0c0c0a"/>
  <rect x="40" y="47" width="16" height="9" fill="#0c0c0a"/>
  <rect x="24" y="24" width="12" height="12" fill="#ff4d1c"/>
</svg>`;

function renderPng(size) {
  const resvg = new Resvg(ICON_SVG, {
    fitTo: { mode: "width", value: size },
  });
  return resvg.render().asPng();
}

const sizes = [16, 32, 48, 64, 120, 152, 167, 180, 192, 512];

console.log("Generating icons...");

// favicon-*.png in /public
for (const size of [16, 32, 64]) {
  const png = renderPng(size);
  writeFileSync(resolve(publicDir, `favicon-${size}x${size}.png`), png);
  console.log(`  favicon-${size}x${size}.png`);
}

// apple-touch-icon (180x180)
writeFileSync(resolve(publicDir, "apple-touch-icon.png"), renderPng(180));
console.log("  apple-touch-icon.png");

// android-chrome
writeFileSync(
  resolve(publicDir, "android-chrome-192x192.png"),
  renderPng(192)
);
console.log("  android-chrome-192x192.png");

writeFileSync(
  resolve(publicDir, "android-chrome-512x512.png"),
  renderPng(512)
);
console.log("  android-chrome-512x512.png");

// /public/icons/*
for (const size of sizes) {
  const png = renderPng(size);
  writeFileSync(resolve(iconsDir, `icon-${size}.png`), png);
  console.log(`  icons/icon-${size}.png`);
}

// favicon.ico — bundle 16, 32, 48
const icoBuffers = [renderPng(16), renderPng(32), renderPng(48)];
const ico = await pngToIco(icoBuffers);
writeFileSync(resolve(publicDir, "favicon.ico"), ico);
console.log("  favicon.ico");

console.log("Done.");
```

- [ ] **Step 3: Run the script**

```bash
cd frontend && node scripts/gen-icons.mjs
```

Expected output:
```
Generating icons...
  favicon-16x16.png
  favicon-32x32.png
  favicon-64x64.png
  apple-touch-icon.png
  android-chrome-192x192.png
  android-chrome-512x512.png
  icons/icon-16.png
  ... (all sizes)
  favicon.ico
Done.
```

- [ ] **Step 4: Verify icons look correct**

Open `frontend/public/favicon-32x32.png` and `frontend/public/android-chrome-512x512.png` in an image viewer. Should see the bracket mark on a warm off-white (`#f4f1e8`) background with the orange accent square.

- [ ] **Step 5: Commit**

```bash
git add frontend/scripts/gen-icons.mjs frontend/public/ && \
git add frontend/package.json frontend/package-lock.json
git commit -m "feat(brand): regenerate favicon and PWA icon set from SVG"
```

---

## Task 3: Fix PWA manifest colours

**Files:**
- Modify: `frontend/vite.config.ts`

Current `background_color` and `theme_color` in the manifest are slightly off from the design tokens. Fix them to match.

- [ ] **Step 1: Update vite.config.ts**

In `frontend/vite.config.ts`, update the `VitePWA` manifest block:

```ts
manifest: {
  name: "Hatchdoor",
  short_name: "Hatchdoor",
  description: "Read-only Obsidian vault web frontend",
  start_url: "/",
  display: "standalone",
  background_color: "#f4f1e8",   // was "#f4f2ec" — matches --bg light token
  theme_color: "#f4f1e8",        // was "#ece8da" — consistent with background
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
```

- [ ] **Step 2: Commit**

```bash
git add frontend/vite.config.ts
git commit -m "fix(pwa): align manifest background_color and theme_color to brand tokens"
```

---

## Task 4: Run tests and verify nothing broken

- [ ] **Step 1: Run test suite**

```bash
cd frontend && npm test
```

Expected: all tests pass (the topbar tests don't assert on brand mark content, so no changes needed).

- [ ] **Step 2: Run typecheck**

```bash
cd frontend && npm run typecheck
```

Expected: no errors.

- [ ] **Step 3: Run lint**

```bash
cd frontend && npm run lint
```

Expected: no errors.
