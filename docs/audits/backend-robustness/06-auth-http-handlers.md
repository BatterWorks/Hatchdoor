# Auth & HTTP Handler Robustness

4 confirmed (3 medium, 1 low), 0 refuted.

## Confirmed findings

### MEDIUM: /mcp is exempt from the web auth layer; enabling HATCHDOOR_WEB_BEARER_TOKEN does not protect MCP read tools

- **Trigger conditions:**
  - HATCHDOOR_MCP_ENABLED=true with no HATCHDOOR_MCP_BEARER_TOKEN and write disabled (read-only)
  - operator has set HATCHDOOR_WEB_BEARER_TOKEN believing the whole vault is locked down
  - non-browser client (curl) sending no Origin header, bypassing the origin allowlist

- **Location:** src/main.rs:86

- **What happens:** POST /mcp tools/call can invoke search_notes / get_note / get_tree and read the entire vault with no credentials, even though HATCHDOOR_WEB_BEARER_TOKEN is set.

- **Why:** The two token systems are independent and the coupling gap is non-obvious. An operator who locks down the web UI for a public launch and turns on MCP for their own assistant, without also setting the MCP token, silently exposes full vault contents. MCP is off by default, which bounds the blast radius, but it is a real data-exposure path.

- **Fix sketch:** Require an MCP bearer token whenever MCP is enabled (make validate() reject enabled && bearer_token.is_none(), not just write mode), or make /mcp inherit the web token when set; at minimum log a loud startup warning when MCP is enabled tokenless.

### MEDIUM: Attachment uploads are capped at axum's 2 MB default body limit while 10 MB is advertised/enforced downstream

- **Trigger conditions:**
  - POST /api/attachment (upload_attachment_handler) with a file between 2 MB and max_attachment_bytes (default 10 MB)
  - any deployment relying on advertised HATCHDOOR_MCP_MAX_ATTACHMENT_BYTES (default 10*1024*1024)

- **Location:** src/handlers/write_api.rs:186

- **What happens:** A 3-10 MB image is rejected by the framework with a generic 413 before handler code runs, so the 10 MB limit is unreachable over HTTP and the advertised value is misleading.

- **Why:** Users are told 10 MB is allowed but real uploads fail at 2 MB with an opaque framework error (no ErrorResponse JSON body), reading as a broken upload feature. Exactly the advertised-vs-actual body-size mismatch flagged for launch.

- **Fix sketch:** Apply an explicit DefaultBodyLimit sized from max_attachment_bytes (plus multipart overhead) to the attachment route and keep JSON routes at a sane limit; ensure advertised max matches the enforced framework limit.

### MEDIUM: When HATCHDOOR_WEB_BEARER_TOKEN is unset, all mutating routes are fully unauthenticated with only an info-level guardrail

- **Trigger conditions:**
  - HATCHDOOR_WEB_BEARER_TOKEN unset (web_bearer_token=None)
  - HOST=0.0.0.0 (or fronted without an auth proxy) exposing the port beyond localhost

- **Location:** src/main.rs:76

- **What happens:** The entire router—including POST /api/note, PUT/DELETE /api/note/{slug}, the rename/move/archive PATCH routes, and POST /api/attachment—is served unauthenticated. The only guardrail against public exposure is an info-level startup log when host==0.0.0.0 with no token, plus a warning string in the /api/write-capabilities body. Nothing refuses to start or downgrades to read-only.

- **Why:** For a public launch, an insecure-by-default allowing anonymous overwrite/deletion of the vault, guarded only by an easily-missed info log, is a foot-gun. Content-hash checks stop blind overwrites but not create/upload or hash-then-delete sequences.

- **Fix sketch:** Refuse to bind a non-loopback host without a web token (hard error), or auto-disable frontend write routes when no token is configured, instead of only logging at info level.

### LOW: Bearer token accepted as an access_token query parameter, exposing it in URLs and (at debug log level) request spans

- **Trigger conditions:**
  - any <img>/download navigation using ?access_token=<token>
  - tower_http log level raised to debug, where DefaultMakeSpan records the full request URI
  - reverse proxies / browser history / Referer headers that capture full URLs

- **Location:** src/auth.rs:54

- **What happens:** request_is_authorized accepts the web token via the access_token query parameter to support <img>/download navigations that cannot set an Authorization header. The token lands in browser history, proxy access logs, and potentially Referer headers for the affected URLs. At debug log level, the token is recorded in request spans.

- **Why:** Secrets in URLs are a recognized weakness; even with the default log level, the long-lived token is durably exposed in intermediaries the app does not control. Relevant to hardening before a public launch.

- **Fix sketch:** Use a short-lived, scoped signed token (or a cookie derived from the Authorization header) for asset/download URLs instead of the long-lived web bearer token, and redact the query string in the trace span.

## Refuted

No findings were refuted.
