# Client edge-case audit

Pre-public-launch audit of client/browser edge cases for the Hatchdoor React PWA
frontend (`frontend/src`). Targets: Chrome, Edge, Firefox, desktop + iOS Safari
(WebKit), Android Chrome.

## How it runs

A disk-backed, resumable workflow. Each category is found (Opus), adversarially
verified (3× Sonnet-high refutation per finding — a finding survives only if it
is NOT refuted by a majority), and written to its own file by a scribe (Haiku).

**A category is "done" when its `NN-slug.md` file exists below.** That file IS the
checkpoint — there is no separate mutable ledger to corrupt.

## Resuming after an interruption (e.g. 5-hour limit)

In a fresh session, say: **"resume the client edge-case audit"**. The workflow
re-scans this directory, skips every category whose file already exists, and runs
only the remaining ones. Same-session resume is automatic; cross-session resume
relies entirely on the files in this folder.

## Categories

| File | Scope | Key sources |
|---|---|---|
| `01-service-worker-pwa.md` | Workbox SW, autoUpdate reload, offline, install, cache staleness | `vite.config.ts`, `main.tsx` |
| `02-clipboard-upload-download.md` | Clipboard, image upload, file download (iOS PWA download) | `clipboard.ts`, `imageUpload.ts`, `App.links-download.test.tsx` |
| `03-rendering-engines.md` | mermaid, KaTeX, react-markdown, d3-force graph across WebKit/Gecko | `components/note-page/renderers.tsx`, `RendererComponents.tsx`, `GraphPage.tsx`, `markdown.ts` |
| `04-matchmedia-theme-touch.md` | matchMedia, theme, touch vs hover vs pointer, gestures | `app/useIsMobile.ts`, `app/useTheme.ts`, `app/AppTopbar.tsx` |
| `05-responsive-safe-area.md` | Viewport, safe-area insets, dvh/vh, RTL, font scaling | `styles/responsive.css`, `base.css`, `layout-explorer.css`, `topbar.css` |
| `06-browser-api-compat.md` | `<dialog>`, date inputs, crypto, Web Share, structuredClone, etc. | `components/ui.tsx`, `NoteActionsDialog.tsx`, `SearchDialog.tsx`, `note-page/frontmatter.ts` |
| `07-network-auth-seam.md` | fetch/timeout/retry, CORS, auth token, error-shape contract with Rust API | `api.ts`, `writeApi.ts`, `components/TokenPrompt.tsx` |

`SUMMARY.md` is regenerated from the per-category files on each full pass.
