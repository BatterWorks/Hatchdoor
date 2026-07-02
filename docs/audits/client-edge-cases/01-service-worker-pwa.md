# Service worker / PWA / offline / install / cache staleness

**3 confirmed findings (1 high, 1 medium, 1 low), 1 refuted.**

## Confirmed findings

### HIGH: Service-worker auto-update reloads the page mid-edit and destroys unsaved new-note content (create draft never persisted)

- **Affected clients:** Desktop Chrome, Edge, Firefox, desktop Safari (WebKit), iOS Safari, iOS installed PWA, Android Chrome
- **Location:** `frontend/src/components/NoteActionsDialog.tsx:194`
- **What happens:** When a new service worker deploys, registerType:\"autoUpdate\" in vite.config.ts makes the bundled registerSW force an unconditional `window.location.reload()` on all open clients. The Create-note form contains an uncontrolled textarea with no onChange handler or persisted state. Users composing a new note lose all typed content when the page reloads mid-edit.
- **Why:** The Create-note form's textarea (`<textarea name="content">` at line 194) stores content only in the DOM with no localStorage persistence. The `createDraftKey()` function exists in `writeDrafts.ts` (lines 12–14) but is imported nowhere and never used by the create path, unlike the Edit path which persists draftContent to localStorage on every keystroke. When the auto-update reload fires, the uncontrolled textarea is destroyed with no recovery mechanism.
- **Fix sketch:** Persist the create-dialog textarea to localStorage under `createDraftKey()` on input (debounced), restore it when the dialog opens, and clear it on successful note creation. Alternatively, make the dialog controlled and lift content into App state. Independently, gate the auto-reload listener so it does not fire while an edit or create form is dirty.

### MEDIUM: No periodic SW update check — iOS standalone PWA can run stale cached JS against a newer backend for an entire session

- **Affected clients:** iOS Safari, iOS installed PWA, desktop Safari (WebKit)
- **Location:** `frontend/src/main.tsx:9–14`
- **What happens:** registerSW is called with only `immediate:true` and `onNeedRefresh`; no periodic `registration.update()` interval is configured. On iOS WebKit standalone PWAs that are backgrounded and resumed, the browser does not re-fetch the service worker script, so no update check fires. The old precached JS/CSS bundle persists indefinitely while the backend API has moved forward, risking frontend/backend contract drift.
- **Why:** Engine-specific to WebKit/iOS standalone: Chrome and Firefox re-check the SW byte-for-byte on each navigation, but iOS standalone PWAs commonly suspend without triggering a navigation, so without an app-driven periodic `update()` call the old precache persists much longer than expected.
- **Fix sketch:** Add an `onRegisteredSW(swUrl, r)` callback that calls `r && setInterval(() => r.update(), 60*60*1000)` and also triggers `r.update()` on `visibilitychange`/focus events, so long-lived iOS PWA sessions pick up new deploys without manual relaunch.

### LOW: Installable PWA has no API runtime caching — offline yields a working shell but every note/tree request fails

- **Affected clients:** iOS installed PWA, Android Chrome, Desktop Chrome, Edge
- **Location:** `frontend/vite.config.ts:37–45`
- **What happens:** The workbox config sets only `globPatterns` (app shell precache) and `navigateFallbackDenylist`; there is no `runtimeCaching` entry for /api routes. The manifest declares `display:standalone` and full icon set, so the app is installable and presents as offline-capable. Offline, the precached shell loads but all API requests fail, yielding an empty app rather than true offline content reading.
- **Why:** All install-capable engines cache and serve the shell while offline because index.html is precached, but none can return /api data since nothing caches it. This mismatch between the install affordance and offline capability creates a degraded experience.
- **Fix sketch:** Either add a NetworkFirst/StaleWhileRevalidate `runtimeCaching` rule for read-only GET `/api/note` and `/api/tree` so recently viewed content survives offline, or reset expectations by not promoting install or explicitly showing an offline-unavailable state so users do not install expecting offline reading capability.

## Refuted (not real / already handled)

- **onNeedRefresh in main.tsx is dead code under autoUpdate — there is no hook to flush drafts before the forced reload**  
  The callback is unreachable under autoUpdate mode, but the app is a read-only vault frontend with no user-editable forms, so the missing save hook has no functional impact on any named client. The reload still occurs correctly via the library's activated listener; the dead code is a code-quality issue, not a behavioral defect.
