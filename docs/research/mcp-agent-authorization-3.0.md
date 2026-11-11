# How agents prove who they act for over MCP

Resolves research ticket [#717](https://github.com/BatterWorks/Hatchdoor/issues/717), part of wayfinder map [#714](https://github.com/BatterWorks/Hatchdoor/issues/714) (user accounts and Vault permissions, 3.0). Feeds the decision ticket [#719](https://github.com/BatterWorks/Hatchdoor/issues/719), "How a person's agents get their access".

Sources were read on 2026-11-11. Hatchdoor facts come from `development` at the commit this branch was cut from. Every claim carries its source. Anything that could not be confirmed from a first-party source is marked `unchecked`.

## Summary in plain words

Today one shared MCP token opens every Vault. In 3.0 each person needs their own way in for their agents, so Hatchdoor can tell whose agent is calling and show it only that person's Vaults. There are two ways an agent app can carry that proof.

**A key you paste.** The person signs in to Hatchdoor in the browser, makes a key for one agent ("Claude Code on my laptop"), and pastes it into that agent's settings. Hatchdoor recognises the key and knows whose it is. Deleting the key in Hatchdoor cuts that agent off at once. This works today in the agent apps that run on the person's own computer: Claude Code, Codex, Cursor, VS Code, Gemini CLI and Zed. It does not work in ChatGPT, which accepts no pasted key at all. In claude.ai and Claude Desktop it works only where Anthropic shows a "Request headers" field when adding a connector, which its own pages describe differently (see the table).

**A sign-in button ("OAuth").** The person types Hatchdoor's address into the agent app. The app opens a Hatchdoor sign-in page in the browser, the person signs in and clicks "allow", and the app receives a pass that it renews by itself. Nothing is copied by hand. Every agent app checked supports this, including claude.ai, Claude Desktop and ChatGPT. The catch is on Hatchdoor's side: something has to run that sign-in page and hand out the passes (an "authorization server"). Either Hatchdoor does that itself, which is a lot of new security code, or the household runs a separate sign-in service (an identity provider such as Keycloak or Pocket ID) and Hatchdoor trusts it, which is one more service to install and look after.

**One thing no sign-in method fixes.** claude.ai, Claude Desktop and ChatGPT do not call Hatchdoor from the person's computer. They call it from Anthropic's or OpenAI's servers on the internet. A Hatchdoor that is only reachable at home cannot be used from those apps, however agents prove who they are. ChatGPT offers a tunnel program that avoids opening the home network; Anthropic documents nothing equivalent.

The options for #719 are: Hatchdoor runs its own sign-in for agents; Hatchdoor trusts an outside sign-in service; pasted keys only; or keys first and a sign-in button added for the apps that need it. They are compared at the end.

## Client support at a glance

"Static key" means the person can put a fixed `Authorization: Bearer ...` header (or similar) in the app's configuration. "OAuth" means the app runs the MCP authorization flow itself. "Calls from" says where the network requests to Hatchdoor come from.

| Agent app | Static key or header | OAuth sign-in | Client registration it can use | Calls from |
| --- | --- | --- | --- | --- |
| Claude Code | Yes: `--header`, `headers`, or a `headersHelper` script | Yes | Client ID Metadata Document (CIMD), Dynamic Client Registration (DCR), or a pre-registered client ID | The person's machine (a claude.ai connector used inside a Claude Code cloud session goes through Anthropic) |
| claude.ai, Claude Desktop, Claude mobile, Cowork | Partly: a "Request headers" field; Anthropic's developer page calls it a beta for a limited set of organisations, entered once by an organisation Owner; its help centre lists it for Pro, Max, Team and Enterprise | Yes | CIMD, DCR, or a client ID and secret typed into the connector's advanced settings | Anthropic's cloud (`160.79.104.0/21`); the server must be reachable from the public internet |
| ChatGPT (custom MCP plugin) | No: "cannot present custom API keys" | Yes, or no authentication | CIMD, DCR, or "static credentials" (a fixed OAuth client) | OpenAI's servers, or through OpenAI's Secure MCP Tunnel client run inside the home network |
| Codex | Yes: `bearer_token_env_var`, `http_headers`, `env_http_headers` | Yes (`codex mcp login`) | CIMD, DCR, pre-registered client | The person's machine (CLI) |
| Cursor | Yes: `headers`, with `${env:...}` interpolation | Yes | DCR or a static client ID and secret; CIMD not mentioned in its docs | The person's machine |
| VS Code (GitHub Copilot) | Yes: `headers`, with `${input:...}` prompts in the editor format | Yes | CIMD (preferred since 1.106), DCR, or a pre-registered client ID | The person's machine |
| Gemini CLI | Yes: `headers`, or `gemini mcp add -H` | Yes | DCR or a configured client ID; CIMD not mentioned in its docs | The person's machine |
| Zed | Yes: `headers` | Yes, offered when no `Authorization` header is set | Not stated in its docs | The person's machine |

Sources per row are in [Each agent app](#each-agent-app).

## What the MCP specification says

Both revisions Hatchdoor serves (`2025-11-25` and `2026-07-28`) share the same structure. The differences that matter here are listed at the end of this section.

### Authorization is optional; if you do it, it is OAuth

- "Authorization is **OPTIONAL** for MCP implementations." Implementations over HTTP "**SHOULD** conform to this specification" ([2026-07-28 authorization](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization); same wording in [2025-11-25](https://modelcontextprotocol.io/specification/2025-11-25/basic/authorization)).
- Neither revision mentions API keys or static bearer tokens. A `must_contain` check for "API key" on the 2025-11-25 page found nothing. So the specification neither forbids nor describes a server-issued static key: a server that uses one simply is not using the specification's authorization flow, which the specification allows because the flow is optional. Hatchdoor's current shared token is in that position today.
- When a server does implement it: the MCP server is an OAuth 2.1 resource server, the client is an OAuth 2.1 client, and an authorization server issues the tokens. "It may be hosted with the resource server or a separate entity" ([2026-07-28 authorization](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization), Roles). So Hatchdoor may be its own authorization server or point at an outside one; the specification does not prefer either.

### What a server must do when it implements authorization

From the 2026-07-28 pages ([authorization](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization), [authorization server discovery](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/authorization-server-discovery), [security considerations](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/security-considerations)):

- **Protected resource metadata (RFC 9728).** "MCP servers **MUST** implement" it, and the document "**MUST** include the `authorization_servers` field containing at least one authorization server." The server must make it findable either through `resource_metadata` in the `WWW-Authenticate` header of a 401, or at a well-known path (`/.well-known/oauth-protected-resource/mcp` or `/.well-known/oauth-protected-resource`).
- **401 with a challenge.** Invalid or expired tokens "**MUST** receive a HTTP 401 response". Servers "**SHOULD** include a `scope` parameter in the `WWW-Authenticate` header". Insufficient scope gets 403 with `error="insufficient_scope"`. Hatchdoor's 401 today carries no `WWW-Authenticate` header at all (`src/mcp/auth.rs`, `validate_mcp_request`).
- **Audience binding.** Servers "**MUST** validate that access tokens were issued specifically for them as the intended audience, according to RFC 8707", "**MUST** only accept tokens that are valid for use with their own resources" and "**MUST NOT** accept or transit any other tokens."
- **Token in the header only.** Clients use `Authorization: Bearer <access-token>`, "Access tokens **MUST NOT** be included in the URI query string", and authorization "**MUST** be included in every HTTP request from client to server."
- **HTTPS.** "All authorization server endpoints **MUST** be served over HTTPS" and "All redirect URIs **MUST** be either `localhost` or use HTTPS" (security considerations, Communication Security). Hatchdoor itself speaks plain HTTP and relies on a TLS proxy in front of it (ADR-34 context in `docs/adr/README.md`).

### What the authorization server must do

- Implement OAuth 2.1 "for both confidential and public clients", provide RFC 8414 metadata or OpenID Connect Discovery, and advertise `code_challenge_methods_supported`, because clients "**MUST** implement PKCE" with `S256` and "**MUST** verify PKCE support before proceeding" ([security considerations](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/security-considerations)).
- "**SHOULD** issue short-lived access tokens" and, for public clients, "**MUST** rotate refresh tokens" (same page).
- **SHOULD** return the `iss` parameter in authorization responses (RFC 9207), new in 2026-07-28; clients validate it ([2026-07-28 authorization](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization), Authorization Response Validation).
- Validate exact redirect URIs against registered values (security considerations).

### Client registration: how the agent app gets a client ID

Three mechanisms, in the client's priority order: a pre-registered client ID, then a Client ID Metadata Document if the authorization server advertises `client_id_metadata_document_supported`, then Dynamic Client Registration if it exposes a `registration_endpoint`, then asking the user ([2026-07-28 client registration](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/client-registration)).

- **CIMD** (Client ID Metadata Document): the client's ID is an HTTPS URL pointing at a JSON document describing it (name, redirect URIs). The authorization server fetches it. Clients and authorization servers "**SHOULD** support" it in both revisions. The authorization server must guard the fetch against server-side request forgery ([security best practices](https://modelcontextprotocol.io/docs/2026-07-28/tutorials/security/security_best_practices), SSRF section).
- **DCR** (Dynamic Client Registration, RFC 7591): the client posts its details to a registration endpoint and receives a client ID. "**MAY**" in both revisions. In 2026-07-28: "Dynamic Client Registration is deprecated. New implementations should use Client ID Metadata Documents instead." In 2025-11-25 it is a plain MAY without deprecation (a `must_contain` check for "deprecated" on that page found nothing).
- **Pre-registration**: the client is configured with a client ID (and maybe secret) made by hand at the authorization server. Clients "**SHOULD** support an option for static client credentials".

### Differences between the two revisions

| Point | 2025-11-25 | 2026-07-28 |
| --- | --- | --- |
| DCR | MAY | MAY, marked deprecated |
| CIMD | SHOULD | SHOULD |
| `iss` in authorization responses (RFC 9207) | Not required | SHOULD, with client validation rules |
| Sessions | Server-assigned session IDs exist | "MCP is stateless and has no protocol-level sessions"; servers that hold state SHOULD bind each handle to the authenticated user ([security best practices](https://modelcontextprotocol.io/docs/2026-07-28/tutorials/security/security_best_practices)) |

## Each agent app

### Claude Code

- Static headers: `claude mcp add-json ... '{"type":"http","url":...,"headers":{"Authorization":"Bearer token"}}'`, `--header`, or a `headersHelper` command that prints headers at connect time ([Claude Code MCP docs](https://code.claude.com/docs/en/mcp), "Add MCP servers from JSON configuration" and "Use dynamic headers for custom authentication").
- OAuth: `/mcp` or `claude mcp login <name>` opens the browser; tokens "are stored securely and refreshed automatically". It supports DCR, "servers that use a Client ID Metadata Document (CIMD) instead of Dynamic Client Registration", and pre-configured `--client-id` / `--client-secret`. "If you configure an `Authorization` header, Claude Code uses it as the server's credential and doesn't fall back to OAuth for that server" (same page, "Authenticate with remote MCP servers").
- Redirect: an RFC 8252 loopback redirect "on an ephemeral port that varies per session"; it "declares `http://localhost/callback` and `http://127.0.0.1/callback` in its Client ID Metadata Document", so an authorization server must match both with the port ignored ([Claude connector authentication](https://claude.com/docs/connectors/building/authentication), Callback URLs). `--callback-port` fixes the port when an authorization server needs an exact redirect.
- Removing a server deletes its stored tokens; `claude mcp logout` revokes them first where the authorization server supports revocation ([Claude Code MCP docs](https://code.claude.com/docs/en/mcp)).

### claude.ai, Claude Desktop, Claude mobile and Cowork

- Network: "Claude connects to your remote MCP server from Anthropic's cloud infrastructure, rather than from your local device. This is true across every Claude client, including claude.ai, Claude Desktop, Cowork, and the mobile apps. This means your MCP server must be reachable over the public internet from Anthropic's IP ranges" ([help centre: custom connectors](https://support.claude.com/en/articles/11175166-get-started-with-custom-connectors-using-remote-mcp), Network requirements). Outbound traffic comes from `160.79.104.0/21` ([connector authentication](https://claude.com/docs/connectors/building/authentication)).
- OAuth: supported types are `oauth_dcr`, `oauth_cimd` and client credentials entered when connecting. Claude always sends PKCE `S256`. It "uses only the first entry in your metadata's `authorization_servers` list", requires a 401 (it ignores `WWW-Authenticate` on a 200), and selects CIMD only when the authorization server advertises both `client_id_metadata_document_supported: true` and `none` in `token_endpoint_auth_methods_supported`, otherwise falls back to DCR. The hosted apps use the fixed redirect URI `https://claude.ai/api/mcp/auth_callback`. Discovery and token endpoints get 10 seconds to answer. Machine-to-machine `client_credentials` is not supported ([connector authentication](https://claude.com/docs/connectors/building/authentication)).
- Static header: the developer page lists `static_headers`, "Fixed credential (API key or bearer token) entered by an organization Owner as a request header when adding the connector", as "Beta, for a limited set of organizations", and tells server authors to "Treat it as the organization's credential, not a person's" ([connector authentication](https://claude.com/docs/connectors/building/authentication)). The help centre, however, shows a **Request headers** step in the add-connector instructions for both "Team and Enterprise plans" and "Pro and Max plans" ([help centre](https://support.claude.com/en/articles/11175166-get-started-with-custom-connectors-using-remote-mcp)). Which accounts see the field today is `unchecked`. On a personal Pro or Max account the header would be that one person's, so the "organisation's credential" warning would not apply there.
- Claude Desktop also lists "Local command" connectors that run a program on the person's machine ([custom connectors](https://claude.com/docs/claude-science/custom-connectors)). A local bridge program that forwards to a home Hatchdoor with a header would sidestep the cloud path; no first-party bridge is documented, so this route is `unchecked`.

### ChatGPT

- Adding a custom MCP server: "Authentication options include **OAuth**, **No authentication**, and **OAuth or no authentication**." For OAuth, "if static credentials are provided, then they will be used. Otherwise, ChatGPT can use Client ID Metadata Documents ... ChatGPT can also use DCR when configured" ([OpenAI: add custom MCP server](https://developers.openai.com/api/docs/guides/custom-mcp-server)).
- No keys: ChatGPT "does **not** support machine-to-machine OAuth grants such as client credentials, service accounts, or JWT bearer assertions, nor can it present custom API keys or customer-provided mTLS certificates" ([OpenAI plugins: authentication](https://developers.openai.com/plugins/build/auth), Client identification).
- Requirements on the authorization server: `code_challenge_methods_supported` must include `S256` or the server is unsupported; RFC 9207 `iss` lets ChatGPT use a stable redirect; its CIMD URL is `https://chatgpt.com/oauth/client.json` for servers meeting the issuer requirements ([OpenAI plugins: authentication](https://developers.openai.com/plugins/build/auth)).
- Network: ChatGPT calls from OpenAI's side (its docs offer allowlisting "ChatGPT's published egress IP ranges"). For a server that is not public, OpenAI's Secure MCP Tunnel runs a `tunnel-client` "inside the network that can already reach your MCP server" with outbound HTTPS only, and "OAuth discovery can travel through the tunnel path so the MCP server itself can remain private" ([Secure MCP Tunnel](https://developers.openai.com/api/docs/guides/secure-mcp-tunnels)). Setup needs an OpenAI Platform organisation and API keys with tunnel permissions (same page).

### Codex

- Streamable HTTP servers support "Bearer token authentication", "OAuth authentication, including Client ID Metadata Documents (CIMD) and Dynamic Client Registration (DCR)". Config keys `bearer_token_env_var`, `http_headers` and `env_http_headers`; `codex mcp login <server-name>` starts OAuth ([Codex MCP docs](https://learn.chatgpt.com/docs/extend/mcp?surface=cli), served from the `developers.openai.com/codex/mcp` address).

### Cursor

- Headers with `${env:...}` interpolation. OAuth "uses the authorization code flow with PKCE"; Cursor "reads the server's protected resource metadata (RFC 9728) to find its authorization server and resource indicator (RFC 8707). With dynamic client registration, Cursor registers as a public client". Static `CLIENT_ID` / `CLIENT_SECRET` can replace DCR ([Cursor MCP docs](https://cursor.com/docs/mcp)). CIMD is not mentioned on that page.

### VS Code

- `headers` such as `{"Authorization": "Bearer ${input:api-token}"}` and an `oauth.clientId` for a pre-registered client ([VS Code MCP configuration reference](https://code.visualstudio.com/docs/agents/reference/mcp-configuration)).
- CIMD since 1.106: "When connecting to an MCP server that uses an authorization server that supports CIMD, VS Code will automatically use that flow over DCR" ([VS Code 1.106 release notes](https://code.visualstudio.com/updates/v1_106)). Its extension guide says it starts with DCR and falls back when the provider lacks it ([VS Code MCP developer guide](https://code.visualstudio.com/api/extension-guides/ai/mcp)).
- Caveat for the newer Copilot harness: "There is no built-in MCP configuration option that passes a VS Code `${input:...}` value ... into an HTTP server's authentication header for the Copilot harness"; a key has to come from an environment variable there ([VS Code: migrate customizations](https://code.visualstudio.com/docs/agent-customization/migrate-customizations)).

### Gemini CLI

- `headers` in settings, or `gemini mcp add -H "Authorization: Bearer abc123"`. OAuth by default through `authProviderType: dynamic_discovery`, which performs "dynamic client registration if supported"; a `clientId` is "optional with dynamic registration"; the CLI validates RFC 9207 `iss` ([Gemini CLI MCP docs](https://github.com/google-gemini/gemini-cli/blob/main/docs/tools/mcp-server.md)). CIMD is not mentioned there.

### Zed

- `"headers": { "Authorization": "Bearer <token>" }`, and "When a remote MCP server has no configured `Authorization` header, Zed will prompt you to authenticate yourself against the MCP server using the standard MCP OAuth flow" ([Zed MCP docs](https://zed.dev/docs/ai/mcp)).

## What rmcp 3.8.0 offers

Read from the published crate source Hatchdoor builds against (`rmcp-3.8.0` in the local Cargo registry) and the SDK repository at tag `rmcp-v3.8.0`.

- **The OAuth code is client-side.** The `auth` feature pulls in `oauth2` and `reqwest` (`Cargo.toml` of the crate) and provides `AuthorizationManager`, `AuthClient`, `OAuthState`, credential and state stores, discovery of protected resource and authorization server metadata, CIMD and DCR registration, and scope step-up (`src/transport/auth.rs`; [OAUTH_SUPPORT.md at the tag](https://github.com/modelcontextprotocol/rust-sdk/blob/rmcp-v3.8.0/docs/OAUTH_SUPPORT.md)). All of it is for an MCP client connecting out. Hatchdoor enables only `server` and `transport-streamable-http-server` (`Cargo.toml`).
- **No server-side authorization.** The streamable HTTP server has no token validation, no protected resource metadata endpoint and no `WWW-Authenticate` handling (a search of `src/transport/streamable_http_server*` for OAuth, authorization, bearer and `WWW-Authenticate` returns nothing relevant). It does copy the HTTP request `Parts` into each request's extensions so a handler can read headers and anything middleware inserted (`src/transport/streamable_http_server/tower.rs`, doc comment around line 1276).
- **The SDK's own server examples build everything by hand on axum.** `simple_auth_streamhttp.rs` checks a bearer token in a middleware; `complex_auth_streamhttp.rs` and `cimd_auth_streamhttp.rs` hand-write the metadata, `/authorize`, `/token` and registration endpoints around `StreamableHttpService` ([examples at the tag](https://github.com/modelcontextprotocol/rust-sdk/tree/rmcp-v3.8.0/examples/servers/src)).
- **Hatchdoor already has the seam.** `authorize_mcp_transport` in `src/mcp/routes.rs` validates the request before rmcp sees it and inserts `McpBearerToken` into the request extensions, which the adapter reads to cap subscriptions and tool quotas per token and to sign transfer links. A per-person identity would travel the same way, whichever option is chosen.
- One client-side detail worth knowing: rmcp's default `AuthorizationManager::new` refuses an authorization server on a private address unless the MCP server's own URL is also private (`src/transport/auth.rs`, around line 1448). Codex is built on rmcp (`unchecked` which version), so an instance at a LAN address whose authorization server is at a different private address may be refused by such clients. An authorization server on the same host as Hatchdoor avoids this.

## Outside identity providers

What an outside identity provider needs to be usable from the agent apps above: PKCE `S256` advertised in its metadata, the `resource` parameter (RFC 8707) reflected into the token's audience, and a way for agent apps to get a client ID without the household admin creating one per app. That last point means CIMD (preferred) or open DCR; otherwise every agent app needs a client created by hand.

| Provider | PKCE S256 | `resource` / audience | CIMD | DCR | Source |
| --- | --- | --- | --- | --- | --- |
| Keycloak | Yes | Experimental (`--features=resource-indicators`); without it, an Audience mapper per scope | Experimental (`--features=cimd`), configured through client policies with a trusted-domain list | Yes | [Keycloak MCP guide](https://www.keycloak.org/securing-apps/mcp-authz-server): Keycloak calls its 2026-07-28 and 2025-11-25 conformance "Experimental" |
| Pocket ID | Yes (`plain`, `S256`) | Yes: "APIs" with a resource that becomes the token's `aud` | Yes since v2.13.0, advertised only when an admin sets a CIMD URL allowlist; an API can be granted to "all metadata document clients" | No registration endpoint in its discovery document | [`discovery_handler.go`](https://github.com/pocket-id/pocket-id/blob/main/backend/internal/oidc/discovery_handler.go), [CHANGELOG](https://github.com/pocket-id/pocket-id/blob/main/CHANGELOG.md), [APIs guide](https://pocket-id.org/docs/guides/apis) |
| Authelia | Yes (`require_pkce`, `pkce_challenge_method: S256`) | Yes: `audience` and the RFC 8707 `resource` parameter | Not documented | "not started" on its roadmap | [Authelia roadmap](https://www.authelia.com/roadmap/active/openid-connect-1.0-provider/), [bearer token usage](https://www.authelia.com/integration/openid-connect/oauth-2.0-bearer-token-usage/) |
| authentik | Yes | Not documented on its OAuth2 provider page (`unchecked`) | Not documented | Since 2026.8.0, but "the client authenticates to the registration endpoint using a Bearer access token" with a DCR scope, and each registration creates a new application | [authentik DCR](https://docs.goauthentik.io/add-secure-apps/providers/oauth2/dynamic-client-registration/), [OAuth2 provider](https://docs.goauthentik.io/add-secure-apps/providers/oauth2/) |

Reading of that table:

- Pocket ID and Keycloak can serve agent apps without hand-made clients, through CIMD. Keycloak needs two experimental flags and client-policy setup; Pocket ID needs the CIMD URL allowlist (for example Claude's and ChatGPT's metadata URLs) and an API whose resource is Hatchdoor's MCP address.
- authentik's DCR expects an access token at registration. MCP clients register without one, so this route would not serve them (an inference from authentik's description, `unchecked` against a live client). With Authelia and authentik, each agent app then needs a client created by the admin, and the person must type its client ID into the app. ChatGPT, Claude Code, Cursor, VS Code, Gemini CLI and claude.ai all accept a typed client ID, so this works, but it is the most manual of the OAuth routes.
- Whatever the provider, Hatchdoor would still have to: serve protected resource metadata naming it, validate the token's signature, issuer, expiry and audience on every request (a JWT library and a cached key set, or the provider's introspection endpoint), and map the token's subject to a Hatchdoor person.

## Transfer links and the listening stream, per person

- **Transfer links (ADR-27, ADR-32).** A link is a narrower, separate credential carried in a URL outside the MCP endpoint. The specification's "MUST NOT be included in the URI query string" rule covers access tokens sent to the MCP server, not a server-minted single-file credential; ADR-27 already treats the link as "a new kind of credential, strictly narrower than the token that minted it". Nothing in either revision addresses such out-of-band links, so nothing blocks doing this per person. What must change is inside Hatchdoor: the signing key is derived today from the shared MCP token and its revision (`src/transfer_link.rs`, `key`), and redemption is checked against the live token. Per person, a link has to record which person and which agent credential minted it, and redemption must check that the credential is still valid and the person still holds the grant on that Vault. If the credential is an OAuth access token that is replaced at every refresh, deriving the key from the token itself would strand links at each refresh; deriving it from a stable grant or key identifier avoids that.
- **`subscriptions/listen` (2026-07-28).** The subscriptions page has no rule about authorization: a check for "authoriz", "permission", "credential" and "token" on [the subscriptions page](https://modelcontextprotocol.io/specification/2026-07-28/basic/patterns/subscriptions) found none. The general rules apply: the request that opens the stream carries the credential like any other request, and servers that keep state "**SHOULD** bind handles server-side to the authenticated user ... and reject a handle presented by any other principal" ([security best practices](https://modelcontextprotocol.io/docs/2026-07-28/tutorials/security/security_best_practices)). So the specification allows filtering events by the person's grants and closing a stream when a key is revoked or a grant removed; it does not require either, and it does not say what happens when an access token expires while a stream stays open. Hatchdoor already attributes and caps streams per bearer token (`src/mcp/routes.rs`), which becomes per person or per agent credential.

## What this means for the decision

These are the options for #719, with what each costs. None is chosen here.

### (a) Hatchdoor is its own authorization server

- **Works with:** every app in the table, including claude.ai, Claude Desktop and ChatGPT, provided the instance is reachable from the internet (or, for ChatGPT, through OpenAI's tunnel).
- **Setup for a person:** paste Hatchdoor's address into the agent app, sign in to Hatchdoor in the browser that opens, approve. No copying of secrets.
- **Cancelling:** Hatchdoor deletes the grant (and its refresh token); with short-lived access tokens checked against Hatchdoor's own records on each request, the cut-off can be immediate.
- **What a leak exposes:** a stolen access token works until it expires (the specification asks for short lifetimes); a stolen refresh token works until used by the real client (rotation detects reuse) or until the grant is deleted. The specification adds audience binding, so a token is useless at other services.
- **Cost to build:** the largest. Hatchdoor would need protected resource metadata, authorization server metadata, `/authorize` with a consent page tied to the 3.0 web sign-in, `/token` with PKCE and refresh-token rotation, CIMD fetching with SSRF guards (and DCR for 2025-11-25-era clients that lack CIMD, such as Cursor and Gemini CLI as documented), RFC 9207 `iss`, exact and loopback-aware redirect checks, the `WWW-Authenticate` challenge, and a screen listing and revoking grants. rmcp gives none of this. Authorization endpoints must be served over HTTPS, which for Hatchdoor means a TLS proxy is required, not optional, wherever OAuth is used. It is new security-sensitive code the project would own.

### (b) Hatchdoor is a resource server and trusts an outside identity provider

- **Works with:** the same apps as (a), with the same reachability condition. Keycloak (experimental flags) and Pocket ID (CIMD allowlist) let apps register themselves; with Authelia or authentik the admin creates a client per agent app.
- **Setup for a person:** the same as (a) once running. For the household admin: install, secure and keep up one more service, then configure it for Hatchdoor. That is a large step for a non-technical household, and it ties naturally to whether people sign in to the web UI through that provider too (map #714, "How people sign in", #718).
- **Cancelling:** at the provider (revoking sessions or refresh tokens), plus removing the person's Vault grants in Hatchdoor. Self-contained access tokens stay valid until expiry unless Hatchdoor asks the provider on each request.
- **What a leak exposes:** as in (a), within whatever lifetimes the provider issues.
- **Cost to build:** moderate in Hatchdoor (metadata document, token validation with a JWT library and key fetching, subject-to-person mapping, the challenge header) and high in documentation and support, because each provider's setup differs and two of the four common ones lack self-registration for agent apps.

### (c) Static keys per person or per agent, pasted into the agent

- **Works with:** Claude Code, Codex, Cursor, VS Code, Gemini CLI, Zed. Not ChatGPT. claude.ai and Claude Desktop only where the Request headers field is offered, and then still only if the instance is reachable from Anthropic's cloud.
- **Setup for a person:** sign in to Hatchdoor, create a key named for the agent, copy the one command or config block Hatchdoor shows, paste it into the agent app. One copy and paste, no extra service. The key is a secret sitting in a config file or environment variable on that machine.
- **Cancelling:** delete the key in Hatchdoor; the next request fails. Simple to explain and immediate.
- **What a leak exposes:** everything that person's grants allow, read and write, until someone deletes the key. Keys do not expire unless Hatchdoor adds an expiry. A key per agent limits the damage to that one key and makes the activity record say which agent acted, rather than relying on the self-chosen `Hatchdoor-Origin` label (#570).
- **Cost to build:** small. A key table storing only hashes, a lookup in place of today's single constant-time compare in `validate_mcp_request`, the identity placed in request extensions as `McpBearerToken` is today, and a screen to create and delete keys. Allowed by both specification revisions because authorization is optional.

### (d) A mix: keys now, OAuth where an app needs it

- **Works with:** all apps over time. Keys cover the apps on the person's machine at once; OAuth is added for claude.ai, Claude Desktop and ChatGPT, which matter only for instances reachable from the internet (or through ChatGPT's tunnel).
- **Setup for a person:** a pasted key for local apps; the sign-in button for cloud apps.
- **Cancelling:** both kinds should land in one list of "agent access" per person, so one screen shows and revokes both. If keys and OAuth grants are stored as the same kind of record from the start, adding OAuth later does not change the permission model, transfer links or stream attribution.
- **What a leak exposes:** per credential type, as above.
- **Cost to build:** (c) first, then the cost of (a) or (b) when OAuth is added. Nothing in either revision forbids a server from accepting both its own keys and OAuth tokens it issued itself. One client behaviour to plan for: an app with a configured `Authorization` header uses it and does not fall back to OAuth (Claude Code, Zed), so the two do not collide.

### Points that hold whichever option is chosen

- **Reachability decides more than authentication does.** claude.ai, Claude Desktop, Claude mobile and ChatGPT call from their vendors' clouds. A Hatchdoor reachable only at home works with none of them under any option, except ChatGPT through OpenAI's tunnel program.
- **Identity travels inside Hatchdoor the same way in every option:** a middleware resolves the credential to a person (and an agent credential), puts it in the request extensions, and the tools, transfer links and listening streams read it there. rmcp already passes those extensions through.
- **The 401 today has no `WWW-Authenticate` header.** Any OAuth option must add one with `resource_metadata`; a keys-only option can keep the plain 401.
- **DCR is deprecated in 2026-07-28 but still the only self-registration some documented clients name (Cursor, Gemini CLI).** An OAuth option that supports only CIMD would leave those clients needing a hand-made client ID, and an option that supports only DCR builds on a deprecated mechanism.
