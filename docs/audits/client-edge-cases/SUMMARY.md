# Client edge-case audit — launch-readiness rollup

Pre-public-launch audit of client/browser edge cases in the Hatchdoor React 19 + Vite PWA
frontend (`frontend/src`). Targets: Chrome, Edge, Firefox, desktop + iOS Safari (WebKit),
Android Chrome. All 7 categories found (Opus), adversarially verified (Sonnet panel,
severity-gated), and reported.

**Totals: 5 high · 12 medium · 7 low (low = unverified).** One cross-cutting issue
(missing `viewport-fit=cover`) was independently surfaced by both category 04 and 05.

## Top launch blockers (high)

| Category | Title | Affected clients | Location |
|---|---|---|---|
| 01 SW/PWA | Auto-update `location.reload()` fires mid-edit and destroys unsaved **new-note** content (create draft never persisted) | All | `components/NoteActionsDialog.tsx:194` |
| 02 Upload | Image normalizer re-encodes through canvas **without applying EXIF orientation** → sideways photos | iOS/Android camera uploads | `imageUpload.ts` |
| 03 Rendering | `GraphPage` redraws unconditionally at 60fps (per-frame O(n²) label deconfliction + `getComputedStyle`) → pegs mobile CPU/GPU on large vaults | Mobile WebKit/Blink | `GraphPage.tsx` |
| 04 / 05 | Viewport meta lacks **`viewport-fit=cover`** → every `env(safe-area-inset-*)` resolves to 0 (notch/Dynamic Island/home-indicator overlap, worst in installed PWA) | iOS Safari + PWA | `frontend/index.html:14` |

The 04/05 finding is the same root cause reported twice — **one meta-tag fix** unblocks all
the safe-area CSS. Note category 05 then flags a follow-on medium (double top inset) that
only manifests *after* this fix lands.

## All confirmed findings by severity

| Sev | Category | Title | Clients | Location |
|---|---|---|---|---|
| HIGH | 01 | Auto-update reload destroys unsaved new-note content | All | `NoteActionsDialog.tsx:194` |
| HIGH | 02 | Canvas re-encode drops EXIF orientation | iOS/Android photos | `imageUpload.ts` |
| HIGH | 03 | Graph redraws at 60fps, pegs mobile on large vaults | Mobile | `GraphPage.tsx` |
| HIGH | 04 | Missing `viewport-fit=cover` defeats safe-area insets | iOS | `index.html:14` |
| HIGH | 05 | (same) viewport meta lacks `viewport-fit=cover` | iOS | `index.html` |
| MED | 01 | No periodic SW update check — stale JS vs newer backend a whole session | iOS standalone | `vite.config.ts` |
| MED | 02 | `copyNoteLink` uses `navigator.clipboard` directly, no fallback, swallows errors | All | `clipboard.ts` |
| MED | 02 | Paste-to-upload only reads `clipboardData.files`, misses WebKit `items/getAsFile` | WebKit | `imageUpload.ts` |
| MED | 03 | Canvas buffer-resize recenters transform, discards user pan | All | `GraphPage.tsx` |
| MED | 03 | Read view clips wide KaTeX display equations (overflow fix only in editor preview) | All | `renderers.tsx` |
| MED | 04 | `theme-color` meta static light, never updated for dark mode | iOS/Android | `index.html:8` |
| MED | 05 | Hotbar + topbar both add safe-area-inset-top → double top inset once viewport fixed | iOS | `topbar.css` / `responsive.css` |
| MED | 05 | Create/rename modal centered on 100vh, no `visualViewport` → keyboard hides inputs | Mobile | `responsive.css` |
| MED | 06 | Unpolyfilled `.at(-1)` crashes on Safari < 15.4 while build target floor is safari14 | Old WebKit | build target |
| MED | 07 | Attachment upload silently capped at axum's 2 MB default vs advertised 10 MB | All | `src/` axum config |
| MED | 07 | `EventSource` for `/api/vault-events` has no onerror/close → silent infinite 401 reconnect, TokenPrompt never surfaces | All | `api.ts` |
| MED | 07 | No fetch timeout / AbortController anywhere → permanent spinners after network transitions | iOS Safari/PWA | `api.ts` |
| LOW* | 01 | Installable PWA has no API runtime caching — offline shell works but every note/tree request fails | All | `vite.config.ts` |
| LOW* | 02 | `execCommand` copy fallback (readonly textarea) unreliable on iOS WebKit | iOS | `clipboard.ts` |
| LOW* | 03 | Graph canvas has no `touch-action:none` — relies solely on JS preventDefault | Mobile | `GraphPage.tsx` |
| LOW* | 04 | Unguarded `:hover` on mobile-only element → sticky highlight on touch | Touch | `topbar.css:267` |
| LOW* | 05 | Note body / editor textarea have no `dir=auto` → RTL content mis-renders | All | `responsive.css` |
| LOW* | 06 | `copyNoteLink` unguarded `writeText`, no execCommand fallback | Older WebKit | `clipboard.ts` |
| LOW* | 06 | Folder autocomplete uses `<datalist>`, degrades on iOS/older WebKit | iOS | frontmatter UI |

`LOW*` = unverified: the adversarial panel did not vote on low-severity findings (severity-gated).

## Notes

- **Refuted / already-handled:** 01 (1), 02 findings clean, 03 (0), 06 (1 — search-dialog
  `autoFocus` keyboard claim was refuted). See each `NN-*.md` for the refutation reasons.
- **Backend-adjacent items** live in category 07 (the `api.ts` ↔ Rust seam): the 2 MB upload
  cap and error-shape contract also belong to the planned **Workflow 2 — backend robustness audit**.
- Per-category detail (what happens / why / fix sketch) is in the seven `NN-*.md` files.
