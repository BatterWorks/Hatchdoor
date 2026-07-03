# MCP protocol & tool-surface robustness

**Summary:** 3 confirmed (1 high, 2 low), 0 refuted.

## Confirmed findings

### HIGH: MCP read-only mode has no authentication and /mcp bypasses the web-auth layer, exposing the entire vault

**Trigger conditions:**
- HATCHDOOR_MCP_ENABLED=true with write mode off and no HATCHDOOR_MCP_BEARER_TOKEN (the documented read-only configuration)
- Operator sets HATCHDOOR_WEB_BEARER_TOKEN to lock down the HTTP API but leaves MCP read-only
- HOST=0.0.0.0 / container port exposed beyond localhost

**Location:** `src/main.rs:86`

**What happens:**
The `/mcp` route is registered on the open router (main.rs:86) BEFORE `.merge(protected)` (main.rs:87), so the web bearer-token middleware applied to the protected sub-router (main.rs:76-82) never runs for MCP. MCP's own gate, `validate_mcp_request` (src/mcp/config.rs:108-165), only enforces a bearer token when `config.bearer_token` is `Some`, and `McpConfig::validate` (src/mcp/config.rs:97-105) only requires a token when `write_enabled` is true. In read-only mode the token is therefore optional and typically unset, so any client that can reach the port can call `get_tree`, `get_note`, `search_notes`, `get_note_links`, and `resolve_wikilink` and read the full contents of a private vault with no credential. The Origin allowlist (config.rs:122-131) only blocks browsers that send a disallowed Origin header; a non-browser client (curl, script) sends no Origin and passes.

**Why:**
The vault is a private personal knowledge base and is the confidentiality boundary. An operator who enables MCP read access and locks down the web API with HATCHDOOR_WEB_BEARER_TOKEN reasonably assumes the whole surface is authenticated, but /mcp silently is not — it is a second, unauthenticated read path to the same data. For a public launch on an exposed port this is full unauthenticated vault disclosure.

**Fix sketch:**
Either mount /mcp behind the same web-auth layer, or require an MCP bearer token whenever MCP is enabled (make McpConfig::validate reject enabled && bearer_token.is_none() regardless of write mode), or at minimum reject requests that carry no credential when the listener is non-loopback.

---

### LOW: initialize ignores the client's requested protocolVersion, and later requests require an exact version-header match, which can lock out otherwise-compatible clients

**Trigger conditions:**
- MCP client that negotiates or was built against a different protocol draft (e.g. sends MCP-Protocol-Version: 2025-06-18 on follow-up requests)

**Location:** `src/mcp/routes.rs:104`

**What happens:**
`handle_initialize` (routes.rs:104-118) always returns the hard-coded `PROTOCOL_VERSION` ('2025-11-25', config.rs:10) and never inspects the `protocolVersion` the client sent in the initialize params. Then `validate_mcp_request` (config.rs:150-162) rejects any subsequent request whose `MCP-Protocol-Version` header is not byte-for-byte '2025-11-25' with error -32002. A client that negotiated or defaults to a nearby supported draft will be told '2025-11-25' at initialize but then have every follow-up call hard-rejected, with no content-negotiation fallback.

**Why:**
This is an interop/robustness gap in the protocol handshake: the server dictates a single version and refuses everything else instead of negotiating, so version-skewed but otherwise-valid clients fail after a seemingly successful initialize.

**Fix sketch:**
Echo/negotiate the client's requested protocolVersion in the initialize response when supported, and accept a small set of known-compatible version-header values (or omit strict header equality) rather than a single exact string.

---

### LOW: Inconsistent error surface for 'note not found' between read and write tools

**Trigger conditions:**
- MCP client calling get_note/get_note_links vs update_note/edit_note/etc. against a slug that does not exist

**Location:** `src/mcp/tools.rs:716`

**What happens:**
A missing note is reported two different ways depending on the tool. For reads, `get_note_tool` (tools.rs:291) and `get_note_links_tool` (tools.rs:306) return a tool result with `isError:true` via `tool_error`. For writes, `note_entry` (tools.rs:708-717) maps the same condition to `JsonRpcFailure::invalid_params` (JSON-RPC protocol error -32602). So the identical runtime condition ('Note not found: {slug}') surfaces once as a tool-level error inside a successful result and once as a transport-level JSON-RPC error object.

**Why:**
MCP clients treat `isError` tool results and JSON-RPC error objects differently (the former is fed back to the model for retry, the latter is a protocol failure). Reporting the same not-found state through two channels makes client handling and the model's recovery behavior inconsistent across tools.

**Fix sketch:**
Pick one representation for 'not found' (most naturally a tool_error/isError result for both read and write tools) and route note_entry's not-found case through it instead of invalid_params.

---

## Refuted (not real / already handled)

(No findings were refuted.)
