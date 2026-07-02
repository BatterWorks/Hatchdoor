# Responsive layout / safe-area / viewport / RTL / font-scaling

4 confirmed (1 high, 2 medium, 1 low unverified), 0 refuted.

## Confirmed findings

### HIGH: Viewport meta lacks viewport-fit=cover, so every env(safe-area-inset-*) resolves to 0

- **Affected clients:** iOS Safari (WebKit), iOS installed PWA (WebKit), Android Chrome (notch/gesture devices)
- **Location:** `frontend/index.html:14`
- **What happens:** The viewport meta is `<meta name="viewport" content="width=device-width, initial-scale=1.0" />` with no `viewport-fit=cover`. Per the WebKit/CSS spec, `env(safe-area-inset-*)` only receives non-zero values when the viewport is `viewport-fit=cover`; otherwise all four insets compute to 0px. The codebase relies on these insets in ~8 places: hotbar height (topbar.css:4-9), topbar padding-top (topbar.css / responsive.css:10,144), note-pane bottom padding (responsive.css:95,148), the mobile bottom action menu (responsive.css:53), and the full-screen search panel (responsive.css:86-87). All of that safe-area code is effectively dead.
- **Why:** On a notched iPhone the topbar/hotbar sit flush against the top edge (the intended inset accent is only 3px), and in the installed PWA the note body bottom content and the mobile bottom action menu render underneath the home-indicator bar because env(safe-area-inset-bottom) is 0. The entire notch/home-indicator hardening never activates on the clients it targets.
- **Fix sketch:** Add `viewport-fit=cover` to the meta content, then re-test top/bottom insets on a notched device (mind the double-inset finding first).

### MEDIUM: Hotbar element and topbar padding both add safe-area-inset-top (double top inset once viewport-fit is fixed)

- **Affected clients:** iOS Safari (WebKit), iOS installed PWA (WebKit)
- **Location:** `frontend/src/styles/topbar.css:4`
- **What happens:** `.hotbar` (rendered directly above `.app-topbar` in AppTopbar.tsx:87-88) sets `height: calc(3px + env(safe-area-inset-top))`, consuming the full top inset as a physical element. The topbar that follows ALSO pads by the same inset: standalone rule `padding-top: calc(0.9rem + env(safe-area-inset-top))` (responsive.css:144) and the <=920px rule `padding: calc(0.6rem + env(safe-area-inset-top)) ...` (responsive.css:10). The inset is counted twice in the vertical stack.
- **Why:** Latent today because insets are 0, but adding viewport-fit=cover over-corrects: on a notched iPhone (~47-59px inset) the header gains a full extra inset of dead space, pushing the wordmark/actions down. Fixing the viewport bug silently activates this one.
- **Fix sketch:** Pick one consumer of env(safe-area-inset-top): keep the hotbar as the inset spacer and drop the inset from the topbar padding, or make the hotbar a fixed 3px and let only the topbar pad for the inset.

### MEDIUM: Create/rename modal is vertically centered with 100vh cap and no visualViewport handling, so the on-screen keyboard hides its inputs

- **Affected clients:** iOS Safari (WebKit), iOS installed PWA (WebKit)
- **Location:** `frontend/src/App.css:331`
- **What happens:** The write-mode dialog (NoteActionsDialog.tsx:88-102, create/rename/move forms with input/textarea) renders in `.modal-backdrop` which is `position: fixed; inset: 0; display: flex; align-items: center` (App.css:320-329), and `.modal-panel` is capped at `max-height: min(720px, calc(100vh - 2rem))` (App.css:333). On iOS WebKit, 100vh and a fixed inset:0 layer are sized to the large keyboard-independent viewport and are not shrunk when the keyboard opens; unlike the mobile drawer (App.tsx:291-306) this modal has no visualViewport listener.
- **Why:** Tapping a field to name a new note on iPhone brings up the keyboard covering the lower ~40-50%, but the centered modal stays centered against the full viewport, pushing the focused input and the create/cancel buttons behind the keyboard with no way to scroll them into view (the backdrop does not scroll). Note creation is effectively blocked on iOS portrait for anything but the top field.
- **Fix sketch:** Use 100dvh for the cap and/or align-items: flex-start with top padding, and add a visualViewport resize listener (as App.tsx already does for the drawer) to constrain the modal to window.visualViewport.height.

### LOW: Note body and editor textarea have no dir=auto, so RTL vault content renders left-aligned with misplaced punctuation (unverified)

- **Affected clients:** Desktop Chrome, Edge, Firefox, desktop Safari, iOS Safari, Android Chrome
- **Location:** `frontend/src/styles/note-content.css:48`
- **What happens:** `<html lang="en">` (index.html:2) with no dir attribute anywhere, and no dir="auto"/direction on the rendered note body (.note-content/.note-body, note-content.css:24-50) or the write-mode editor (.note-editor-textarea, note-content.css:80-92; .modal-panel textarea, App.css:365). Directional styling is all physical: active-note `box-shadow: inset 3px 0 0` (layout-explorer.css:92), tree border-left (layout-explorer.css:178), padding-left indents.
- **Why:** The vault can contain Arabic/Hebrew notes. Without dir="auto" on the prose container and textarea, RTL paragraphs render as LTR: text hugs the left edge and trailing punctuation/parentheses jump to the wrong end of the line. Engine-agnostic (all clients) but only affects RTL content.
- **Fix sketch:** Add dir="auto" to the rendered note content wrapper and both editor textareas so the bidi algorithm picks direction per block from the first strong character.

## Refuted (not real / already handled)

(No refuted findings.)
