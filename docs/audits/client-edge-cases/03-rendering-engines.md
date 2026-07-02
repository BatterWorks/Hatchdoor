# Markdown / mermaid / KaTeX / d3-force graph rendering across engines

**3 confirmed (1 high + 2 medium), 0 refuted. 1 low unverified.**

## Confirmed findings

### HIGH: GraphPage redraws unconditionally at 60fps with per-frame O(n log n) sort + O(n²) label deconfliction + repeated getComputedStyle — pegs mobile CPU/GPU on large vaults

- **Affected clients**: iOS Safari, iOS installed PWA, Android Chrome, desktop Safari
- **Location**: `frontend/src/components/GraphPage.tsx:152-453`
- **What happens**: `startLoop()` (447-453) schedules `requestAnimationFrame(tick)` forever with no settle/idle detection — `render()` runs every frame even after the d3-force simulation has cooled and nothing moves. Each `render()` call does heavy work: it calls `cssVar()` (getComputedStyle on document.documentElement) six times (204-209), sorts every node by backlink_count to compute the hub threshold (337: nodes.map(...).sort(...)), rebuilds the label-candidate list, and runs collidesWithPlaced (375-376) which does placed.some() over a 'placed' array pre-seeded with every visible node (365-370) for every label candidate — O(n²) in node count. measureText is also called per candidate per frame (390). All of this repeats ~60×/second indefinitely.
- **Why**: getComputedStyle forces a synchronous style recalc each call; six per frame at 60fps is a known WebKit/Blink reflow sink, and WebKit's getComputedStyle is comparatively expensive. Combined with the per-frame O(n²) deconfliction and sort, a vault with hundreds+ of nodes keeps a mobile GPU/CPU fully busy with no frames ever skipped. On iOS Safari/PWA this drains battery, heats the device, and triggers the OS frame-rate throttle; the page is effectively janky/unusable for large graphs. Desktop engines absorb it better but still spin a core continuously.
- **Fix sketch**: Stop the rAF loop once sim.alpha() falls below alphaMin and only restart it on interaction/hover/zoom/drag. Hoist the six cssVar() reads out of the hot path (read once per theme change, cache in a ref). Compute the hub threshold and the node 'placed' seed array once per data/zoom change rather than every frame.

### MEDIUM: Canvas buffer-resize inside render() recenters the transform and discards the user's pan whenever clientWidth/clientHeight changes

- **Affected clients**: iOS Safari, iOS installed PWA
- **Location**: `frontend/src/components/GraphPage.tsx:163-173`
- **What happens**: Every frame `render()` compares the canvas backing-store size to `clientWidth*dpr / clientHeight*dpr` and, on any mismatch, sets `canvas.width/height` and then resets `transformRef` to `{ x: cssW/2, y: cssH/2, k }`. This throws away the user's panned x/y origin (only k is preserved) on every canvas size change.
- **Why**: On iOS Safari and the installed PWA the canvas CSS height changes during normal use — the URL/toolbar collapses and expands as the user scrolls, the dynamic safe-area/viewport shifts, and rotation changes dimensions. Each of those changes `clientHeight`, which makes this code path fire and snap the graph back to center, losing the user's pan position mid-interaction. On desktop this only happens on a deliberate window resize, so it's far less visible there; on iOS it recenters during ordinary scrolling.
- **Fix sketch**: When resizing the backing store, preserve the existing pan: keep `transformRef.x/y` (or translate them by half the delta of the size change) instead of hard-resetting to `cssW/2, cssH/2`. Only do the center-on-first-valid-size once (the resize effect already has a `centred` guard — `render()` should not re-center at all).

### MEDIUM: Read view clips wide KaTeX display equations — overflow-x:auto fix is applied only to the editor preview, not to the published note body

- **Affected clients**: iOS Safari, iOS installed PWA, Android Chrome, desktop Safari, Firefox, Chrome, Edge
- **Location**: `frontend/src/App.css:256-260`
- **What happens**: KaTeX's own `.katex-display` rule has no overflow handling (node_modules/katex/dist/katex.min.css: `.katex-display{display:block;margin:1em 0;text-align:center}`). The app adds `overflow-x:auto; overflow-y:hidden` to `.katex-display` only under `.note-editor-preview` (App.css 256-260). The actual read view (NotePage's `.note-body`) gets no such rule, and its scroll container `.note-pane` has `overflow-x:hidden` (frontend/src/styles/note-content.css:8-9). A display equation wider than the column therefore overflows and is clipped — there is no horizontal scroll to reach the rest of the equation.
- **Why**: This is a layout/overflow gap rather than an engine bug, but its impact is worst on the narrow-viewport target clients — iOS Safari/PWA and Android Chrome — where a non-trivial `\displaystyle` equation or a long aligned environment routinely exceeds the available width and gets silently cut off (the right side of the formula is unreadable). The editor preview shows it correctly, so authors won't notice the published view is truncated.
- **Fix sketch**: Add the same overflow treatment to the read view, e.g. `.note-body .katex-display { overflow-x: auto; overflow-y: hidden; }`, mirroring the `.note-editor-preview` rule (ideally factor it into a shared selector covering both).

### LOW: Graph canvas has no touch-action:none — relies solely on JS preventDefault to suppress native scroll/zoom gestures (unverified)

- **Affected clients**: Android Chrome, iOS Safari, iOS installed PWA
- **Location**: `frontend/src/styles/graph.css:149-154`
- **What happens**: `.graph-canvas` sets cursor but no touch-action. Gesture suppression depends entirely on the JS handlers calling `e.preventDefault()` in non-passive touchstart/touchmove listeners (GraphPage.tsx 674-815, registered with `{ passive: false }`).
- **Why**: On Android Chrome the compositor can begin a scroll/pinch on the first touch before the non-passive JS handler runs and cancels it, producing a one-frame jank; on iOS, gestures handled above the touch-event layer (page pinch-zoom, double-tap-zoom) are not reliably stopped by preventDefault alone. `touch-action: none` declares the intent to the compositor up front and is the correct belt-and-suspenders. The JS preventDefault mitigates most cases, hence low severity.
- **Fix sketch**: Add `touch-action: none;` (and `overscroll-behavior: contain;`) to `.graph-canvas` so the browser never starts a native pan/zoom on the element.

## Refuted (not real / already handled)

(None.)
