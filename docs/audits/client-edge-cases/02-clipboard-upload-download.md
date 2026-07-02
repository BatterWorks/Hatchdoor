# Clipboard / image upload / file download

4 confirmed findings (1 high, 2 medium, 1 low), 0 refuted.

## Confirmed findings

### HIGH: Image normalizer re-encodes through canvas without applying EXIF orientation

- **Affected clients:** iOS Safari, iOS PWA, desktop Safari
- **Location:** frontend/src/imageUpload.ts:38-52
- **What happens:** normalizeImageForUpload calls `window.createImageBitmap(file)` with no options and then drawImage()s the bitmap onto a canvas which is re-encoded to WebP. No `{ imageOrientation: "from-image" }` is passed, and the canvas re-encode discards the source EXIF orientation tag entirely.
- **Why:** Photos taken on phones are stored with an EXIF orientation flag rather than rotated pixels. Whether createImageBitmap auto-applies that flag depends on the engine default, which differs across the target matrix: Chrome and Firefox apply EXIF orientation by default, but WebKit has shipped versions that do not auto-orient when no imageOrientation option is given. Because the canvas re-encode strips EXIF, any portrait camera photo uploaded from iOS Safari / the installed PWA (the primary mobile capture path) can be permanently baked sideways into the stored attachment, with no EXIF tag left for downstream viewers to correct. This is exactly the case that matters most on the mobile targets.
- **Fix sketch:** Pass `createImageBitmap(file, { imageOrientation: "from-image" })` so all engines normalize orientation into pixels before the canvas re-encode; keep the existing try/catch fallback for engines that reject the option.

### MEDIUM: copyNoteLink uses navigator.clipboard directly with no fallback and swallows all errors silently

- **Affected clients:** Desktop Chrome, Edge, Firefox, desktop Safari, iOS Safari, iOS PWA, Android Chrome
- **Location:** frontend/src/App.tsx:510-519
- **What happens:** copyNoteLink calls `await navigator.clipboard.writeText(window.location.href)` inside a try/catch whose catch body is empty. Unlike copyText() in clipboard.ts, there is no execCommand textarea fallback and no user-facing feedback on failure.
- **Why:** navigator.clipboard is gated behind a secure context. On any non-HTTPS access (LAN IP over HTTP, which is a common self-hosted Hatchdoor scenario per the deploy notes, plus localhost-but-by-IP and dev) `navigator.clipboard` is undefined in every engine, so the call throws and is silently discarded — the Copy-link button does nothing with zero indication. Even on HTTPS, Firefox can reject writeText when the document is not focused, and Safari requires the write to occur in the activation; the empty catch hides all of these. This is inconsistent with copyPageContent (line 524), which routes through the robust copyText() helper.
- **Fix sketch:** Route copyNoteLink through the same copyText() helper (it already has the execCommand fallback) and surface a toast/notice on the boolean result instead of swallowing failures.

### MEDIUM: Paste-to-upload only reads clipboardData.files, missing images that WebKit exposes only via items/getAsFile

- **Affected clients:** desktop Safari
- **Location:** frontend/src/components/NoteEditor.tsx:194-207
- **What happens:** handlePaste resolves the pasted image via `firstImageFile(event.clipboardData.files)` (line 201). It never falls back to iterating `event.clipboardData.items` and calling getAsFile().
- **Why:** In WebKit, an image copied from another web page (right-click Copy Image) is frequently surfaced only through clipboardData.items as a 'file' entry, with clipboardData.files left empty — this is the documented Safari divergence from Chromium, where .files is reliably populated for pasted images. On desktop Safari the paste then matches no file and is silently ignored (no preventDefault, no upload), so users paste an image and nothing happens. The drop path (line 213, dataTransfer.files) is unaffected.
- **Fix sketch:** When clipboardData.files yields nothing, scan clipboardData.items for entries with kind === 'file' and type starting with 'image/', calling getAsFile() to recover the blob before bailing out.

### LOW: execCommand copy fallback uses a readonly textarea pattern that is unreliable on iOS WebKit

- **Affected clients:** iOS Safari, iOS PWA
- **Location:** frontend/src/clipboard.ts:11-23
- **What happens:** The non-async fallback in copyText creates a textarea, sets the readonly attribute (line 13), then calls select()/setSelectionRange() and document.execCommand('copy') (lines 19-21).
- **Why:** On iOS WebKit, programmatic text selection for execCommand('copy') does not work on a plain readonly textarea via select()/setSelectionRange(); the long-standing iOS workaround requires making the element contentEditable and selecting an explicit Range, otherwise the selection is empty and execCommand returns false. So whenever this fallback is reached on iOS — i.e. when navigator.clipboard is unavailable, such as an insecure-context/LAN deployment — copy silently fails. Impact is limited because on HTTPS the primary navigator.clipboard path is used, so this only bites insecure-context iOS users.
- **Fix sketch:** For the fallback, use a contentEditable element plus a Selection/Range (the standard iOS-safe pattern) or feature-detect and message the user that copy requires a secure context.

## Refuted (not real / already handled)

None.
