# Security baseline for an instance exposed to the internet

> Research record.
>
> Researched: 2026-11-11
> Resolves: research ticket [#724](https://github.com/BatterWorks/Hatchdoor/issues/724), part of map [#714](https://github.com/BatterWorks/Hatchdoor/issues/714)
> Feeds: [#725](https://github.com/BatterWorks/Hatchdoor/issues/725) "What protects an instance exposed to the internet"; also bears on [#723](https://github.com/BatterWorks/Hatchdoor/issues/723) "Does a shared instance need HTTPS on a domain name?" and [#718](https://github.com/BatterWorks/Hatchdoor/issues/718) "How people sign in"
> Investigated against: `development` at `b50c24a`
> Builds on: [Sign-in options for a self-hosted household app](https://github.com/BatterWorks/Hatchdoor/blob/research/sign-in-options/docs/research/sign-in-options-3.0.md) (#716) and [How agents prove who they act for over MCP](https://github.com/BatterWorks/Hatchdoor/blob/research/mcp-agent-auth/docs/research/mcp-agent-authorization-3.0.md) (#717)
> Status: Decision input. It lists the baseline and the choices, and does not make them.

## Summary in plain words

Once Hatchdoor can be reached from the internet, anyone can knock on the door, not just the household. Most of the knocking is automated: programs that try common passwords on every login page they find. The baseline below is what the security standards and the comparable apps agree a small shared app needs so that this knocking gets nowhere.

What someone on the internet could try, and what protects against each:

1. **Guess a password or a key.** Protected by long passwords that are not on lists of common ones, by slowing down or blocking repeated failed attempts, and by an optional second step at sign-in (a six-digit code from a phone app, or a passkey) so that a guessed or leaked password alone is not enough.
2. **Use a password leaked from another site.** People reuse passwords. Only the second step really protects here; checking new passwords against a list of known-leaked ones helps.
3. **Steal a signed-in session.** After signing in, the browser holds a pass that proves it already signed in. It must be long and random, kept where the page's own scripts cannot read it (a cookie marked `HttpOnly`), sent only over HTTPS, and it must expire and be cancellable: signing out, "sign out everywhere", and the admin ending someone's sessions.
4. **Trick a signed-in browser into acting** (a malicious site sending requests on the person's behalf, called cross-site request forgery). Protected by cookie settings (`SameSite`) and by checking where a request came from. Today's design is mostly immune because the token travels in a header, but moving to cookies brings this back.
5. **Reach someone else's Vault.** Every request, from the browser, from an agent over MCP, through a transfer link, in an "all Vaults" search, must check that this person was granted this Vault, on the server.
6. **Inject a script into a note that runs in another person's browser.** Once several people share Vaults, one person's note is rendered in another's browser, the admin's included. Protected by never rendering raw HTML from notes (Hatchdoor already does this) and by a Content Security Policy header that stops injected scripts from running.
7. **Read secrets from logs and addresses.** Keys must not appear in web addresses, proxy logs or Hatchdoor's own logs.
8. **Go unnoticed.** Every sign-in, failed or not, should be logged with who and from where, so a tool such as fail2ban can block an address and the admin can see that something is happening.

What Hatchdoor has today ([details](#what-hatchdoor-has-today)): two shared secrets, one for the browser and one for agents, compared safely; a refusal to start on a network address without the browser secret; strict origin checks for the case with no secret; per-key limits on agent tool calls; secrets removed from its own logs; notes rendered without raw HTML; SVG files made harmless. Its own manual today says to never put it directly on the internet.

What it lacks against the baseline: any account or per-person check; any limit on wrong guesses (a wrong browser or agent secret can be retried as fast as the network allows, and nothing is logged at the default log level); any knowledge of the caller's network address; sessions that expire or can be ended; sign-out; a second sign-in step; the standard browser security headers; and any way to tell the admin that something is wrong.

The standards point the same way on one big choice. OWASP's verification standard, at the level most applications should aim for, requires a second sign-in step for everyone unless a documented reason and other protections replace it. Of the eight comparable apps, two offer a second step (Nextcloud and Paperless-ngx), both optional by default; the other six have none. That gap between the standard and practice is the main decision for #725.

## Gap table

Size is a rough guess of build effort: **S** is a few days of focused work, **M** about a week or two, **L** several weeks or a whole decision of its own. "Owned by" names the map ticket where the item is decided, when it is not #725.

| Baseline item | Source | Hatchdoor today | What 3.0 would need | Size |
|---|---|---|---|---|
| A sign-in per person, password stored as a slow salted hash | ASVS 6.2, NIST 3.1.1.2 | One shared web token, any non-empty string (`config.rs`, `web_bearer_token`) | Accounts, Argon2id hashes; owned by #718 | L |
| Password rules: at least 8 characters (15 recommended, 15 required by NIST when a password is the only factor), up to 64 or more, no character-class rules, paste and password managers allowed, checked against common and leaked passwords | ASVS 6.2.1 to 6.2.12, NIST 3.1.1.2 | Not applicable (no passwords) | Length rules, a bundled list of at least 3,000 common passwords, a "show password" toggle | S |
| Limit failed sign-in attempts, and document how, without letting an attacker lock the household out | ASVS 6.1.1, 6.3.1, NIST 3.2.2 | Nothing. Wrong web and MCP tokens are refused but not counted; the MCP quota counts only calls that already passed the token check (`mcp/routes.rs`, order at `validate_mcp_request` then `check_quota_units`) | Per-account and per-address slowing or blocking on the sign-in route, the second-step route and agent-key checks | M |
| Know the caller's network address | Needed by per-address limits and ASVS 16.2.1 | Not captured: the server is started without connection info (`server.rs`, `serve_until_shutdown`) and forwarded headers are trusted only for transfer-link addresses (ADR-34) | Peer address, plus a trusted-proxy setting to read `X-Forwarded-For` or `Forwarded`; a new ADR, since ADR-34 says these headers affect nothing else | S, plus ADR |
| A second sign-in step | ASVS 6.3.3 (L2), 6.5, NIST AAL2 | None | Authenticator codes (TOTP) with recovery codes, or passkeys, or trust in an identity provider's second step | M (TOTP), L (passkeys) |
| Session after sign-in: random, at least 128 bits, new at each sign-in, held in a `Secure`, `HttpOnly`, `SameSite`, `__Host-` cookie, not in local storage | ASVS 7.2, 3.3, NIST 5.1 | The token sits in the PWA's `localStorage` (`frontend/src/api/api.ts`) and rides in an `access_token` query parameter for images, streams and downloads (ADR-08) | Server-side sessions and a cookie, replacing the stored token and the query parameter | M |
| Session lifetime: an idle timeout and an absolute limit | ASVS 7.1.1, 7.3, NIST 2.1.3, 2.2.3 | None: the token is valid until the operator changes it | Two limits, chosen in #725 | S (with sessions) |
| Sign-out, sign out everywhere, admin ends a person's sessions, sessions end when an account is disabled | ASVS 7.4.1 to 7.4.5, 7.5.2 | None; rotating the shared token signs everyone out | A session list per person and for the admin | S to M (with sessions) |
| Ask again before sensitive changes (password, second step, agent keys) | ASVS 7.5.1, 6.2.3 | None | Re-enter the password, or the second step, before those changes | S |
| Cross-site request forgery defence | ASVS 3.5.1 to 3.5.3, 3.3.2, NIST 5.1 | Not needed with a header token; the token-less loopback mode checks `Origin` and `Sec-Fetch-Site` (`auth.rs`, `require_local_host_and_origin`) | With cookies: `SameSite`, plus the same `Origin` check on every state-changing request | S |
| Per-Vault access checked on the server for every path | ASVS 8.2.1, 8.2.2, 8.3.1 | No per-person checks: either token opens every Vault | Grants checked on web, MCP, transfer links, events streams, attachments and "all" scopes; owned by the access tickets on the map | L |
| Agent keys per person, random, revocable, limited per key | ASVS 13.2.1 (by analogy), MCP authorization research | One shared MCP token, any non-empty string (`mcp/config.rs`, `from_snapshot`); per-token tool quota 120 per minute and session caps (`mcp/limits.rs`) | Keys per person and agent, stored hashed, shown once, with last use; owned by #719 | M |
| Browser security headers: Content Security Policy with `frame-ancestors`, `X-Content-Type-Options: nosniff`, `Referrer-Policy`, HSTS over HTTPS | ASVS 3.4.1, 3.4.3 to 3.4.6 | Only on SVG assets: `Content-Security-Policy: sandbox` and `Content-Disposition: attachment` (`handlers/assets.rs`, `asset_response`) | One header layer on every response; HSTS only when served over HTTPS (RFC 6797) | S |
| Log every sign-in, success and failure, with who, how and from where; log refused access; no secrets in logs | ASVS 16.2.1, 16.2.5, 16.3.1, 16.3.2 | Secrets are redacted from the request log (`server.rs`, `traced_uri`; `auth.rs`, `redact_query_token`; `config.rs`, `within_mcp_library_cap`). A 401 or 403 is not logged at the default level, and no address is known | Explicit security log lines in a stable format that fail2ban or CrowdSec can match | S |
| Tell the admin about suspicious activity | ASVS 6.3.5, 6.3.7 (L3), NIST account notifications | No channel: Hatchdoor sends no email and has no notifications | An admin screen of recent security events, at minimum; an outbound alert is a further choice | M |
| TLS with a public certificate, no fallback to HTTP | ASVS 12.1.1, 12.2.1, 12.2.2 | Plain HTTP, relying on a proxy in front; the manual says never expose it directly | A documented proxy requirement, or a refusal to run multi-user over plain HTTP; owned by #723 | S (docs) |
| No default accounts; a recovery path that does not skip the second step | ASVS 6.3.2, 6.4.3 | No accounts | First admin without a browser step (ADR-14), recovery by a subcommand that also resets the second step | S |

## Which ASVS level fits, and why

The current OWASP Application Security Verification Standard is version 5.0.0, released 2025-05-30 ([release](https://github.com/OWASP/ASVS/releases/tag/v5.0.0_release)). Every ASVS citation below is to the chapter files at that tag. A newer "bleeding edge" build exists on the project's main branch (released 2026-10-26 as `latest`), but it is not a numbered release and was not used.

ASVS has three levels ([What is the ASVS](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x03-What-is-the-ASVS.md)):

- **Level 1** "contains the minimum requirements ... first-layer of defense requirements for preventing common attacks that do not require other vulnerabilities or preconditions". Password rules sit here because "from higher levels, the multi-factor authentication requirements become relevant".
- **Level 2**: "Most applications should be striving to achieve this level of security."
- **Level 3** is for "applications looking to demonstrate the highest levels of security", mostly defence in depth.

The standard leaves the choice to the owner, "depending on the sensitivity of the application", and encourages a tailored fork that omits irrelevant sections (same chapter).

**Level 2 is the fit, with Level 1 as the floor that must hold in full.** Reasons:

- The data is personal notes, which in practice hold health, money, family and work matters, and agents can write as well as read.
- The instance is reachable from the internet by design (the map's own note, "Judge every decision as if the instance were online"), so every Level 1 attack applies without any precondition.
- Several Level 2 items exist precisely for a shared, multi-person app: ending sessions when an account is disabled, letting people see and end their own sessions, admin control of sessions, logging failed authorisation.

Some Level 2 items do not fit a one-container household app and would be named as deliberate exceptions rather than silently skipped: shipping logs to a separate system (16.4.3) is the operator's job, and the full written inventories (16.1.1, 8.1.x) shrink to a page in the manual. The Level 2 item that needs an explicit decision is **6.3.3, a second sign-in step for everyone**; it is discussed under [what this means for the decision](#what-this-means-for-the-decision).

## What Hatchdoor has today

Read from the code at `b50c24a`. Only what the code shows.

**Who can get in.**
- The web API is guarded by one shared token, `HATCHDOOR_WEB_BEARER_TOKEN`, accepted as `Authorization: Bearer` or as an `access_token` query parameter (`src/auth.rs`, `require_web_token` and `request_is_authorized`; ADR-08). Any non-empty value is accepted; there is no minimum length (`src/config.rs`, the `web_bearer_token` parsing in the config loader). A token generated by Hatchdoor is 256 random bits (`src/auth.rs`, `generate_bearer_token`).
- MCP has its own shared token, `HATCHDOOR_MCP_BEARER_TOKEN`, also any non-empty value (`src/mcp/config.rs`, `McpConfig::from_snapshot`), checked after an `Origin` allow-list (`src/mcp/auth.rs`, `validate_mcp_request`; ADR-09).
- Both comparisons hash both sides with BLAKE3 and compare the digests without short-circuiting, so timing does not reveal how much of a guess was right (`src/auth.rs`, `constant_time_eq`).
- A wrong token gets `401` with `WWW-Authenticate: Bearer` on the web side (`src/auth.rs`, `unauthorized`) and a JSON-RPC error with `401` on MCP (`src/mcp/auth.rs`).
- The web token also guards Settings, where the MCP token can be changed (`src/server.rs`, the `settings` router layered with `require_web_token`), so the web token is effectively the admin credential.

**Refusing unsafe setups.**
- A non-loopback bind without a web token refuses to start, and saves a freshly generated token to `new-web-token.env` (mode 0600) instead of logging it (`src/server.rs`, `check_web_auth_posture` and `save_new_web_token`; ADR-07).
- With no web token (loopback only), every request must name a loopback host or the public URL's host, and a state-changing request from another site is refused by `Origin` or, without one, by `Sec-Fetch-Site: cross-site` (`src/auth.rs`, `require_local_host_and_origin`; ADR-07 note for #629). This is the only cross-site request check outside `/mcp`; with a token set, the header token itself is what keeps other sites out.
- Demo mode refuses to start with MCP or Git sync (ADR-07).

**Limits.**
- MCP: 120 tool calls per rolling minute per token, 8 concurrent tool calls and 2 concurrent searches process-wide, 60 handshakes per minute, 32 legacy sessions per token and 64 in all; over-limit answers are `429` with `Retry-After` (`src/mcp/limits.rs`, constants and `RateLimiter`). All of it can be switched off with `HATCHDOOR_MCP_RATE_LIMITS_ENABLED` for deployments behind their own gateway.
- These limits run after the token check (`src/mcp/routes.rs`: `validate_mcp_request` first, then `try_acquire` and `check_quota_units`). Failed token checks are not counted anywhere, on MCP or on the web API.
- Request bodies are capped: `DefaultBodyLimit` on `/mcp` and on attachment uploads (`src/server.rs`, `mcp_body_limit` and `attachment_body_limit`), axum's default elsewhere.
- No request timeout layer is configured on the router (`src/server.rs`, `build_router`).

**Transfer links** carry their own signed credential, name one file, expire five minutes after minting, and die on restart or MCP token change because the signing key lives only in memory (`src/transfer_link.rs`, `LINK_LIFETIME`; ADR-27, ADR-32).

**Logging.**
- Every request gets a trace span whose URI has `access_token` and a transfer link's `signature` replaced by `REDACTED` (`src/server.rs`, `traced_uri`; `src/auth.rs`, `redact_query_token`).
- The MCP library's debug and trace output is capped at info whatever `RUST_LOG` says, because tool results can carry transfer links (`src/config.rs`, `within_mcp_library_cap`).
- The default filter is `hatchdoor=info,tower_http=info` (`src/config.rs`, `init_logging`). The trace layer logs responses with tower-http's `DefaultOnResponse`, whose default level is DEBUG (tower-http 0.7.1, `src/trace/mod.rs`, `DEFAULT_MESSAGE_LEVEL`), and Hatchdoor classifies only 5xx as failures (`src/server.rs`, `MakeFailureClassifier`). A `401` or `403` therefore leaves no line at the default level.
- No client address is known to any handler: the server runs `axum::serve(listener, app)` without `into_make_service_with_connect_info` (`src/server.rs`, `serve_until_shutdown`), and no code reads `X-Forwarded-For` or `ConnectInfo`.

**Browser side.**
- The PWA stores the web token in `localStorage` under `hatchdoor_web_token` and appends it as `access_token` to URLs that cannot carry a header (`frontend/src/api/api.ts`).
- Notes render through `react-markdown` without `rehype-raw` (`frontend/package.json`; no `rehype-raw` or `allowDangerousHtml` in `frontend/src`), so raw HTML in a note is not rendered. Mermaid diagrams are inserted as SVG with `securityLevel: "strict"` (`frontend/src/components/note-page/RendererComponents.tsx`).
- SVG assets are served with `Content-Security-Policy: sandbox` and `Content-Disposition: attachment`, and assets get `Cache-Control: private` so a proxy does not store a URL carrying the token (`src/handlers/assets.rs`, `asset_response`).
- No other security header is set: a search of `src/` and the frontend for `Content-Security-Policy`, `X-Frame-Options`, `nosniff`, `Strict-Transport-Security`, `Referrer-Policy` and `frame-ancestors` finds only the SVG case above.
- No cookies are set anywhere.

**What the manual says.** "Never put Hatchdoor directly on the internet. If you need to reach it from outside, put it behind a service that adds its own sign-in and encryption, such as a VPN" (`docs/user-vault/01 Get started/Install Hatchdoor with Docker Compose.md`); the same rule is in "The security model" and "Connect your agent". 3.0 reverses this, so those pages change with it.

## OWASP ASVS 5.0 reduced to Hatchdoor

Only the requirements that apply to a household app with people, an admin, agent keys and whole-Vault grants. Level in brackets. Items about SMS, email factors, SAML, hardware-attested authenticators and multi-tenant isolation are left out, as are Level 3 items unless they matter to a choice.

### Authentication (V6)

From [V6 Authentication](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x15-V6-Authentication.md).

**Write down the anti-guessing controls.**
- 6.1.1 [L1]: documentation "defines how controls such as rate limiting, anti-automation, and adaptive response, are used to defend against attacks such as credential stuffing and password brute force", and must "prevent malicious account lockout".
- 6.3.1 [L1]: those controls are implemented as documented.
- 6.1.3 and 6.3.4 [L2]: every way in (password, identity provider, agent key, recovery command) is documented and held to the same strength; "no undocumented pathways".

**Passwords.**
- 6.2.1 [L1]: "at least 8 characters in length although a minimum of 15 characters is strongly recommended".
- 6.2.2, 6.2.3 [L1]: people can change their password, and a change asks for the current one.
- 6.2.4 [L1]: new passwords are checked against "at least, the top 3000 passwords".
- 6.2.5 [L1]: no rules about character types.
- 6.2.6 to 6.2.8 [L1]: masked input, paste and password managers allowed, the password verified exactly as typed.
- 6.2.9 [L2]: "passwords of at least 64 characters are permitted".
- 6.2.10 [L2]: no forced periodic change.
- 6.2.12 [L2]: checked against "a set of breached passwords". The standard does not say the set must be fetched online; a bundled list satisfies the wording.

**General.**
- 6.3.2 [L1]: no default accounts such as `admin`. Matters for how the first admin is created (ADR-14).
- 6.3.3 [L2]: "either a multi-factor authentication mechanism or a combination of single-factor authentication mechanisms, must be used in order to access the application". "Relaxing any of the considerations in this requirement requires a fully documented rationale and a comprehensive set of mitigating controls." The section introduction states it plainly: "L2 applications must force the use of multi-factor authentication (MFA)."
- 6.3.5 [L3]: tell people about suspicious sign-in attempts. 6.3.7 [L3]: tell people after their credentials change. 6.3.8 [L3]: wrong-username and wrong-password failures look the same. Level 3, but cheap and relevant to the "telling the admin" choice.

**Lifecycle and recovery.**
- 6.4.1 [L1]: a temporary password made by the system is random, follows the policy, and expires quickly or after first use; it may not become the long-term password. This is the "admin resets a person" path from the sign-in research.
- 6.4.2 [L1]: no password hints or secret questions.
- 6.4.3 [L2]: password reset "does not bypass any enabled multi-factor authentication mechanisms". A recovery subcommand that resets a password must therefore deal with the second step deliberately.
- 6.4.6 [L3]: the admin can start a reset but cannot choose the person's password.

**Second-step mechanics, when one is offered** ([L2] unless noted).
- 6.5.1: codes, recovery codes and TOTP values work once.
- 6.5.2: recovery codes under 112 bits are stored with a password hash.
- 6.5.3: secrets come from a cryptographic random generator.
- 6.5.5: a TOTP has "a maximum lifetime of 30 seconds".
- 6.5.6 [L3]: any factor can be revoked when lost.

**Identity provider, when one is used.**
- 6.8.4 [L2]: if Hatchdoor expects a certain strength (a second step), it checks the provider's `acr`, `amr` or `auth_time` claims, or assumes the weakest sign-in when they are absent. Paperless-ngx implements exactly this with `PAPERLESS_SOCIAL_ACCOUNT_MFA_TRUSTED_AMR`, which skips its own second step only when the provider's `amr` claim contains a trusted value ([configuration.md](https://github.com/paperless-ngx/paperless-ngx/blob/dev/docs/configuration.md)).

### Session management (V7)

From [V7 Session Management](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x16-V7-Session-Management.md).

**The session itself.**
- 7.2.1 [L1]: session checks happen on the server.
- 7.2.2 [L1]: sessions use tokens "dynamically generated for session management, i.e. not using static API secrets and keys". Today's shared web token is exactly the static secret this rules out for browser sessions.
- 7.2.3 [L1]: reference tokens with "at least 128 bits of entropy".
- 7.2.4 [L1]: "a new session token on user authentication, including re-authentication".

**Lifetime.**
- 7.1.1 [L2]: the idle timeout and absolute lifetime are documented, with "justification for any deviations from NIST SP 800-63B re-authentication requirements".
- 7.3.1, 7.3.2 [L2]: both limits exist.
- 7.1.2 [L2]: how many parallel sessions one account may have is documented.

**Ending sessions.**
- 7.4.1 [L1]: after sign-out or expiry the session is dead on the server.
- 7.4.2 [L1]: disabling or deleting an account ends all its sessions.
- 7.4.3 [L2]: after a password or second-step change, offer to end the other sessions.
- 7.4.4 [L2]: sign-out is easy to find.
- 7.4.5 [L2]: "application administrators are able to terminate active sessions for an individual user or for all users".
- 7.5.2 [L2]: people can see their sessions and end any or all of them, after confirming with one factor.

**Re-checking identity.**
- 7.5.1 [L2]: "full re-authentication before allowing modifications to sensitive account attributes", such as the second-step setup. Agent keys are the closest Hatchdoor equivalent.
- 7.6.1 [L2]: with an identity provider, session lifetimes between Hatchdoor and the provider behave as documented.

### Access control (V8)

From [V8 Authorization](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x17-V8-Authorization.md).

- 8.1.1 [L1]: the rules for who may do what with which data are written down. For 3.0 that is the grant model the map is deciding.
- 8.2.1 [L1]: function-level access only with explicit permission, for example admin-only screens and tools.
- 8.2.2 [L1]: "data-specific access is restricted to consumers with explicit permissions to specific data items to mitigate insecure direct object reference (IDOR)". In Hatchdoor a `vault_id` in a URL, an MCP argument, a transfer link or an events stream is such a reference; each must be checked against the person's grants.
- 8.3.1 [L1]: rules are enforced on the server, not in the PWA.
- 8.3.3 [L3]: access is decided on the originating person's permissions, not on an intermediary's. Relevant to agents: an agent key acts with its person's grants, never more.
- 8.4.2 [L3]: admin interfaces get several layers of protection. Relevant to whether the admin alone must use a second step.

### Security logging (V16)

From [V16 Security Logging and Error Handling](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x25-V16-Security-Logging-and-Error-Handling.md). All Level 2.

**Content of each line.**
- 16.2.1: each entry carries "when, where, who, what".
- 16.2.2: timestamps in UTC or with an offset.
- 16.2.4: a format a log processor can read. fail2ban and similar tools match lines with regular expressions, so a stable line shape matters.
- 16.2.5: credentials are never logged, and session tokens only hashed or masked. Hatchdoor already does this for its tokens.

**What to log.**
- 16.3.1: "all authentication operations are logged, including successful and unsuccessful attempts", with the type of authentication.
- 16.3.2: "failed authorization attempts are logged", for 3.0 a refusal because a Vault was not granted.
- 16.3.3: attempts to get around anti-automation are logged.

**Protecting and handling.**
- 16.4.1: log injection is prevented by encoding. A username typed at a login box ends up in a log line.
- 16.4.2, 16.4.3: logs are protected and shipped elsewhere. For a container writing to standard output these are the operator's side.
- 16.5.1: generic error messages to the caller.

### Browser-facing items (V3) and transport (V12)

From [V3 Web Frontend Security](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x12-V3-Web-Frontend-Security.md) and [V12 Secure Communication](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x21-V12-Secure-Communication.md).

**Cookies.**
- 3.3.1 [L1]: cookies have `Secure` and the `__Host-` or `__Secure-` prefix.
- 3.3.2 [L2]: `SameSite` set to suit the cookie's purpose.
- 3.3.3 [L2]: `__Host-` unless the cookie must be shared with other hosts.
- 3.3.4 [L2]: session cookies are `HttpOnly` and the value is sent only in `Set-Cookie`.

**Headers.**
- 3.4.1 [L1]: `Strict-Transport-Security` on all responses, at least one year.
- 3.4.3 [L2]: a Content Security Policy with at least `object-src 'none'` and `base-uri 'none'`.
- 3.4.4 [L2]: `X-Content-Type-Options: nosniff` on all responses.
- 3.4.5 [L2]: a referrer policy.
- 3.4.6 [L2]: `frame-ancestors` in the CSP; "X-Frame-Options ... is obsolete and may not be relied upon".

**Cross-site requests.**
- 3.5.1 to 3.5.3 [L1]: cross-site request forgery defences, either anti-forgery tokens or checks that requests come from the app itself, and no sensitive action on `GET`. 3.5.3 names "strict validation of the Sec-Fetch-* request header fields" as an accepted way, which is what `require_local_host_and_origin` already does for the token-less case.

**Wrong-context rendering.**
- 3.2.1 [L1]: user files requested directly must not render in the wrong context; it names "the sandbox directive of the Content-Security-Policy" and "the attachment disposition type", exactly what `asset_response` does for SVG. Shared Vaults raise the stakes: an attachment one person uploads is opened by another.

**Transport.**
- 12.1.1 [L1]: TLS 1.2 or 1.3 only.
- 12.2.1 [L1]: TLS for all client connections "and does not fall back to insecure or unencrypted communications".
- 12.2.2 [L1]: publicly trusted certificates.

In Hatchdoor's architecture these sit with the TLS proxy in front, but Hatchdoor decides whether it will run a multi-user instance on plain HTTP at all (#723).

**HSTS placement.** HSTS cannot be sent by Hatchdoor itself on plain HTTP: "An HSTS Host MUST NOT include the STS header field in HTTP responses conveyed over non-secure transport" ([RFC 6797, 7.2](https://www.rfc-editor.org/rfc/rfc6797.html)). Behind a TLS proxy, either the proxy sets it, or Hatchdoor sets it when its public address is `https://`.

## NIST SP 800-63B

The current revision is SP 800-63B-4, published August 2025 ([SP 800-63B](https://pages.nist.gov/800-63-4/sp800-63b.html); page dated 2025-08-26). It is written for US federal systems, and its numbers are upper bounds and defaults for them; ASVS 5.0 builds its authentication chapter on it ([V6 introduction](https://github.com/OWASP/ASVS/blob/v5.0.0_release/5.0/en/0x15-V6-Authentication.md)). Section names below are from the page.

### Passwords ("Password Verifiers")

- "Verifiers and CSPs **SHALL** require passwords that are used as a single-factor authentication mechanism to be a minimum of 15 characters in length." Passwords used only with a second factor may be shorter, but at least eight. So the minimum length depends on the second-step decision: 15 if passwords can be used alone, 8 if a second step is always required.
- Maximum length "at least 64 characters" (SHOULD).
- All printable ASCII and space accepted (SHOULD). Unicode accepted (SHOULD), counted per code point, normalised with NFC before hashing.
- No composition rules and no periodic changes (SHALL NOT); a forced change only on evidence of compromise.
- No hints or security questions (SHALL NOT).
- New passwords compared against "a blocklist that contains known commonly used, expected, or compromised passwords", including "context-specific words, such as the name of the service, the username". The blocklist need not be huge: "Excessively large blocklists are of little incremental security benefit because the blocklist is used to defend against online attacks, which are already limited by the throttling requirements".
- Password managers and autofill allowed (SHALL), paste allowed (SHOULD), an option to show the password while typing (SHOULD).
- Stored "salted and hashed using a suitable password hashing scheme" with a salt of at least 32 bits. The sign-in research covers Argon2id and the `argon2` crate's defaults, which match OWASP's recommended parameters ([sign-in options, Passwords kept by Hatchdoor](https://github.com/BatterWorks/Hatchdoor/blob/research/sign-in-options/docs/research/sign-in-options-3.0.md)).

### Limiting repeated attempts ("Rate Limiting (Throttling)")

- The verifier "**SHALL** limit consecutive failed authentication attempts using a specific authenticator on a single subscriber account to no more than 100 by disabling that authenticator". A disabled authenticator must be bound again (re-enrolled) before use.
- "The limit of 100 attempts is an upper bound, and agencies **MAY** impose lower limits."
- To keep attackers from locking the real person out, it lists optional techniques: a bot challenge; making the person "wait after a failed attempt for a period of time that increases as the subscriber account approaches its maximum allowance for consecutive failed attempts (e.g., 30 seconds up to an hour)"; and risk signals such as the IP address.
- After a successful sign-in the verifier SHOULD disregard earlier failures.
- The same throttling SHALL apply to look-up secrets ("Look-Up Secret Verifiers") and to saved recovery codes ("Saved Recovery Codes"), and to OTPs shorter than 64 bits ("Single-Factor OTP Verifiers"), which covers six-digit authenticator codes.

### Second sign-in steps

- **AAL1** (one factor) allows a password alone, and verifiers "SHOULD make multi-factor authentication options available at AAL1 and encourage their use".
- **AAL2** requires "either a multi-factor authenticator or a combination of two separate authentication factors". A password plus a single-factor OTP (an authenticator app code) or plus a look-up secret qualifies. Verifiers "**SHALL** offer at least one phishing-resistant authentication option at AAL2".
- Phishing resistance requires cryptographic authentication; OTPs "**SHALL NOT** be considered phishing-resistant because the manual entry does not bind the authenticator output to the specific session" ("Phishing Resistance"). Passkeys are the household-scale phishing-resistant option.
- Each OTP is accepted "only once while it is valid" ("Single-Factor OTP Verifiers").
- **Recovery codes** ("Saved Recovery Codes"): at least 64 random bits, stored hashed, throttled, invalidated after use and replaced.

### Sessions and reauthentication

**Timeouts by level.**
- AAL1: an overall timeout that "SHOULD be no more than 30 days"; an inactivity timeout "MAY be applied but is not required".
- AAL2: overall "no more than 24 hours", inactivity "no more than 1 hour". After the inactivity timeout but before the overall one, the verifier may accept the password alone together with the session secret.

**The session secret** ("Session Bindings").
- At least 64 random bits (ASVS asks 128).
- Erased at logout.
- "SHOULD NOT be placed in insecure locations (e.g., HTML5 Local Storage) due to the potential exposure of local storage to cross-site scripting (XSS) attacks". Hatchdoor's PWA keeps its token there today.
- "POST/PUT content **SHALL** contain a session identifier that the RP **SHALL** verify to protect against cross-site request forgery (CSRF)."
- "Sessions SHOULD provide a readily accessible mechanism for subscribers to terminate (i.e., log off) their session."

**Browser cookies.** Cookies SHALL be HTTPS-only and scoped to the minimum hosts and paths. They SHOULD be `HttpOnly`, carry the `__Host-` prefix with `Path=/`, set `SameSite=Lax` or `Strict`, and hold only an opaque identifier.

**Agent tokens** ("Access Tokens"): an access token is not proof the person is present, and "could be valid long after the authentication session has ended". This is the honest description of an agent key: it acts while the person is away, so it needs its own limits and revocation, not the browser session's.

### Telling people ("Account Notifications")

Events such as binding a new authenticator or account recovery "require the subscriber or someone designated by them to be independently notified", through an address such as email, text message or push notification. Hatchdoor has no such channel. In a household, the closest equivalents are a notice inside the app at next sign-in and a notice to the admin; whether that suffices is part of the #725 choice.

## How comparable apps do it

The same eight apps as the [sign-in research](https://github.com/BatterWorks/Hatchdoor/blob/research/sign-in-options/docs/research/sign-in-options-3.0.md#how-comparable-apps-do-it). "Not in the docs read" means the pages and code cited did not mention it; it is not proof the feature is absent.

| App | Second sign-in step | After repeated failed sign-ins | How long a sign-in lasts; signing out everywhere | Telling the admin |
|---|---|---|---|---|
| Nextcloud | TOTP, notification and WebAuthn providers, optional per person; admin can enforce for everyone or chosen groups; 10 backup codes | Per-address slowdown up to 25 s; after 10 failures in 30 min, `429` until it drops; on by default; documented fail2ban filter | 24 h of inactivity; "Remember me" 15 days; list of connected browsers and devices, each revocable; per-device app passwords | Not in the docs read |
| Paperless-ngx | TOTP, optional per person, 10 recovery codes; superuser can turn it off for someone; can trust the identity provider's second step | API token endpoint limited to 5 per minute by default; `PAPERLESS_TRUSTED_PROXIES` for correct addresses in fail2ban log entries | 2 weeks by default (`PAPERLESS_SESSION_COOKIE_AGE`), or until the browser closes | Not in the docs read |
| Jellyfin | Not in the docs read | Account disabled after 3 failures (5 for admins), per user setting; admin unlocks by hand; documented fail2ban filter | "Disable this user" ends existing connections | A lockout appears in the activity feed on the admin dashboard |
| Mealie | Not in the docs read | Account locked after 5 failures for 24 hours (both configurable); API otherwise not rate limited, proxy suggested | 48 hours (`TOKEN_TIME`) | Not in the docs read |
| Actual Budget | Not in the docs read (outside provider for multi-user) | 5 failed attempts per 15 minutes on login, bootstrap and password change, successful ones not counted | Not in the code read | Not in the docs read |
| Audiobookshelf | Not in the docs read | 40 attempts per 10 minutes per address on login and token refresh, configurable or off; failures logged with address | Access token 1 hour, refresh token 30 days, both configurable; a function to end all of a user's sessions | Rate-limit hits logged with address and user agent |
| Karakeep | Not in the docs read | API rate limiting off by default (`RATE_LIMITING_ENABLED`) | Not in the docs read | Not in the docs read |
| Immich | Not in the docs read (a PIN exists for the locked folder, not for sign-in) | No attempt counter in the login function; failures logged with address | "Authorized Devices" list and "Log Out All Devices"; password change and the admin reset command offer to end sessions | Not in the docs read |

### Nextcloud

**Failed sign-ins.** Brute-force protection is built in and on by default ([Brute force protection](https://github.com/nextcloud/documentation/blob/master/admin_manual/configuration_server/bruteforce_configuration.rst)):
- It "makes requests - coming from an IP address via a brute force protected entrypoint - slower for up to a 24 hour period", and "in extreme circumstances it may prevent access outright, for up to 30 minutes".
- "The maximum delay is 25 seconds, unless maximum number of attempts (currently 10) was reached within the last 30 minutes", after which it answers `429 Too Many Requests`.
- The filter is per address, deliberately: an account is "not impacted when it is connecting from a different IP address than any brute force attempts", which "helps minimize inadvertent denial of service attacks against legitimate connections".
- A successful sign-in from the same address clears earlier failures.
- Behind a reverse proxy, `trusted_proxies` and `forwarded_for_headers` must be set, "Otherwise it can happen that Nextcloud actually starts throttling all traffic coming from the reverse proxy".
- Turning it off is "heavily discouraged for production servers, particularly if your server is reachable via a public IP address".

**fail2ban.** The hardening guide gives a filter that matches the log messages `Login failed:`, `Two-factor challenge failed:` and `Trusted domain error.` with the client address. It notes that `audit.log` "currently only logs successful logins and cannot be used" ([Hardening, Setup fail2ban](https://github.com/nextcloud/documentation/blob/master/admin_manual/installation/harden_server.rst)).

**Second step** ([Two-factor authentication](https://github.com/nextcloud/documentation/blob/master/admin_manual/configuration_user/two_factor-auth.rst)):
- Providers for TOTP, for approval from an already signed-in device, and backup codes ("Generates ten backup codes"). WebAuthn is also offered as a second factor (sign-in research).
- "By default 2FA is *optional*". Admins can enforce it "system-wide (all users) or for selected groups only".

**Sessions.**
- `session_lifetime` defaults to 24 hours "after inactivity"; "Remember me" logins last 15 days by default (`remember_login_cookie_lifetime`); `auto_logout` is off by default ([config.sample.php parameters](https://github.com/nextcloud/documentation/blob/master/admin_manual/configuration_server/config_sample_php_parameters.rst)).
- Each person sees "connected browsers and devices" and can revoke each. With 2FA on, "device-specific passwords are the only way to configure clients" ([Manage connected browsers and devices](https://github.com/nextcloud/documentation/blob/master/user_manual/session_management.rst)). That is the same split as a person's sign-in versus their agent keys.

### Paperless-ngx

- **Second step.** "Users can enable two-factor authentication (2FA) for their accounts from the 'My Profile' dialog". After enabling, "the user will be shown a set of 10 recovery codes". "Should a user lose access to their 2FA device and all recovery codes, a superuser can disable 2FA for the user" ([usage.md, Two-factor authentication](https://github.com/paperless-ngx/paperless-ngx/blob/dev/docs/usage.md)).
- **Trusting the provider's second step.** `PAPERLESS_SOCIAL_ACCOUNT_MFA_TRUSTED_AMR` skips the Paperless prompt when the provider's `amr` claim matches, default `mfa`. The docs warn that Pocket ID sends `otp` "for single-factor email codes, which should _not_ be trusted" ([configuration.md](https://github.com/paperless-ngx/paperless-ngx/blob/dev/docs/configuration.md)).
- **Failed attempts.** `PAPERLESS_TOKEN_THROTTLE_RATE` limits the API token endpoint "to mitigate brute-force login attempts", default `5/min`. `PAPERLESS_TRUSTED_PROXIES` "may be needed to prevent IP address spoofing if you are using e.g. fail2ban with log entries for failed authorization attempts" (same page). How the browser login form itself is throttled is not in the docs read.
- **Sessions.** `PAPERLESS_SESSION_COOKIE_AGE` defaults to 1209600 seconds (2 weeks); with `PAPERLESS_ACCOUNT_SESSION_REMEMBER` off, sessions end when the browser closes (same page).

### Jellyfin

- **Lockout.** "Failed login attempts before user is locked out ... 0 means inheriting the default of 3 for non-admin and 5 for admin, -1 disables lockout." Unlocking "is a manual process for the Jellyfin administrator. When a user is locked-out a message of the lockout appears on the activity feed on the administrator dashboard" ([Adding and managing users](https://github.com/jellyfin/jellyfin.org/blob/master/docs/general/server/users/adding-managing-users.md)).
- **Disabling.** "Disable this user ... existing connections will be abruptly terminated" (same page).
- **fail2ban.** A jail with `maxretry = 3`, `bantime = 86400`, `findtime = 43200` and the filter `Authentication request for .* has been denied \(IP: "<ADDR>"\)`. It needs log level `Info` because "failed authentication entries are not logged at `Error`" ([fail2ban](https://github.com/jellyfin/jellyfin.org/blob/master/docs/general/post-install/networking/9_advanced/fail2ban.md)).
- **No second step** in the docs read.

### Mealie

- **Lockout.** `SECURITY_MAX_LOGIN_ATTEMPTS` default 5, "Maximum times a user can provide an invalid password before their account is locked"; `SECURITY_USER_LOCKOUT_TIME` default 24 hours ([Backend configuration](https://mealie.io/documentation/getting-started/installation/backend-config/)). `reset_locked_users.py` unlocks from the container (sign-in research).
- **Session length.** `TOKEN_TIME` default 48, "The time in hours a login session lasts" (same page).
- **Rate limiting.** "By default, the API is **not** rate limited", and the docs suggest rate limiting at the reverse proxy ([Security considerations](https://mealie.io/documentation/getting-started/installation/security/)).

### Actual Budget

The sync server's account routes use `express-rate-limit` with `windowMs: 15 * 60 * 1000` and `max: 5`, `skipSuccessfulRequests: true`, on `/login`, `/bootstrap` and `/change-password` ([app-account.js](https://github.com/actualbudget/actual/blob/master/packages/sync-server/src/app-account.js)). This is a per-key window that slows guessing rather than a lockout. Which key the library uses behind a proxy depends on Express's `trust proxy` setting, which was not checked (`unchecked`).

### Audiobookshelf

- **Rate limit.** Defaults are `DEFAULT_WINDOW_MS = 10 * 60 * 1000` and `DEFAULT_MAX = 40`, set with `RATE_LIMIT_AUTH_WINDOW` and `RATE_LIMIT_AUTH_MAX`; `RATE_LIMIT_AUTH_MAX=0` disables it. The key is the client address, read with a helper that looks at forwarding headers. A hit is logged as `Rate limit exceeded - IP: ..., Endpoint: ..., User-Agent: ...` ([rateLimiterFactory.js](https://github.com/advplyr/audiobookshelf/blob/master/server/utils/rateLimiterFactory.js)). The limiter guards `/login` and `/auth/refresh` ([Auth.js](https://github.com/advplyr/audiobookshelf/blob/master/server/Auth.js)).
- **Failures logged.** Failed logins are logged as `Failed login attempt for username "..." from ip ...` ([LocalAuthStrategy.js](https://github.com/advplyr/audiobookshelf/blob/master/server/auth/LocalAuthStrategy.js)).
- **Sessions.** Access tokens last 1 hour and refresh tokens 30 days by default (`ACCESS_TOKEN_EXPIRY`, `REFRESH_TOKEN_EXPIRY`) ([TokenManager.js](https://github.com/advplyr/audiobookshelf/blob/master/server/auth/TokenManager.js)). `invalidateJwtSessionsForUser` ends all of a user's sessions ([Auth.js](https://github.com/advplyr/audiobookshelf/blob/master/server/Auth.js)).

### Karakeep

`RATE_LIMITING_ENABLED` defaults to `false`: "If set to true, API rate limiting will be enabled" ([Configuration](https://docs.karakeep.app/configuration/environment-variables/)). No second step appears on that page.

### Immich

- **Failed sign-ins.** The login function runs the password check even for unknown emails "so response time is constant regardless of whether the email is registered, preventing timing-based user enumeration". It logs `Failed login attempt for user ${dto.email} from ip address ${details.clientIp}` and answers "Incorrect email or password". No attempt counter is in that function ([auth.service.ts](https://github.com/immich-app/immich/blob/main/server/src/services/auth.service.ts)). Whether a limiter sits elsewhere in the server was not checked (`unchecked`).
- **Sessions.** The web app has "Authorized Devices", "Manage your logged-in devices" and "Log Out All Devices" ([i18n/en.json](https://github.com/immich-app/immich/blob/main/i18n/en.json)). Changing a password carries an `invalidateSessions` option (auth.service.ts). The admin reset command asks "Invalidate existing sessions?" ([server-commands.md](https://github.com/immich-app/immich/blob/main/docs/docs/administration/server-commands.md)).
- **Second step.** None in the docs read. A PIN code protects the locked folder (`setupPinCode` in auth.service.ts) and is not part of sign-in.

### Patterns across the eight

1. **A second step is the exception.** Two of eight offer it (Nextcloud, Paperless-ngx), both optional by default, both with ten single-use recovery codes, and Nextcloud lets the admin require it for everyone or for chosen groups. Apps that support an outside provider lean on it instead (Paperless-ngx can trust the provider's second step).
2. **Slowing down outnumbers locking out.** Nextcloud, Actual Budget and Audiobookshelf slow or refuse per address or per window and recover by themselves. Jellyfin and Mealie disable the account, and Jellyfin needs the admin to unlock it. Nextcloud's docs give the reason for the per-address design: it avoids locking out the real person.
3. **The reverse proxy problem is universal.** Every app that limits or logs by address documents a trusted-proxy setting (Nextcloud, Paperless-ngx), because behind a proxy all traffic otherwise appears to come from one address.
4. **A log line for fail2ban is the documented way to block at the network level** (Nextcloud, Jellyfin; Paperless-ngx mentions it), and its exact wording becomes a compatibility promise.
5. **Session lengths are long.** 24 hours idle with 15-day "remember me" (Nextcloud), 2 weeks (Paperless-ngx), 48 hours (Mealie), 30 days of refresh (Audiobookshelf). All are longer than NIST's AAL2 numbers and within or near AAL1's 30 days.
6. **A device list with "sign out everywhere" is common** (Nextcloud, Immich), and so is ending sessions after a password change (Immich).
7. **Admin alerts are rare.** Only Jellyfin's activity feed surfaced a lockout to the admin in the docs read.

## Rust building blocks

Versions and dates from the crates.io API on 2026-11-11. Repository activity from the GitHub API on the same day. Crates covered by the sign-in research (`argon2`, `password-hash`, `webauthn-rs`, `openidconnect`, `tower-sessions`, `axum-login`, `tower-sessions-file-store`) are not repeated except where a new fact matters.

| Need | Crate | Version | Released | Maintenance signals |
|---|---|---|---|---|
| Authenticator-app codes (TOTP) | [`totp-rs`](https://crates.io/crates/totp-rs) | 6.0.0 | 2026-08-06 | Repository pushed 2026-10-30, 0 open issues; MSRV 1.88; optional features `otpauth` (the setup link), `qr` (a PNG QR code), `gen_secret`, `zeroize`; configurable time skew; warns that "some authenticator apps will accept the `SHA256` and `SHA512` algorithms but silently fallback to `SHA1`" ([README](https://github.com/constantoine/totp-rs)) |
| Rate limiting as tower middleware | [`tower_governor`](https://crates.io/crates/tower_governor) | 0.8.0 | 2025-08-14 | Built on `governor` 0.10 and axum 0.8 ([Cargo.toml](https://github.com/benwis/tower-governor/blob/main/Cargo.toml)); last push 2025-08-14, 25 open issues; about 15 months without a release |
| Rate limiting core (keyed limiters) | [`governor`](https://crates.io/crates/governor) | 0.10.4 | 2025-12-16 | About 93 million downloads; repository pushed 2026-08-24, 36 open issues |
| Sessions | [`tower-sessions`](https://crates.io/crates/tower-sessions) | 0.15.0 | 2026-02-01 | Repository pushed 2026-10-26, 5 open issues; uses `tower-cookies` 0.11, with optional `signed` and `private` cookie features ([Cargo.toml](https://github.com/maxcountryman/tower-sessions/blob/main/Cargo.toml)); stores are pluggable, including a file store and a SQLite store ([README](https://github.com/maxcountryman/tower-sessions)) |
| Cookies without a session framework | [`axum-extra`](https://crates.io/crates/axum-extra) | 0.12.6 | 2026-04-14 | Part of the axum project; MSRV 1.80 |
| Cookie parsing and building | [`cookie`](https://crates.io/crates/cookie) | 0.18.2 | 2026-08-08 | Already in Hatchdoor's dependency tree at 0.18.2 (`Cargo.lock`) |
| Security headers, timeouts, body limits | [`tower-http`](https://crates.io/crates/tower-http) | 0.7.1 | 2026-08-31 | Already a dependency (features `fs`, `trace`); it has `set-header`, `timeout`, `limit` and `sensitive-headers` features (`Cargo.toml` of tower-http 0.7.1), so headers need no new crate |
| Cross-site request forgery tokens | [`axum_csrf`](https://crates.io/crates/axum_csrf) | 0.12.0 | 2026-10-22 | About 258,000 downloads; optional, since `SameSite` plus the existing `Origin` and `Sec-Fetch-Site` check covers ASVS 3.5 without tokens |
| Password strength feedback | [`zxcvbn`](https://crates.io/crates/zxcvbn) | 3.1.1 | 2026-04-24 | MSRV 1.66; optional, since NIST and ASVS ask for a blocklist and length rules, not a strength score |

Notes on fit:

- **Rate limiting may need no crate.** `src/mcp/limits.rs` already keeps per-key rolling windows (`RateLimiter::check_quota_units`). A per-account and per-address failure counter with growing delays is the same shape. ADR-13 asks that a new dependency earn its place.
- **Per-address limiting needs the address first.** `tower_governor`'s default key is the peer address and requires the server to be built with `into_make_service_with_connect_info::<SocketAddr>` ([README](https://github.com/benwis/tower-governor)); Hatchdoor does not do that today. Its `SmartIpKeyExtractor` "looks for common IP identification headers usually provided by reverse proxies in order (x-forwarded-for, x-real-ip, forwarded)" (same README). Those headers can be sent by any client unless a proxy overwrites them, so reading them safely needs a trusted-proxy rule. That is the same lesson as Nextcloud's `trusted_proxies` and Paperless-ngx's `PAPERLESS_TRUSTED_PROXIES`, and it means a new ADR next to ADR-34.
- **TOTP is small.** `totp-rs` covers code generation and checking. Hatchdoor itself would add the stored secret per person, the "used once" record (ASVS 6.5.1, NIST "only once while it is valid"), recovery codes stored hashed (the `argon2` crate already chosen for passwords serves), and the throttle.
- **Headers are a single layer.** `tower-http`'s `SetResponseHeaderLayer` behind the `set-header` feature, one line per header, on the router built in `src/server.rs`. The Content Security Policy needs testing against the PWA's own needs (KaTeX styles, Mermaid SVG, the service worker); writing the exact policy is build work, not a decision.

## What this means for the decision

The items below are the choices the operator makes in [#725](https://github.com/BatterWorks/Hatchdoor/issues/725), with what each standard and comparable app says. They are not ranked and none is made here.

### Choices for #725 "What protects an instance exposed to the internet"

**1. Who must use a second sign-in step.**

| Option | Standards | Comparable apps | Cost to the household |
|---|---|---|---|
| A. Not offered | Meets ASVS Level 1 and NIST AAL1 only; ASVS Level 2 needs a "fully documented rationale and a comprehensive set of mitigating controls" (6.3.3) | Six of eight | None |
| B. Optional for everyone | NIST AAL1 says verifiers "SHOULD make multi-factor authentication options available" | Nextcloud and Paperless-ngx by default | Only those who opt in |
| C. Required for the admin, optional for others | Partly meets ASVS 8.4.2's layered protection for admin interfaces (L3); still a 6.3.3 exception for the others | Nextcloud can require it for a chosen group | The admin sets up an authenticator app |
| D. Required for everyone | ASVS Level 2 as written; NIST AAL2 | Nextcloud's "system-wide" enforcement | Everyone sets up an authenticator app or a passkey, and keeps recovery codes |

Points that cut across all four:
- The password minimum follows from this choice: NIST requires 15 characters when a password can be used alone, 8 when a second step always follows.
- With an outside provider (#718), the provider can do the second step, and Hatchdoor checks the `amr` claim (ASVS 6.8.4; Paperless-ngx's pattern).
- The admin's recovery subcommand must not quietly skip a required second step (ASVS 6.4.3); it either resets it too or asks.

**2. Which second step, if any.**
- **TOTP with recovery codes.** Works on any address, plain HTTP included, and with every phone authenticator app. Not phishing-resistant (NIST). Smallest to build (`totp-rs`).
- **Passkeys.** Phishing-resistant, but need HTTPS on one domain (sign-in research) and are the largest build (`webauthn-rs`).
- **Both.** NIST AAL2 wants at least one phishing-resistant option offered.
- **Delegate to an outside provider** when one is configured.

**3. What happens after repeated failed sign-ins.**
- **Slow down per address and per account, recover automatically.** Nextcloud's 25-second ceiling and `429` after 10 failures in 30 minutes; Actual's 5 per 15 minutes; Audiobookshelf's 40 per 10 minutes; NIST's "30 seconds up to an hour". An attacker cannot lock the household out. It needs a trustworthy client address behind the proxy.
- **Lock the account until the admin or a timer releases it.** Jellyfin: 3, or 5 for admins, manual unlock. Mealie: 5, 24 hours. It is simple and stops guessing on that account completely, but anyone who knows a username can lock that person out, which ASVS 6.1.1 asks to prevent. If the admin is locked out, only the recovery subcommand helps.
- **A hard ceiling regardless.** NIST's upper bound is 100 consecutive failures per account before the factor is disabled.
- **Agent keys too.** Today failed MCP token checks are not counted. A per-address limit on failed key checks closes that, whichever option is chosen for people.

**4. How long a sign-in lasts.**
- NIST AAL2: at most 24 hours overall and 1 hour idle. AAL1: at most 30 days overall.
- Comparable apps: 24 hours idle with a 15-day "remember me" (Nextcloud); 2 weeks (Paperless-ngx); 48 hours (Mealie); 1-hour access with 30-day refresh (Audiobookshelf).
- A phone PWA opened a few times a week argues for long sessions; ASVS 7.1.1 asks that a deviation from NIST be written down with its reason.
- Linked choices: whether "remember this device" exists, whether a sensitive change (password, second step, agent keys) asks again (ASVS 7.5.1), and where sessions are stored. In memory, a restart signs everyone out; in a file or SQLite, they survive (sign-in research; ADR-01 allows SQLite only for disposable data).

**5. Signing out everywhere and ending sessions.** Required at Level 1 for disabled accounts (7.4.2) and at Level 2 for "sign out everywhere" and admin control (7.4.3, 7.4.5, 7.5.2). The choice is how much of a device list to show: just a "sign out everywhere" button, or a list with each device (Nextcloud, Immich). Agent keys are a separate list (#719).

**6. How the admin hears about trouble.**
- **Log lines only**, in a stable documented format that fail2ban or CrowdSec can read (Nextcloud, Jellyfin). Smallest; the operator wires the tool. The exact wording becomes a compatibility promise.
- **An admin screen of recent security events** (failed sign-ins, lockouts, new agent keys, second-step changes), like Jellyfin's activity feed. Shown when the admin next opens Hatchdoor.
- **An outbound alert** (a webhook or a push service). Hatchdoor has no outbound notification today, and ADR-39 and ADR-45 set the precedent that Hatchdoor contacts outside services only when asked. Not found in any of the eight apps' docs read for security events.
- **A notice to the person**, in the app at next sign-in, after their password, second step or keys change (ASVS 6.3.7, NIST account notifications).

**7. Whether to trust the proxy's client address.** Per-address limits and useful log lines both need the caller's real address, which behind a proxy arrives in `X-Forwarded-For` or `Forwarded`. ADR-34 says these headers affect nothing but a transfer link's address, and "Any future change that lets these headers affect anything other than a transfer link's address ... breaks this decision". A trusted-proxy setting (an address list, as in Nextcloud and Paperless-ngx) needs a new ADR. Without it, Hatchdoor can still limit per account, and the proxy can do per-address limits itself.

**8. What Hatchdoor does itself versus what the operator's proxy does.** TLS and HSTS (ASVS 12.x, 3.4.1), request timeouts and general rate limiting can live in the proxy. Mealie's docs hand rate limiting to the proxy. Sign-in throttling, sessions, access checks and security log lines cannot, because only Hatchdoor knows who is signing in. The choice is where the line falls and whether Hatchdoor refuses to start multi-user without signs of a proxy (an `https://` public address, for example), in the spirit of ADR-07's fail-fast posture.

**9. Browser security headers.** Little to choose: ASVS 3.4.3 to 3.4.6 name them. The choice is only whether they apply in every mode or only to multi-user instances. A Content Security Policy also limits the damage of any future rendering bug once notes are shared between people.

### What affects #723 "Does a shared instance need HTTPS on a domain name?"

- **Cookies.** Session cookies that meet ASVS 3.3.1 and NIST must be `Secure`, and "Insecure sites (`http:`) cannot set cookies with the `Secure` attribute", except on localhost ([MDN, Set-Cookie](https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Set-Cookie)). A multi-user instance on a plain-HTTP LAN address would need either an exception to the cookie rules or to keep header tokens in the browser, which brings back local storage and the `access_token` query parameter that NIST advises against.
- **HSTS** exists only over HTTPS (RFC 6797, 7.2).
- **Passkeys** need HTTPS on one domain (sign-in research); TOTP does not.
- **ASVS 12.2.1 and 12.2.2** [L1] require TLS with a publicly trusted certificate for an internet-facing service. For an instance reachable from the internet, the standard leaves no plain-HTTP option; the open question is only the LAN-only case.

### What affects #718 "How people sign in"

- **Second step and outside provider.** If an outside provider is offered, ASVS 6.8.4 asks Hatchdoor to check the provider's `amr` before trusting its second step, and to assume a single factor when the claim is missing. The Paperless-ngx docs show that one popular provider reports a single-factor email code as `otp`.
- **Password length depends on #725's second-step choice:** 15 characters minimum when passwords stand alone, 8 when a second step always follows (NIST).
- **Recovery.** The recovery subcommand from the sign-in research must handle the second step (ASVS 6.4.3), and a temporary password from the admin must expire and be changed at first use (ASVS 6.4.1).
- **No default accounts.** The first admin cannot be a fixed name with a known password (ASVS 6.3.2).
- **Every way in documented and equally strong.** Password, provider, proxy header if offered, recovery command, agent keys (ASVS 6.1.3, 6.3.4). A proxy-header mode in particular bypasses every throttle and second step that Hatchdoor implements.

## Sources not read directly, and unchecked points

- **Paperless-ngx browser login throttling.** Only the API token endpoint's throttle is documented in the pages read.
- **Actual Budget behind a proxy.** Which address the rate limiter keys on depends on Express's `trust proxy` setting, not checked (`unchecked`).
- **Immich.** Whether any limiter exists outside the login function was not checked (`unchecked`).
- **Admin alerts.** Notifications to the admin for security events are absent from the docs read for seven of the eight apps; that is not proof they are absent.
- **Second steps.** "Not in the docs read" for six apps means the configuration and user pages cited do not describe one.
- **The ASVS "bleeding edge" build** of 2026-10-26 on the main branch was not compared with 5.0.0.
