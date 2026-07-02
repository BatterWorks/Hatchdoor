# Network / fetch / auth-token / CORS / error-shape contract with the Rust API

4 confirmed (3 medium, 1 low unverified), 0 refuted.

## Confirmed findings

### MEDIUM: Attachment upload silently capped at axum's 2 MB default body limit, contradicting the advertised 10 MB max

- **Affected clients:** iOS Safari, iOS PWA, Android Chrome, Desktop Chrome, Edge, Firefox, desktop Safari
- **Location:** src/main.rs:43
- **What happens:** build_router() (src/main.rs:43-108) never applies DefaultBodyLimit::disable() or a raised limit, so axum's built-in 2 MB (2_097_152 byte) request-body cap governs POST /api/attachment. Meanwhile the server config advertises max_attachment_bytes = 10 MB by default (src/mcp/config.rs:48, checked inside import_attachment_bytes at src/handlers/write_api.rs:186). The frontend normalizeImageForUpload (frontend/src/imageUpload.ts:74-80) deliberately does NOT convert/downscale image/gif or image/svg+xml, and returns the ORIGINAL file whenever createImageBitmap throws (imageUpload.ts:67-68) — which WebKit does for HEIC and some formats on iOS. uploadAttachment (frontend/src/writeApi.ts:94) and handleUploadAttachment (frontend/src/components/NotePage.tsx:531) impose no client-side size check. So a >2 MB animated GIF, large SVG, or an un-decodable image on iOS is sent raw and rejected by axum's body limit before the handler runs.
- **Why:** When the multipart body exceeds 2 MB, axum's RequestBodyLimit aborts the stream, so field.bytes() at src/handlers/write_api.rs:145 fails and the handler returns 400 'invalid file field: ...' (or 'invalid multipart upload'). The user pasting/dropping a normal-sized GIF sees a cryptic 400 even though the app claims a 10 MB ceiling; the two limits are enforced by different layers and disagree. iOS is hit hardest because HEIC photos fall through the createImageBitmap catch and stay large.
- **Fix sketch:** Add .layer(DefaultBodyLimit::max(state.mcp_config.max_attachment_bytes as usize + slack)) to the attachment route (or the protected router), and add a client-side size guard in handleUploadAttachment that reads max_bytes from get_attachment_import_config / write-capabilities and shows a friendly 'file too large' message before uploading.

### MEDIUM: EventSource for /api/vault-events has no onerror/close handling, causing a silent infinite 401 reconnect loop that never surfaces the TokenPrompt

- **Affected clients:** Desktop Chrome, Edge, Firefox, desktop Safari, iOS Safari, iOS PWA, Android Chrome
- **Location:** frontend/src/App.tsx:212
- **What happens:** new EventSource(withAccessToken('/api/vault-events')) (frontend/src/App.tsx:212) attaches only a 'vault-revision' message listener (line 227) and never registers an 'error' handler or inspects readyState. The 401 auth path is wired exclusively through apiFetch's status check (frontend/src/api.ts:61-63), which EventSource does not use. When a protected deployment has no stored token (withAccessToken returns the URL unchanged, api.ts:74-75) or a rotated/invalid token, the SSE endpoint — which sits behind require_web_token (src/main.rs:48, src/auth.rs:31) — returns 401. The browser's native EventSource then auto-reconnects roughly every 3 s indefinitely, each attempt a fresh 401, and unauthorizedHandler is never invoked from this path.
- **Why:** All target browsers auto-reconnect a broken EventSource by spec, and there is no code to stop it, so a background 401 storm runs until the user happens to trigger an apiFetch 401 elsewhere (tree/note load) that raises the TokenPrompt, or until the page is reloaded. On a deployment where SSE is the only early request that fails (e.g., token rotated mid-session while the tree is already cached), the live-refresh channel silently dies with no user-visible signal and keeps hammering the server.
- **Fix sketch:** Add events.onerror that checks events.readyState === EventSource.CLOSED and calls the same unauthorized handler / setAuthRequired(true); or close and back off after repeated failures instead of relying on native infinite reconnect.

### MEDIUM: No fetch timeout or AbortController anywhere — stalled requests leave permanent spinners on iOS Safari/PWA after network transitions

- **Affected clients:** iOS Safari, iOS PWA, desktop Safari
- **Location:** frontend/src/api.ts:60
- **What happens:** apiFetch (frontend/src/api.ts:45-65) calls fetch(input, finalInit) with no signal, and there is zero use of AbortController/AbortSignal/timeout anywhere in frontend/src (verified by grep). Callers set loading state and await unconditionally: loadTree sets loadingTree then awaits with no timeout (frontend/src/App.tsx:200-203, 125), loadNote sets loading then awaits (frontend/src/components/NotePage.tsx:152-159, 115), refreshVault awaits POST /api/refresh (App.tsx:501-509). If the underlying fetch never settles, these promises never resolve or reject.
- **Why:** WebKit on iOS (Safari and installed PWA) is known to leave in-flight fetch() promises hanging indefinitely when the network changes underneath them — resuming the PWA from background, Wi-Fi to cellular handoff, or a captive-portal stall. Because nothing aborts the request, setLoading(false) never runs and the app shows a spinner forever with no error and no retry path except a manual reload, which a home-screen PWA makes awkward. Chrome/Firefox time these out at the OS layer more aggressively, so the impact concentrates on WebKit.
- **Fix sketch:** Wrap apiFetch in an AbortController with a sane timeout (e.g., AbortSignal.timeout(15000)) merged with any caller signal, and treat abort as a retryable/offline error so the loading UI resolves to a 'tap to retry' state.

### LOW: Non-JSON error bodies plus empty HTTP/2 statusText produce bare numeric error messages (unverified)

- **Affected clients:** Desktop Chrome, Edge, Firefox, desktop Safari, iOS Safari, iOS PWA, Android Chrome
- **Location:** frontend/src/writeApi.ts:53
- **What happens:** parseError (frontend/src/writeApi.ts:44-54) assumes the server ErrorResponse shape { error: string } (which the app's own handlers honor via src/app_state.rs / src/auth.rs), but falls back to `${res.status} ${res.statusText}`.trim() when the body is not JSON. Errors emitted by an upstream reverse proxy (nginx/Cloudflare 502/504, or a 413 body-limit rejection surfaced as HTML/plain text) are not JSON, so this fallback runs. Over HTTP/2 — the norm behind any TLS-terminating proxy — Response.statusText is the empty string in all browsers because HTTP/2 dropped the reason phrase, so the trimmed message collapses to just the number (e.g. '502').
- **Why:** Write failures caused by infrastructure (proxy timeouts, oversized uploads rejected by a proxy) surface to the user as a bare '502' or '413' with no explanatory text in the note-action dialog (setNoteActionError paths in NotePage/App), degrading the error UX precisely in the flaky-network scenarios where clear messaging matters most.
- **Fix sketch:** Map common status codes to human-readable strings in parseError rather than relying on statusText, e.g. return a lookup for 413/500/502/503/504 when the JSON parse fails.
