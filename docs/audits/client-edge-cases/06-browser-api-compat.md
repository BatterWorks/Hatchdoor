# Browser-API compatibility

**Summary:** 3 confirmed findings (1 medium, 2 low unverified), 1 refuted.

## Confirmed findings

### MEDIUM: Unpolyfilled Array/String .at(-1) crashes on Safari < 15.4 while build target floor is safari14

- **Affected clients:** desktop Safari 14–15.3, iOS Safari 14–15.3, iOS installed PWA 14–15.3
- **Location:** `frontend/src/components/note-page/wikilinks.ts:146`, `frontend/src/components/NotePage.tsx:333`
- **What happens:** Code calls `Array.prototype.at()` and `String.prototype.at()` directly without polyfill or guard. On Safari/iOS 14.x–15.3 these methods are undefined, so the calls throw `TypeError: parts.at is not a function`. In wikilinks.ts the crash occurs during the wikilink-label render path (hot path), breaking the entire note render; in NotePage.tsx:333 it breaks heading-anchored navigation.
- **Why:** `vite.config.ts` sets no `build.target`, so Vite/esbuild use the default 'modules' preset with baseline `[es2020, edge88, firefox78, chrome87, safari14]`. esbuild only down-levels syntax (e.g., spread, arrow functions), not runtime methods like `.at()`. The `.at()` method first shipped in Safari/iOS 15.4 (March 2022), creating a gap between the declared build floor (`safari14`) and the API actually used in the shipped code.
- **Fix sketch:** Either set `build.target` to a floor that guarantees `.at()` support (safari15.4+) and document it, or replace `.at(-1)` with `parts[parts.length - 1]` to maintain the declared safari14 floor.

### LOW: copyNoteLink calls navigator.clipboard.writeText unguarded with no execCommand fallback (unverified)

- **Affected clients:** iOS Safari (non-HTTPS / self-hosted HTTP), any client in an insecure context
- **Location:** `frontend/src/App.tsx:515`
- **What happens:** The `copyNoteLink` function calls `await navigator.clipboard.writeText(window.location.href)` directly. `navigator.clipboard` is undefined outside a secure context (http:// origins are common in self-hosted LAN deployments). The call throws silently in the try/catch, leaving no fallback and no user feedback; the 'copy note link' button appears to work but does nothing.
- **Why:** In an insecure context, `navigator.clipboard` is `undefined`, so `.writeText()` throws. Everywhere else in the codebase (clipboard.ts) the copy flow guards `navigator.clipboard?.writeText()` and falls back to a textarea + `execCommand`. This code path bypasses that helper entirely. Self-hosters on plain HTTP get a dead button; HTTPS deployments are unaffected.
- **Fix sketch:** Route `copyNoteLink` through the existing `copyText()` helper in `clipboard.ts` to gain the `?.` guard and `execCommand` fallback, and surface a failure notice instead of swallowing the error.

### LOW: Folder autocomplete uses &lt;datalist&gt;, which degrades on iOS/older WebKit (unverified)

- **Affected clients:** iOS Safari (varies by version), iOS installed PWA
- **Location:** `frontend/src/components/NoteActionsDialog.tsx:143–148` (FolderDatalist), inputs at 179–186 and 263–270
- **What happens:** The Create and Move folder forms use `<input list=...>` + `<datalist>` for folder autocomplete. On affected iOS WebKit the dropdown of existing folder paths does not appear, so the autocomplete affordance is silently missing. The plain text input still works, so it is graceful degradation rather than a hard break.
- **Why:** datalist rendering is uneven across WebKit versions. iOS Safari historically lacked datalist dropdown UI, and support remains inconsistent across iOS versions; the element is also unstyled by default.
- **Fix sketch:** If folder suggestions are important on mobile, replace datalist with a custom filtered dropdown (autocomplete.ts already has a matching pattern elsewhere). Otherwise accept the degradation explicitly and document it.

## Refuted (not real / already handled)

**Search dialog input relies on React autoFocus; iOS WebKit will not raise the soft keyboard**  
React 18 flushes updates from discrete events (click, keydown) synchronously within the gesture-handling stack rather than deferring to later microtasks, so autoFocus-driven `.focus()` calls commit within the same synchronous gesture context. The iOS keyboard reliably appears when the dialog is opened by a tap. The finding's premise about React's commit timing is inaccurate.
