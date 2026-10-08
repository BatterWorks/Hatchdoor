---
tags: [type/explanation, topic/security]
---

# The security model

The browser uses the web token, agents use the MCP password, and each Git-backed Vault may hold a token Hatchdoor uses to reach its remote. Give an agent only the MCP password, keep agent writes off until you need them, and never put Hatchdoor directly on the internet.

The rest of this page is the detail. Hatchdoor has three separate secrets. Each protects a different boundary, and none stands in for another.

## The three secrets

| Secret | Protects | Configured as |
| --- | --- | --- |
| **Web bearer token** | The browser and the HTTP API: `/api/v1/vaults/...`, Settings, model setup | `HATCHDOOR_WEB_BEARER_TOKEN` |
| **MCP bearer token** | Agent access via `/mcp`, further split into read and write | `HATCHDOOR_MCP_BEARER_TOKEN` |
| **Git HTTPS credentials** | Hatchdoor's own outbound connection to a Git remote | Per-Vault, entered in Settings or via `create_vault`/`edit_vault` |

The first two decide who can reach Hatchdoor and what they can do. The third lets Hatchdoor prove who it is to a Git server. A leaked Git token lets someone push to your repository as Hatchdoor, and gives them no way into Hatchdoor.

## Web bearer token

The web token guards the browser and the HTTP API: Settings, model setup, and every `/api/v1/vaults/...` route when a token is configured. Hatchdoor reads it from `Authorization: Bearer <token>`, or from an `access_token` query parameter where a header cannot be set, as in an `<img>` tag that points at an attachment. It compares tokens in constant time, so the time an answer takes reveals nothing about how much of a guessed token was right.

> [!warning]
> With a `HOST` that is not a loopback address and no web token, Hatchdoor refuses to start. It generates a token, prints it, and asks you to add it to `.env`. You cannot skip this check. [[Install Hatchdoor with Docker Compose]] shows what it looks like.

## MCP bearer token

MCP has its own secret, checked independently of the web token. It is stricter than the web token in two ways:

- **MCP requires a token even to enable it at all.** With `HATCHDOOR_MCP_ENABLED` on and no `HATCHDOOR_MCP_BEARER_TOKEN`, Hatchdoor refuses to start. The web token can be left out on a loopback address. The MCP token never can.
- **Read access needs the token too.** Read-only MCP still opens every Vault, through `search_notes`, `get_note`, `get_tree` and the rest, and it does not pass through the web token check. So the MCP token is required with write mode on or off.

Two switches apply on top of the token, each on its own:

- **`HATCHDOOR_MCP_ENABLED`**: off by default. While it is off, `/mcp` returns `404`, as if the endpoint did not exist.
- **`HATCHDOOR_MCP_WRITE_ENABLED`**: off by default, even when MCP is on. It separates an agent that can only read from one that can also create, edit, move or delete notes. [[MCP tools reference]] lists the tools each switch allows.

MCP also checks the request's `Origin` header against an allow-list (`HATCHDOOR_MCP_ALLOWED_ORIGINS`) as a defense against DNS-rebinding attacks, and validates the `MCP-Protocol-Version` header. Neither is a secret. Both run before the token check on every request.

### Where the MCP token works outside `/mcp`

Two HTTP routes accept the MCP bearer token as well as the web one, so an agent that holds only the MCP token can upload and download attachments without the web token. Both read the current settings on every request, so turning MCP off shuts them at once, with no restart:

| Route | Accepts the MCP token | Why that gate |
| --- | --- | --- |
| `POST /api/v1/vaults/{vault_id}/attachments` (upload) | while MCP **and** MCP writes are enabled | It writes. Turning write mode off has to close it, or the setting would mean nothing here. |
| `GET /api/v1/vaults/{vault_id}/assets/{*path}` (download) | while MCP is enabled | It reads, so write mode does not matter. The MCP token gets less here than the web token does, as described below. |

The MCP token is header-only on both. Only the web token may ride in an `access_token` query parameter, because only the browser needs it to.

These checks exist only on a deployment with a web token. With none, both routes are open to anyone who can reach the server, like the rest of the web API, and setting an MCP token does not change that. The MCP token only adds a way in. It never makes Hatchdoor ask the browser for a token it does not have.

On the download route the MCP token is held to the **same limits it has over `/mcp`**, so the URL is no way around them:

- **Size.** It can read up to `HATCHDOOR_MCP_MAX_BASE64_BYTES` (5 MiB by default), the same ceiling `get_attachment`'s base64 encoding enforces, and not the route's own, much larger limit. An attachment over that returns `413`. Lower the setting and both paths follow.
- **Rate.** The request spends the same per-token quota and concurrency budget an MCP tool call spends, drawn from the same counter, and gets `429` with `Retry-After` when it runs out. Fetching through the URL does not give a second allowance.

A request with the **web** token is held to neither limit. It comes from the browser, which already has the whole Vault.

> [!note]
> On a deployment with no web token, the download route stays open as before. Turning MCP on never makes it ask for a token the browser does not have.

### Transfer links

Those two routes help a client that holds the MCP token. An agent running inside an MCP client usually does not: the client keeps the token and the server's address in its own configuration and never shows them to the model. Giving the agent the token is the wrong fix: the token opens every Vault for writing, and it would end up in shell commands and transcripts.

So an authenticated MCP call can mint a **transfer link** instead. `get_attachment` returns one for downloading a file, and `create_upload_link` returns one for uploading a file to one named path. A link is a full address that carries its own credential, and it allows far less than the token that made it:

- **One file, one direction.** A download link reads one attachment in one Vault. An upload link writes one file to one path, whether or not it may replace an existing file fixed when it was minted, once. A link that replaces a note also carries the note's content hash, signed in when it was minted, and writes only if the note still has it, so a link can never overwrite an edit made since the agent last read the note.
- **Five minutes.** Every link expires five minutes after it is made. The time is checked when a transfer starts, so a slow upload that started in time finishes.
- **Revocable without a list.** Links are signed with a random key that lives only in memory and changes whenever the MCP password changes. So a restart or a password change ends every link already handed out. Each request also re-reads the live settings: MCP off refuses every link, and **Let assistants change notes** off refuses every upload link.
- **No bigger allowance.** A download through a link spends the same rate quota and is held to the same `HATCHDOOR_MCP_MAX_BASE64_BYTES` ceiling as the MCP token on the download route above. An upload keeps `HATCHDOOR_MAX_ATTACHMENT_BYTES`. A link reaches only what `get_attachment` itself would return.
- **Kept out of logs.** The link's signature is redacted from the server's request log like the web token.

A leaked link exposes one file, or one upload slot, for at most five minutes. A leaked MCP token exposes every Vault until you change it.

Links are built on **Public address** (`HATCHDOOR_PUBLIC_URL`) when it is set. Otherwise they use the address the agent reached the server on: the scheme and host a proxy reports in `Forwarded`, else in `X-Forwarded-Proto` and `X-Forwarded-Host`, else `http` and the request's own `Host`. Hatchdoor trusts these headers from any sender, because the link goes back only to the caller who sent them, and reads them for nothing else. A value that is not `http`/`https` or not a plain host is ignored. Set the public address when your proxy sends none of these headers or serves Hatchdoor under a path.

> [!note]
> The web token and the MCP token have nothing to do with each other, on purpose. A leaked MCP token gives no access to Settings or the Web UI, and changing one token never forces you to change the other. Give an agent the MCP token and never the web token. An agent has no need for Settings.

## Git HTTPS credentials

Each Vault's remote can have an HTTPS token that Hatchdoor signs in with when it fetches or pushes. The token is write-only: once saved, no read returns it, and Hatchdoor's own debug output hides it. An edit to a Vault says `keep`, `remove` or `replace` for the credential, so a stored token never has to be sent back to you and resent to survive the edit. [[HTTP API reference]] has the request shape.

This token signs Hatchdoor in to the remote. It signs nobody in to Hatchdoor, and plays no part in the web token and MCP token checks above. Whoever holds it can do what the remote allows that token, and nothing in Hatchdoor.

## What's open regardless of any token

`/health`, `/ready`, and `/api/startup-status` are never gated by any token. They are liveness and readiness probes for tools that hold no token, such as a container orchestrator or a load balancer. They report whether the process is up and how indexing stands, never Vault content.

The manual is open too. Every instance serves this manual as plain Markdown at `/docs/<page>.md`, with an index at `/docs/index.md`, a word search at `/docs/search`, and an `llms.txt` list of pages, so an agent can read it with an ordinary web fetch. These addresses serve only the manual built into the running version. They never serve Vault content, Vault names or paths, settings or tokens, and you cannot turn them off. A page of the manual can be marked private: such a page answers only with the web token, and never shows up in `llms.txt`, the index or search for anyone without it. With no web token configured, these addresses never serve a private page at all. This is how a page that names the running version stays off the open addresses.

## The update check

With **Tell me about new releases** on, Hatchdoor sends one request a day to GitHub's public Hatchdoor release list, carrying your IP address and the user-agent `Hatchdoor`, nothing else; it is off by default (see [[How to upgrade Hatchdoor#Hear about new releases]]).

## The usage report

The usage report is telemetry, and it is off unless you turn it on. With **Send a usage report** on, Hatchdoor sends one small report a day to `https://telemetry-hatchdoor.battercloud.cc`, a collector run by Hatchdoor's maintainer. The report says what the install runs on and which parts of Hatchdoor it uses, as fixed words, yes or no answers and ranges, with a random install ID. It carries nothing about your notes, your Vaults' names, your machine or any token. [[Usage report reference]] lists every field, and Settings shows the exact report before you decide.

The first request goes within a minute of turning the report on. After that Hatchdoor makes at most one successful request in any 24 hours, and tries again an hour after one that failed. Each is a single `POST` that carries your IP address, the user-agent `Hatchdoor` and the report, and Hatchdoor uses nothing from the answer beyond whether the report got through. While the report is off, and in a public demo, no request is made at all.

## Demo mode's narrower rule

A public, read-only demo (`HATCHDOOR_DEMO_MODE=true`) changes these rules, and not in the same way everywhere:

- Settings and model setup do not exist: their routes answer `404`.
- Vault reads are public and need no token, which is the point of a demo.
- Vault write and Vault control routes exist and answer `403 demo_read_only`.
- The page the browser loads carries a link preview, so a demo link shared in a chat app shows a title, a short description and a picture. A note's link previews that note's title and opening words, and only for a note the demo already serves to anyone: a missing note, a disabled Vault or a note on a demoted layer previews as Hatchdoor itself. The picture and the page's own address appear only when **Public address** (`HATCHDOOR_PUBLIC_URL`) is set, because Hatchdoor never builds them from the `Host` or forwarded headers a request carries.

Outside demo mode none of this applies. The page is sent before any token is checked, so it stays the same file for every address and names no note.

Demo mode and MCP cannot run together: Hatchdoor refuses to start with both `HATCHDOOR_DEMO_MODE=true` and `HATCHDOOR_MCP_ENABLED=true`. A demo has no operator to hold an MCP token.

---

Related: [[Connect your agent]] · [[HTTP API reference]] · [[MCP tools reference]]
