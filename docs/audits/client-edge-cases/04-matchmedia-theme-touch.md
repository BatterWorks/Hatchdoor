# matchMedia / theme / touch vs hover vs pointer

**Summary:** 3 confirmed (1 high, 1 medium, 1 low unverified), 0 refuted. Note that LOW findings are included as "unverified" (panel did not vote on them).

## Confirmed findings

### HIGH: Missing viewport-fit=cover defeats every env(safe-area-inset-*) rule on iOS

**Affected clients:**
- iOS Safari (WebKit)
- iOS installed PWA (WebKit)

**Location:** `frontend/index.html:14`

**What happens:** The viewport meta is `content="width=device-width, initial-scale=1.0"` with no `viewport-fit=cover`. Yet the CSS relies on env(safe-area-inset-*) in four files: topbar.css:4 (.hotbar height calc(3px + env(safe-area-inset-top))), responsive.css:10/53/87/95/144/148 (topbar top padding, actions-menu bottom offset, search panel, note pane, standalone padding), plus note-content.css and noteEnhancements.css. Every safe-area guard silently collapses: the mobile actions menu, pinned with `bottom: calc(0.6rem + env(safe-area-inset-bottom))` (responsive.css:53), sits under the home indicator; the standalone PWA topbar padding (responsive.css:144) collapses so the header underlaps the status bar / Dynamic Island; the .hotbar accent renders as a bare 3px strip beneath the notch. Worst in the installed PWA because apple-mobile-web-app-capable=yes (index.html:7) makes it fullscreen.

**Why:** WebKit only resolves env(safe-area-inset-*) to non-zero values when the page opts into the full display area via viewport-fit=cover. Without it, all four insets evaluate to 0 on notched/Dynamic-Island iPhones. Chrome/Firefox/desktop unaffected.

**Fix sketch:** Change the meta to `content="width=device-width, initial-scale=1.0, viewport-fit=cover"` so the existing safe-area CSS actually takes effect.

---

### MEDIUM: theme-color meta is a static light color and is never updated for dark mode

**Affected clients:**
- iOS Safari (WebKit)
- iOS installed PWA (WebKit)
- Android Chrome

**Location:** `frontend/index.html:8`

**What happens:** Only one `<meta name="theme-color" content="#f4f1e8" />` exists (light cream). No `media="(prefers-color-scheme: dark)"` variant, and useTheme.ts (cycleTheme / the theme useEffect at useTheme.ts:14-21) only mutates document.documentElement.dataset.theme and localStorage, never the theme-color meta. A grep for theme-color across the frontend returns only this single static tag. In dark mode --bg is #0c0c0a (base.css:98 auto-dark, base.css:51 explicit dark) but the surrounding chrome stays #f4f1e8, producing a bright cream bar clashing with the dark UI. It never corrects when the user cycles to Dark or when the system is dark under theme=auto. Desktop browsers ignore theme-color. No apple-mobile-web-app-status-bar-style is set either, compounding the standalone case.

**Why:** iOS Safari 15+, the iOS standalone PWA, and Android Chrome paint the browser chrome / status bar band from theme-color. The theme system never updates this meta at runtime for manual overrides or system preference changes.

**Fix sketch:** Add a second theme-color meta with `media="(prefers-color-scheme: dark)"` for the theme=auto path, and in useTheme's effect update the meta content to match the resolved bg for manual light/dark overrides.

---

### LOW: Unguarded :hover on a mobile-only element leaves a sticky highlight on touch (unverified)

**Affected clients:**
- iOS Safari (WebKit)
- Android Chrome

**Location:** `frontend/src/styles/topbar.css:267`

**What happens:** `.topbar-mobile-path:hover { background: var(--paper-2) }` is not wrapped in @media (hover: hover), unlike the sibling .icon-button and .topbar-search-trigger hovers in the same file (topbar.css:116 and :160). This element renders ONLY on mobile: topbar.css:242 sets .topbar-mobile-meta { display: none } and responsive.css:44 flips it to display: block below 920px. The same unguarded pattern exists for explorer/search touch targets (layout-explorer.css:78 .note-link:hover, :164 folder summary:hover; search.css:111/132/159; ui-common.css:23). On return from the search overlay, the --paper-2 background stays stuck.

**Why:** On touch WebKit/Blink, tapping applies :hover and keeps it until the user taps elsewhere (no pointer to move away). The mobile path button opens the search overlay rather than navigating, so on return the highlight remains. The author gated hover for the icon buttons but missed this touch-only element and the shared explorer/search styles. Purely cosmetic.

**Fix sketch:** Wrap these :hover rules in @media (hover: hover) so they never fire on touch, matching the existing guarded rules in topbar.css.

---

## Refuted (not real / already handled)

*None.*
