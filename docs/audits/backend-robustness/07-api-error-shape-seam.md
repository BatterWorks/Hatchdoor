# API error-shape contract seam with the frontend

**Summary:** 3 confirmed (1 high, 1 medium, 1 low), 0 refuted.

## Confirmed findings

### HIGH: Attachment uploads over 2MB fail with a misleading 400 error, contradicting the advertised 10MB limit (no DefaultBodyLimit configured)

- **Trigger conditions**
  - Any `/api/attachment` upload whose multipart body exceeds axum's default 2MB limit (a normalized webp screenshot at 2560px edge, a GIF/SVG/PDF, or any file the client sends unchanged)
  - The handler's real `HATCHDOOR_MCP_MAX_ATTACHMENT_BYTES=10MB` check at `src/handlers/write_api.rs:186` is never reached for 2–10MB files

- **Location** `src/main.rs:43-108`

- **What happens**
  Build_router installs no axum DefaultBodyLimit override (grep of src/ finds none), so axum's built-in 2MB request-body cap governs the Multipart extractor in upload_attachment_handler. When a body exceeds 2MB, `field.bytes().await` (`src/handlers/write_api.rs:145`) returns a length-limit error and the handler responds 400 with JSON `{error: "invalid file field: <length limit exceeded>"}` (`write_api.rs:148-155`). Meanwhile the server's own limit is 10MB (`src/mcp/config.rs:82`) and is advertised to clients as max_bytes via get_attachment_import_config (`src/mcp/tools.rs:52`). The frontend surfaces the raw backend message through parseError/uploadAttachment (`frontend/src/writeApi.ts:44-54,101-108`), so the user sees "invalid file field: length limit exceeded" at 2MB even though the nominal limit is 10MB.

- **Why**
  Real, user-facing break of a core write feature (image paste / attachment upload) on a public launch: the effective limit (2MB) is 5× smaller than what the backend enforces and advertises, and the error text points the user at a nonexistent 'invalid file field' problem instead of a size problem. The 10MB validation path and its clean error message are dead code for the HTTP surface.

- **Fix sketch**
  Add `.layer(DefaultBodyLimit::max(max_attachment_bytes + slack))` to the attachment route (or protected router) so the Multipart extractor allows bodies up to the configured cap and the handler's own size check + clean error message actually run; keep JSON routes at a small limit.

### MEDIUM: Read-path handlers discard the server's structured {error} JSON body and surface only the numeric HTTP status

- **Trigger conditions**
  - `GET /api/note/{slug}` returning 404 with `{error: "Note not found: <slug>"}` (`src/handlers/api.rs:262-270`) rendered as "Failed loading note: 404"
  - `GET /api/tree`, `/api/search`, `/api/stats`, `/api/graph`, `/api/recently-modified` on any 500/503 where internal_error's real cause message is dropped

- **Location** `frontend/src/components/NotePage.tsx:116-118`

- **What happens**
  Every read fetch throws `new Error(`Failed loading X: ${res.status}`)` without parsing the body: NotePage.tsx:117 (note), :136 (links), App.tsx:127 (tree), :145 (recently-modified), :430 (search), GraphPage/StatsPage similarly. The Rust handlers uniformly return ErrorResponse `{error: string}` (`api_types.rs:93-96`) with a descriptive message (`api.rs:262` note-not-found, app_state internal_error for 500s). The read layer never parses it (unlike writeApi.ts parseError), so server diagnostics are replaced by a bare status code.

- **Why**
  Not data-loss, but hides the actual backend failure reason from users/support on a public launch and is inconsistent with the write path. A 500 whose `{error}` explains a cache/index failure is indistinguishable from any other 500 in the UI.

- **Fix sketch**
  Add a shared read helper mirroring writeApi.ts parseError: on `!res.ok`, try `res.json()` → `json.error` string, fall back to `${status} ${statusText}`, throw that; reuse in NotePage/App/GraphPage/StatsPage.

### LOW: JSON write-body rejections are coerced to 400 regardless of the rejection's real status (e.g. body-too-large → 413 instead of 400)

- **Trigger conditions**
  - Any JSON write body (`POST /api/note`, `PUT/PATCH/DELETE /api/note/{slug}`) exceeding axum's 2MB JSON limit: JsonRejection carries 413 but write_payload emits 400
  - Clients/proxies/monitoring that key off status codes rather than the `{error}` string

- **Location** `src/handlers/write_api.rs:616-628`

- **What happens**
  write_payload maps every JsonRejection to a hardcoded `StatusCode::BAD_REQUEST` while taking the message from `rejection.body_text()`. Axum's JsonRejection variants have distinct statuses (415 content-type, 413 length-limit, 400/422 parse) all flattened to 400. Body shape stays correct JSON `{error}` so the frontend parseError and 409-vs-other logic (NotePage.tsx:461) are unaffected, but the status-code contract is lossy.

- **Why**
  Low launch impact: the frontend only special-cases 409 and otherwise shows the message, so it degrades gracefully. A contract inaccuracy worth noting for status-code-based clients.

- **Fix sketch**
  Return `rejection.status()` instead of hardcoded `StatusCode::BAD_REQUEST` in write_payload, preserving the JSON `{error: rejection.body_text()}` body.

## Refuted (not real / already handled)

(No refuted findings in this audit.)
