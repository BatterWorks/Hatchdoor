# MCP Wire-Level Error Semantics

Hatchdoor does not rewrite, aggregate, or restate the errors that the MCP transport layer produces. The status codes, the JSON-RPC error codes, and the wording of transport-level failures are rmcp's, and requests to improve them are filed against [`modelcontextprotocol/rust-sdk`](https://github.com/modelcontextprotocol/rust-sdk) rather than implemented here.

This covers the errors raised before a request reaches a Hatchdoor tool: session lookup and session-less requests, protocol version negotiation, the `2026-07-28` per-request header contract (`MCP-Protocol-Version`, `Mcp-Method`, `Mcp-Name`), and the `params._meta` requirement checks. It does not cover the errors Hatchdoor's own security gate raises before rmcp sees the request (bearer token, Origin allowlist, retired protocol revision, body size, rate limits), and it does not cover tool-level errors, which are entirely ours.

## Why this is out of scope

ADR-17 made rmcp the MCP protocol boundary and named request `_meta` and header validation among the things it owns. The point of that decision was to stop growing bespoke wire code that every future protocol revision would have to re-earn inside Hatchdoor. Transport error semantics are wire code. Improving them upstream means the improvement arrives on the next pinned-version bump, applies to every revision rmcp serves, and costs us nothing in boundary erosion or golden-test churn.

There is also a concrete hazard in doing it locally. Hatchdoor's `/mcp` middleware runs ahead of rmcp and already buffers the POST body, so it is technically able to answer a request before rmcp routes it. Using that to improve a transport error means the middleware must first reproduce rmcp's own routing decision, and that decision is not a simple one. Whether a session-less POST is a legacy session-routed request or a modern stateless one is derived from the headers, the body shape, and `legacy_session_mode` together, inside rmcp. A middleware that reads it wrong refuses valid modern traffic. That is an availability risk taken on in exchange for a better error string, and it would have to be re-verified on every rmcp upgrade.

The third reason is that we are frequently not sure the local "improvement" is correct. The motivating example was the difference between these two answers:

```
POST /mcp with a session id the server never issued
  404 Not Found: Session not found

POST /mcp with no session id at all
  422 Unexpected message, expect initialize request
```

The 404 reads as more actionable, and the obvious suggestion is to answer both the same way. But the two are not the same condition. The 422 is rmcp applying a real specification rule, that a POST carrying no session id is expected to be `initialize`, and the specification's guidance for a missing session id is 400 rather than 404. Collapsing them would conflate "your session is gone" with "you never offered one", on a supported revision, on our own authority. If that collapse is right, it is right for every rmcp user and belongs upstream.

## What this costs

A well-behaved client pays nothing. It sends the full header and `_meta` contract or none of it, and it holds a session id it received from `initialize`. The cases these requests are about are a client that has already misbehaved, and a human assembling a request by hand to test conformance. Both are real, and neither is a runtime concern.

The known rough edges, recorded so nobody has to re-derive them:

- A session-less POST is answered with `422 Unexpected message, expect initialize request`, which describes what the server expected to receive rather than what the caller should do. Re-initializing is the remedy.
- The `2026-07-28` request requirements are reported one at a time, and `params._meta` is checked before the headers, so a request missing everything needs three failed round trips (the `_meta` pair together, then `Mcp-Method`, then `Mcp-Name`) before it succeeds. The `_meta` check names both of its own missing fields at once; the header checks return on the first failure.

Both are reproducible against the `/mcp` router as it stands. The strings come from rmcp 3.1.4: `unexpected_message_response` in `transport/common/server_side_http.rs`, the session lookup in `transport/streamable_http_server/tower.rs`, `validate_request_protocol_version_meta` in the same file, and `validate_request_headers` in `transport/common/mcp_headers.rs`.

## Reconsidering this

Two things would reopen it. If a supported client turns out to genuinely fail to recover from one of these answers, and the evidence points at the server's reply rather than the client, then it is a defect worth fixing wherever it has to be fixed. And if rmcp declines the upstream request, the choice becomes carrying a patch against the pin or living with the wording, which is a decision for ADR-17 to record rather than for this file to pre-empt.

## Prior requests

- #213 — "MCP error responses do not carry enough to recover in one step"
