# Sign-in options for a self-hosted household app

> Research record.
>
> Researched: 2026-11-11
> Resolves: research ticket [#716](https://github.com/BatterWorks/Hatchdoor/issues/716), part of map [#714](https://github.com/BatterWorks/Hatchdoor/issues/714)
> Feeds: [#718](https://github.com/BatterWorks/Hatchdoor/issues/718) "How people sign in"
> Investigated against: `development` at `b50c24a`
> Status: Decision input. It compares options and does not choose one.

## Summary in plain words

Hatchdoor 3.0 lets several people who know each other share one instance. Each person needs a way to prove who they are. There are four common ways to do that, and they differ mostly in who holds the secret and who has to run extra software.

1. **Passwords kept by Hatchdoor.** Each person has a username and a password, and Hatchdoor stores a scrambled form of the password (a hash) that cannot be turned back into the password. This is what every app we looked at offers. For the household it is the familiar login box, and phones' password managers fill it in. For the operator it costs nothing extra. When someone forgets their password, the admin sets a new one, and when the admin forgets their own, the operator runs a command on the server.

2. **Passkeys.** Instead of a password, the phone or laptop holds a secret key and unlocks it with a fingerprint, face or PIN. It cannot be phished or guessed. For the household it is "tap and look at the phone". For the operator it has one hard requirement: Hatchdoor must be reached through a real domain name over HTTPS. A passkey cannot be made for an address like `192.168.1.20`, and changing the domain later strands every passkey made for the old one. Losing the only device that holds a passkey needs a way back in, such as a one-time sign-in link from the admin. Only two of the eight apps we looked at offer passkeys, one of them only since 2026-11-10.

3. **An outside identity provider over OpenID Connect (OIDC).** The household signs in at a separate sign-in app (Authelia, Authentik, Pocket ID, Keycloak and similar) that the operator runs, and that app vouches for them to Hatchdoor. One sign-in then covers every app in the house that is connected to it, and the provider can offer passkeys so Hatchdoor does not have to. For the operator it means running and maintaining a second service, registering Hatchdoor in it and copying a client ID and secret into Hatchdoor's settings. Forgotten passwords are the provider's problem, but when the provider is down or misconfigured nobody can sign in, so apps that offer it keep a local way in for the admin.

4. **Trusting a header from a reverse proxy (forward auth).** A web server in front of Hatchdoor checks who you are and passes the name on in a request header. Hatchdoor only reads the name. It is the least code, but it is only safe when nothing can reach Hatchdoor except through that proxy: anyone who reaches it directly can type any name into the header and become that person, the admin included. Every app that offers it says so in a warning. Agents cannot do the proxy's interactive sign-in, so the operator has to exempt the agent endpoint from it.

What the comparable apps do: all eight have a sign-in of their own (for Actual Budget it is one password shared by everyone); six offer OIDC as an option next to passwords, and one (Actual Budget) requires it as soon as there is more than one person; three accept a proxy header; two offer passkeys. None of them relies on email to recover a forgotten admin password: most ship a command to run inside the container, Jellyfin writes a file to read on the server, and Karakeep falls back to editing the database.

## Comparison table

| | Passwords kept by Hatchdoor | Passkeys | Outside identity provider (OIDC) | Proxy header (forward auth) |
|---|---|---|---|---|
| **What a household member does** | Types a username and password, or lets the phone fill them in | Creates a passkey once, then unlocks with face, fingerprint or PIN | Taps "Sign in with ...", signs in at the provider's page, comes back | Signs in at the proxy's page; Hatchdoor shows no login of its own |
| **What the operator sets up** | Nothing beyond adding people; a first admin from an environment variable or a command | A domain name with HTTPS, and keeping that domain for good | A second service, a client registration with a redirect address, a client ID and secret in Hatchdoor's settings | A proxy plus an authentication server, header stripping, Hatchdoor reachable only through the proxy, an exemption for the agent endpoint |
| **Forgotten password, no email server** | Admin sets a new one; the admin's own reset is a command run in the container | Lost device: another passkey on a second device, or a one-time sign-in link from the admin | Handled by the provider; Hatchdoor still needs a local way in when the provider is down | Handled by the proxy's authentication server |
| **Installed phone app (PWA)** | Works; stays inside the app | Expected to work, not verified for iOS Home Screen apps (`unchecked`) | The sign-in page leaves the app's scope and shows browser controls, then returns | Works while the proxy's session lasts; an expired proxy session must be handled by the app |
| **Rust crates** | `argon2` 0.6.0, `password-hash` 0.6.1 | `webauthn-rs` 0.6.2 | `openidconnect` 4.1.0 on `oauth2` 5.0.0 | None needed |
| **Rough build size** | Smallest of the real options | Largest: two ceremonies, a credential list per person, a fallback | Medium: one redirect flow, claim mapping, an escape hatch | Smallest code, largest safety burden on the operator |
| **Works on a plain-HTTP LAN address** | Yes (as today's token does) | No | Depends on the provider; most expect HTTPS | Yes, but the header can be forged by anyone on the LAN who reaches Hatchdoor directly |

## How comparable apps do it

Eight self-hosted apps with good official documentation. "Not in the docs read" means the pages cited did not mention it; it is not proof the feature is absent.

| App | Own passwords | Passkeys | OIDC | Proxy header | Getting back in without email |
|---|---|---|---|---|---|
| Immich | Yes, can be switched off | Yes, web only, new in v3.4.0 | Optional | Not in the docs read | Admin resets a user to a random temporary password; `immich-admin reset-admin-password` in the container |
| Paperless-ngx | Yes, with an optional second factor | Not in the docs read | Optional | Yes, with a warning | Email reset needs SMTP; `createsuperuser` in the container |
| Jellyfin | Yes, plus Quick Connect codes | Not in the docs read | Not built in; LDAP by plugin | Not in the docs read | "Forgot password" from the home network writes a PIN to a file on the server |
| Mealie | Yes | Not in the docs read | Optional, plus LDAP | Yes, with a shared secret | `change_password.py` in the container |
| Actual Budget | One shared server password | Not in the docs read | Required for more than one person | Yes, carrying the server password | `reset-password.js` in the container |
| Audiobookshelf | Yes | Not in the docs read | Optional | Not in the docs read | No path found in the docs read (`unchecked`) |
| Karakeep | Yes, can be switched off | Not in the docs read | Optional | Not in the docs read | Admin resets a user's password; the admin's own needs SMTP or a database edit |
| Nextcloud | Yes | Yes, built in and on by default | Optional app | Not in the docs read | `occ user:resetpassword` |

### Immich

- Passwords and OAuth/OIDC sit side by side. The admin can turn password login off for the whole instance, and if OIDC is also off "no users will be able to login using any method"; the docs point to the server command to turn it back on ([System settings, Password authentication](https://docs.immich.app/administration/system-settings/)).
- OIDC options include **Auto Register** (create an account the first time someone signs in, on by default), **Auto Launch** (skip Immich's login page and go straight to the provider, with `/auth/login?autoLaunch=0` as the way back to the login page), and a role claim that makes someone an admin when the provider says `admin` ([OAuth authentication](https://docs.immich.app/administration/oauth/)).
- The native mobile app needs its own redirect address, `app.immich:///oauth-callback`, registered at the provider ([OAuth authentication, Prerequisites](https://docs.immich.app/administration/oauth/)). A PWA has no such custom address and returns to its own HTTPS origin instead.
- Recovery: the admin resets a user from **Administration > Users**; "the user's password will be reset to a random password and they have to change it next time they sign in" ([User management, Password reset](https://docs.immich.app/administration/user-management/)). The admin's own password is reset with `immich-admin reset-admin-password` run in the server container, which also offers to sign out existing sessions; the same tool has `enable-password-login`, `disable-oauth-login`, `grant-admin` and `list-users` ([Server commands](https://docs.immich.app/administration/server-commands/)).
- Passkeys: the pull request "feat: passkeys" (merged 2026-10-20) adds passkeys "on web", with the relying-party domain taken from the request's origin by default or set in config, related-origin support for up to five extra domains, and mobile "not implemented" ([immich#32194](https://github.com/immich-app/immich/pull/32194)). It is listed in the v3.4.0-rc.1 release notes and its code is in the v3.4.0 tag released 2026-11-10, but the final v3.4.0 notes and the documentation do not mention it yet ([v3.4.0-rc.1](https://github.com/immich-app/immich/releases/tag/v3.4.0-rc.1), [v3.4.0](https://github.com/immich-app/immich/releases/tag/v3.4.0)).

### Paperless-ngx

The published docs site refused automated reading (a Cloudflare challenge), so these come from the same pages' source in the project repository.

- Social and OIDC login come through django-allauth, configured with `PAPERLESS_SOCIALACCOUNT_PROVIDERS`; `PAPERLESS_DISABLE_REGULAR_LOGIN` hides the password form once SSO works, but "does not disable the Django admin login nor logging in with local credentials via the API"; `PAPERLESS_REDIRECT_LOGIN_TO_SSO` sends people straight to the provider ([configuration.md](https://github.com/paperless-ngx/paperless-ngx/blob/dev/docs/configuration.md)).
- Group sync from the provider has a lockout warning: if the admin group claim is missing, "the logged-in user will immediately lose their own superuser access", and the way back is `manage.py createsuperuser` ([configuration.md](https://github.com/paperless-ngx/paperless-ngx/blob/dev/docs/configuration.md)).
- The proxy header mode, `PAPERLESS_ENABLE_HTTP_REMOTE_USER`, "will allow authentication by simply adding a `Remote-User: <username>` header to a request. Use with care!", and the operator "*must* ensure that any such header is not passed from external requests". It is off by default, and the API version is a separate switch so as not to open existing setups by surprise ([configuration.md](https://github.com/paperless-ngx/paperless-ngx/blob/dev/docs/configuration.md)).
- `PAPERLESS_ADMIN_USER` creates the first admin from an environment variable, for setups where running `createsuperuser` by hand is awkward ([configuration.md](https://github.com/paperless-ngx/paperless-ngx/blob/dev/docs/configuration.md)).
- Password reset by email needs an SMTP server ([configuration.md, Email sending](https://github.com/paperless-ngx/paperless-ngx/blob/dev/docs/configuration.md)).
- It has its own second factor, and `PAPERLESS_SOCIAL_ACCOUNT_MFA_TRUSTED_AMR` skips it when the OIDC provider already did multi-factor sign-in; the docs note that Pocket ID reports `phr` for passkey logins but `otp` for single-factor email codes ([configuration.md](https://github.com/paperless-ngx/paperless-ngx/blob/dev/docs/configuration.md)).

### Jellyfin

- Local accounts with passwords. The only other built-in provider choice is LDAP through the LDAP-Auth plugin ([Adding and managing users](https://github.com/jellyfin/jellyfin.org/blob/master/docs/general/server/users/adding-managing-users.md), [Plugins](https://github.com/jellyfin/jellyfin.org/blob/master/docs/general/server/plugins/index.mdx)).
- **Quick Connect** signs a new device in "without entering a username or password": the new device shows a code and an already signed-in device approves it ([Quick Connect](https://jellyfin.org/docs/general/server/quick-connect/)). It is a useful pattern for a household TV or a relative's tablet.
- Forgotten password without email: when the request comes from the local network, the server writes a PIN into a `passwordreset*.json` file in its data folder, valid 30 minutes, and redeeming the PIN sets it as the new password; from outside the network no file is written and a warning is logged ([DefaultPasswordResetProvider.cs](https://github.com/jellyfin/jellyfin/blob/master/Jellyfin.Server.Implementations/Users/DefaultPasswordResetProvider.cs), [UserController.cs](https://github.com/jellyfin/jellyfin/blob/master/Jellyfin.Api/Controllers/UserController.cs)). Whoever can read the server's files can therefore reset any password. When that fails, the docs fall back to editing the database by hand ([Troubleshooting, Unlock locked user account](https://github.com/jellyfin/jellyfin.org/blob/master/docs/general/administration/troubleshooting.md)).

### Mealie

- Password login, LDAP and OIDC can all be on at once; `ALLOW_PASSWORD_LOGIN` hides the password fields, `OIDC_SIGNUP_ENABLED` controls account creation on first sign-in, and `OIDC_AUTO_REDIRECT` skips the login page with `?direct=1` as the way back ([Backend configuration](https://mealie.io/documentation/getting-started/installation/backend-config/)).
- Behind a TLS proxy the server must trust the proxy's forwarded headers "otherwise ... the generated OIDC redirect URI will use the wrong scheme (http instead of https)" ([OIDC](https://mealie.io/documentation/getting-started/authentication/oidc/)). Hatchdoor already reads these headers for transfer links (ADR-34).
- **Reverse proxy auth** is the most careful version found: Mealie "trusts the header's value completely", so the proxy must overwrite the header, Mealie must not be otherwise reachable, and `REVERSE_PROXY_AUTH_SECRET` adds a shared secret the proxy sends in a second header, because "checking the connecting IP isn't reliable enough for this" ([Backend configuration, Reverse proxy auth](https://mealie.io/documentation/getting-started/installation/backend-config/)).
- Recovery: `change_password.py`, `make_admin.py` and `reset_locked_users.py` run inside the container; the password script also offers to switch an OIDC-only user back to local login ([FAQ, Account management](https://mealie.io/documentation/getting-started/faq/)).

### Actual Budget

- Out of the box there is one server password for everyone. "If you need support for multiple users, you must enable" OpenID ([Authenticating with an OpenID provider](https://github.com/actualbudget/docs/blob/master/docs/config/oauth-auth.md)); the multi-user page repeats that it "requires you to have set up an OpenID Provider" ([Multi-user](https://github.com/actualbudget/docs/blob/master/docs/config/multi-user.md)). This is the one app where an outside provider is required rather than optional.
- Header auth sets `x-actual-password` to the server password, with a warning that "a misconfiguration ... could make your instance available to the whole internet", and an optional list of trusted proxies ([HTTP header auth](https://github.com/actualbudget/docs/blob/master/docs/advanced/http-header-auth.md)).
- Recovery: `node /app/src/scripts/reset-password.js` run in the container ([Reset password](https://github.com/actualbudget/docs/blob/master/docs/troubleshooting/reset_password.md)).

### Audiobookshelf

- OIDC with auto-discovery, **Auto Register**, and a way to bypass automatic OIDC login with `/login/?autoLaunch=0` ([OIDC authentication](https://www.audiobookshelf.org/guides/oidc_authentication/)).
- Native mobile apps use custom redirect addresses (`audiobookshelf://oauth` and several third-party ones) that the server allows on its own list, so they need not be registered at the provider ([OIDC authentication, Mobile app redirect](https://www.audiobookshelf.org/guides/oidc_authentication/)).
- No official page on recovering a lost root password was found; the community answers point at editing the database (`unchecked`).

### Karakeep

- `DISABLE_PASSWORD_AUTH` leaves only OAuth/OIDC; `OAUTH_AUTO_REDIRECT` skips the login page; `DISABLE_SIGNUPS` closes self sign-up; `OAUTH_ALLOW_DANGEROUS_EMAIL_ACCOUNT_LINKING` links an OIDC login to an existing account by email, with "Only enable it if you trust the OAuth provider!" ([Configuration](https://docs.karakeep.app/configuration/environment-variables/)). The name is a useful reminder that matching accounts by email is a trust decision.
- A client secret is optional: "Leave it unset for a public client, which authenticates with PKCE only" ([Configuration](https://docs.karakeep.app/configuration/environment-variables/)).
- Recovery: "Administrators can reset the password of any user"; an admin who forgot their own password uses the email link if SMTP is set up, "otherwise, you have to reset the password in the database" ([FAQ, Lost password](https://docs.karakeep.app/administration/FAQ/)).

### Nextcloud

- Passkey sign-in without a password is built in: `auth.webauthn.enabled` defaults to on, "but it can be explicitly disabled by admins" ([config.sample.php parameters](https://github.com/nextcloud/documentation/blob/master/admin_manual/configuration_server/config_sample_php_parameters.rst)). WebAuthn is also offered as a second factor, and the user manual warns not to use the same key for both, "as this would mean you are again only using a single factor" ([Two-factor authentication](https://github.com/nextcloud/documentation/blob/master/user_manual/user_2fa.rst)).
- OIDC comes from the optional `user_oidc` app, which can create users on first sign-in or only authenticate existing ones ([OIDC user authentication](https://github.com/nextcloud/documentation/blob/master/admin_manual/configuration_user/user_auth_oidc.rst)).
- Client apps sign in with app passwords rather than the account password ([Authentication](https://docs.nextcloud.com/server/stable/admin_manual/configuration_user/authentication.html)), the same split Hatchdoor's map plans between a person and their agent keys.
- Recovery: `occ user:resetpassword` resets "any user's password, including administrators" ([occ user commands](https://github.com/nextcloud/documentation/blob/master/admin_manual/occ_users.rst)).

### Patterns across the eight

1. **Passwords are the floor.** Every app keeps its own sign-in, even those that support a provider; several let the admin hide it once a provider works, and all of those keep an escape hatch (a URL parameter, or a command that turns password login back on).
2. **OIDC is an option, rarely a requirement.** Six offer it next to passwords. Only Actual Budget makes it mandatory, and only for its multi-user mode. The common switches are the same everywhere: create accounts on first sign-in or not, skip the app's login page or not, map a claim to the admin role or not.
3. **A command in the container is the universal recovery.** Immich, Paperless-ngx, Mealie, Actual Budget and Nextcloud all reset the admin with a command run through the container runtime; Jellyfin uses a file on the server; Karakeep falls back to editing the database. None depends on email.
4. **Admin resets for everyone else**, usually to a temporary password the person must change (Immich).
5. **Proxy-header trust always comes with a warning**, and the stronger implementations add a check that the request really came from the proxy: a trusted-proxy list (Actual Budget, and Authelia's recommendation below) or a shared secret (Mealie).
6. **Passkeys are still rare** in this class of app: Nextcloud has them, Immich just added them for the web, and Pocket ID shows the other route, where the provider holds the passkeys and the apps never see one.

## Method by method

### Passwords kept by Hatchdoor

**How it works.** Hatchdoor stores, per person, a password hash in a format that records its own algorithm and settings. OWASP recommends "Argon2id with a minimum configuration of 19 MiB of memory, an iteration count of 2, and 1 degree of parallelism" ([OWASP Password Storage Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html)). The `argon2` crate's defaults are exactly that: `DEFAULT_M_COST = 19 * 1024`, `DEFAULT_T_COST = 2`, `DEFAULT_P_COST = 1` ([argon2/src/params.rs](https://github.com/RustCrypto/password-hashes/blob/master/argon2/src/params.rs)).

**Where the accounts live.** ADR-01 makes SQLite a disposable cache, so people, password hashes and grants cannot live only there. The existing pattern is a JSON file in the state directory, as the Vault registry already is (`HATCHDOOR_VAULT_REGISTRY_PATH`, `AGENTS.md`). A hash in that file is safe to back up with the rest of the state.

**First admin.** ADR-14 says no step needed to reach a working state may be reachable only through the browser. Paperless-ngx's `PAPERLESS_ADMIN_USER` is the environment-variable version of a first admin; Immich registers its first admin through a "Getting Started" button in the browser, a browser-only first step that ADR-14 would not allow in Hatchdoor ([User management, Register the admin user](https://docs.immich.app/administration/user-management/)).

**Household member.** Types a username and password. The phone's password manager offers to save and fill them, inside the installed app as in the browser.

**Operator.** Adds people; nothing else to run.

**Forgotten password, no email.**
- Another person: the admin sets a temporary password, shown once, and the person changes it at next sign-in (the Immich pattern).
- The admin: a subcommand of the binary, run with `docker exec <container> /path/to/hatchdoor <subcommand>`. A distroless image has no shell, but `docker exec` can start the binary directly, exactly as the `--healthcheck` subcommand already runs (ADR-12, `src/main.rs`). The other apps' instructions open a shell first (`docker exec -it mealie bash`), which Hatchdoor's image cannot do; the command line has to call the binary by its path.
- A design point for #718 or later: the subcommand and the running server both touch the account file. Either the server notices the file change, or the subcommand asks the running server over loopback, as `--healthcheck` does.

**Installed phone app.** No difference from the browser.

**Rust building blocks.**

| Crate | Version | Released | Maintenance signals |
|---|---|---|---|
| [`argon2`](https://crates.io/crates/argon2) | 0.6.0 | 2026-08-27 | RustCrypto project; the previous release, 0.5.3, was 2024-01-20; MSRV 1.87 ([Cargo.toml](https://github.com/RustCrypto/password-hashes/blob/master/argon2/Cargo.toml)); repository has 21 open issues and pull requests |
| [`password-hash`](https://crates.io/crates/password-hash) | 0.6.1 | 2026-04-08 | RustCrypto traits for the self-describing hash string; 0.6.0 was yanked |

Both are pure Rust and add no system library to the container. The repository pins Rust 1.97.1 (`rust-toolchain.toml`), above every MSRV in this record.

**Weak points.** Guessing and reused passwords. Limiting repeated attempts is listed on the map under "Security details", not here.

### Passkeys (WebAuthn)

**How it works.** The person's device makes a key pair for Hatchdoor, keeps the private half, and proves possession by signing a fresh challenge. The WebAuthn specification ties every credential to a relying-party ID, which "is a valid domain string"; "a public key credential can only be used for authentication with the same entity (as identified by RP ID) it was registered with", and the origin must be `https`, or `http` on `localhost` ([WebAuthn Level 3, Terminology, RP ID](https://www.w3.org/TR/webauthn-3/)). An IP address is refused outright: "Only the domain format of host is allowed here" ([WebAuthn Level 3, 5.1.3 step 7](https://www.w3.org/TR/webauthn-3/)).

**What that means for Hatchdoor installs.**
- The documented LAN setup, where devices open Hatchdoor at an IP address and port over plain HTTP ("Optional: expose Hatchdoor to your LAN" in the install guide), cannot use passkeys at all.
- A passkey made under one domain does not work under another. Moving an instance to a new domain strands every passkey unless the related-origins mechanism is set up ([WebAuthn Level 3, 5.11](https://www.w3.org/TR/webauthn-3/)); Immich exposes that mechanism as `config.passkey.additionalDomains` ([immich#32194](https://github.com/immich-app/immich/pull/32194)).
- An installed PWA already needs HTTPS, because service workers "are only available in secure contexts" ([MDN, Service Worker API](https://developer.mozilla.org/en-US/docs/Web/API/Service_Worker_API)). So the households that use the phone app already meet the HTTPS half of the requirement; the domain half still matters.

**Household member.** Creates a passkey once while signed in (or from an invitation), then signs in by unlocking the phone. Passkeys saved in iCloud Keychain or Google Password Manager follow the person to their other devices. On iOS the unlock is the device's passcode, Touch ID or Face ID ([passkeys.dev, iOS](https://passkeys.dev/docs/reference/ios/)).

**Operator.** A stable domain name with HTTPS, and Hatchdoor told what that domain is (Hatchdoor already has `HATCHDOOR_PUBLIC_URL`).

**Lost device, no email.**
- A second passkey on another device, or a synced passkey, avoids the problem; Pocket ID's docs say the same ("If they still have a passkey on another device, they don't need a code").
- Otherwise the admin issues a one-time sign-in link and the person adds a new passkey. Pocket ID does exactly this from its admin screen, and for the admin from the server: `pocket-id one-time-access-token <username>` "prints a link that signs the user in once within the next hour" ([Pocket ID, Account recovery](https://pocket-id.org/docs/troubleshooting/account-recovery)).
- Or a password stays as a fallback, which keeps the password's weak points.

**Installed phone app.**
- WebAuthn is supported in Safari on iOS and in Android WebView ([MDN browser-compat data via OpenPWA](https://openpwa.net/compatibility/webauthn/)), and Immich's new passkeys are web only ([immich#32194](https://github.com/immich-app/immich/pull/32194)). No first-party source was found that states passkeys work inside an iOS Home Screen web app or an Android installed PWA: `unchecked`. It needs a test on real phones before passkeys are chosen.

**Rust building blocks.**

| Crate | Version | Released | Maintenance signals |
|---|---|---|---|
| [`webauthn-rs`](https://crates.io/crates/webauthn-rs) | 0.6.2 | 2026-10-31 | Kanidm project; regular releases since 0.5.0 (2024-04-28); "has passed a security audit performed by SUSE product security"; since 0.6.0 uses RustCrypto "instead of OpenSSL" ([README](https://github.com/kanidm/webauthn-rs)); MSRV 1.89 ([workspace Cargo.toml](https://github.com/kanidm/webauthn-rs/blob/master/Cargo.toml)); 29 open issues and pull requests |

Things the crate decides for you, from its README and docs ([README](https://github.com/kanidm/webauthn-rs), [docs.rs](https://docs.rs/webauthn-rs/0.6.2/webauthn_rs/)):
- It "requires user verification on login by default, and actually checks the response".
- The state between "start" and "finish" of a ceremony is not serialisable unless the `danger-allow-state-serialisation` feature is turned on, so Hatchdoor would hold it in memory between the two requests, or opt in to storing it.
- Its README says synced passkey managers' "credentials ... can be copied, and some don't even implement user verification correctly, so they should only ever be used with a secure second factor". That is the library authors' stance; most consumer passkeys today are the synced kind.

**Build size.** The largest of the four: registration and sign-in ceremonies on server and PWA, a list of credentials per person (named, removable), a fallback for lost devices, and the domain checks above.

### Outside identity provider (OpenID Connect)

**How it works.** Hatchdoor sends the browser to the provider; the provider signs the person in and sends the browser back with a one-time code; Hatchdoor's server exchanges the code for an ID token that names the person, and starts its own session ([OpenID Connect Core 1.0](https://openid.net/specs/openid-connect-core-1_0.html)). For a web app like Hatchdoor's PWA, the IETF's best current practice for browser apps describes the server doing this exchange and giving the browser only a cookie, the "Backend for Frontend" pattern: tokens stay on the server, and the cookie "MUST" be `Secure` and `HttpOnly` ([RFC 10017, 6.1](https://www.rfc-editor.org/rfc/rfc10017.html)).

**Household member.** Taps "Sign in with ...", signs in at the provider (possibly with a passkey, if the provider offers them), lands back in Hatchdoor. One provider account covers every app connected to it.

**Operator.**
- Runs the provider. Pocket ID, for example, "only supports passkey authentication" ([Pocket ID](https://github.com/pocket-id/pocket-id)); Authelia, Authentik and Keycloak are larger.
- Registers Hatchdoor as a client with its redirect address, and puts the issuer address, client ID and secret into Hatchdoor's settings (environment variables, per ADR-14). Immich's and Mealie's setup pages are representative ([Immich OAuth](https://docs.immich.app/administration/oauth/), [Mealie OIDC](https://mealie.io/documentation/getting-started/authentication/oidc/)).
- Decides whether a first sign-in creates an account (Immich's Auto Register, Mealie's `OIDC_SIGNUP_ENABLED`) or only links to people the admin already added. For a household where the admin invites people, linking only to existing people fits the map's "no self sign-up" rule.
- Hatchdoor's container must reach the provider over the network for discovery, the token exchange and the signing keys.

**Forgotten password, no email.** The provider's job. But a provider that is down, or a claim mapping that goes wrong, locks everyone out of Hatchdoor too, which is why Immich keeps `enable-password-login` and Paperless-ngx points to `createsuperuser` ([Immich server commands](https://docs.immich.app/administration/server-commands/), [Paperless-ngx configuration.md](https://github.com/paperless-ngx/paperless-ngx/blob/dev/docs/configuration.md)). Hatchdoor would need a local way in for the admin in any case.

**Installed phone app.**
- The provider's page is outside the PWA's navigation scope. The manifest specification leaves the handling to the browser but recommends showing "some UI when the application context is navigated to an out-of-scope URL" ([Web App Manifest, 5.1](https://www.w3.org/TR/appmanifest/)); Chrome renders such a page "in an in-app browser instead of within your PWA window" ([web.dev, Web app manifest](https://web.dev/learn/pwa/web-app-manifest/)); MDN describes the same address bar appearing ([MDN, scope](https://developer.mozilla.org/en-US/docs/Web/Progressive_web_apps/Manifest/Reference/scope)). The person sees browser controls during sign-in, then the redirect back lands in scope.
- On iOS, "the website data of home screen web applications is kept isolated from Safari" ([WebKit, Tracking Prevention](https://webkit.org/tracking-prevention/)). Being signed in to the provider in Safari therefore does not sign the person in inside the Home Screen app; they sign in to the provider once more there. Exactly how iOS presents an out-of-scope page inside a Home Screen app, and whether the session cookie set on return reaches the app's window, was not confirmed from a first-party source: `unchecked`, test on a phone.
- Native apps such as Immich's and Audiobookshelf's use custom URL schemes for the return trip ([Immich](https://docs.immich.app/administration/oauth/), [Audiobookshelf](https://www.audiobookshelf.org/guides/oidc_authentication/)); a PWA cannot register one and uses its HTTPS address.

**Rust building blocks.**

| Crate | Version | Released | Maintenance signals |
|---|---|---|---|
| [`openidconnect`](https://crates.io/crates/openidconnect) | 4.1.0 | 2026-10-31 | One maintainer's project; 4.0.1 was 2025-07-06; MSRV 1.88 ([README](https://github.com/ramosbugs/openidconnect-rs)); 83 open issues and pull requests |
| [`oauth2`](https://crates.io/crates/oauth2) | 5.0.0 | 2025-01-21 | Same maintainer; `openidconnect` 4.1.0 depends on `oauth2` 5.0.0 ([Cargo.toml](https://github.com/ramosbugs/openidconnect-rs/blob/main/Cargo.toml)); 50 open issues and pull requests |

- Its default HTTP client is `reqwest`. Its `ureq` feature uses ureq 2 ([oauth2 5.0.0 Cargo.toml](https://github.com/ramosbugs/oauth2-rs/blob/5.0.0/Cargo.toml)) while Hatchdoor uses ureq 3 (`Cargo.toml`). The crate accepts any function from request to response as a client ([oauth2 docs, HTTP clients](https://docs.rs/oauth2/5.0.0/oauth2/)), so Hatchdoor can pass its existing ureq 3 instead of compiling a second HTTP stack.
- The same docs warn "to prevent SSRF vulnerabilities, be sure to configure the HTTP client not to follow redirects" ([oauth2 docs](https://docs.rs/oauth2/5.0.0/oauth2/)).

**Build size.** Medium: the redirect and callback routes, discovery and key caching, mapping a provider identity to a Hatchdoor person, an admin claim or not, and the escape hatch.

### Trusting a sign-in header from a reverse proxy (forward auth)

**How it works.** The proxy asks an authentication server about every request; when the person is signed in, the proxy forwards the request to the app with headers such as `Remote-User` and `Remote-Groups` ([Authelia, Trusted header SSO](https://www.authelia.com/integration/trusted-header-sso/introduction/)). The app believes them.

**When it is safe.** Only when the app is "only accessible via the proxy so that the proxy can strip or override the original request headers" ([Authelia, Trusted header SSO FAQ](https://github.com/authelia/authelia/blob/master/docs/content/integration/trusted-header-sso/frequently-asked-questions.md)). Two extra checks are documented:
- **Trust only the proxy's address.** Authelia recommends a list of addresses allowed to send these headers "even if it is optional", warns that in Docker it must be the proxy's address on the shared network, and says trusting a whole Docker network "is not considered secure as any compromised container may be able to be used to bypass authentication" ([Authelia, Trusted remote networks](https://www.authelia.com/integration/trusted-header-sso/introduction/)).
- **A shared secret.** Mealie's proxy sends a second header with a secret only the proxy knows, because the connecting address "isn't reliable enough" ([Mealie, Reverse proxy auth](https://mealie.io/documentation/getting-started/installation/backend-config/)).

**How it would sit in Hatchdoor today.**
- Hatchdoor currently trusts forwarded headers from any sender, but only to build a transfer link's address, and ADR-34 says a change that lets them "affect anything other than a transfer link's address ... breaks this decision". An identity header would be a new and different kind of trust, needing its own ADR.
- ADR-07's loopback exemption lets a request from the same machine in without the web token. A proxy on the same machine connects from loopback, so header trust and the loopback exemption would interact; the map already lists the loopback question under "Security details".
- Agents cannot complete a proxy's interactive sign-in. The operator would have to exempt `/mcp` and the transfer-link routes from the proxy's check (Authelia's `bypass` policy for a path is the documented way, [Access control](https://www.authelia.com/configuration/security/access-control/)), and agents would keep their own keys.

**Household member.** Signs in at the proxy's page. Hatchdoor shows no login.

**Operator.** Runs the proxy and the authentication server, configures header stripping and forwarding, keeps Hatchdoor off every other network path, and sets the exemptions above. This is the most expert setup of the four.

**Forgotten password, no email.** Entirely the authentication server's job.

**Installed phone app.** Works while the proxy's session lasts. When it expires, the proxy answers the PWA's background API calls with its own redirect or login page rather than a Hatchdoor 401, which the PWA has to recognise (an inference from how forward auth works, not a documented finding).

**Rust building blocks.** None needed: reading a header and checking the peer address or a secret is a few lines of middleware. The cost is in getting the safety checks right.

### Sessions in the browser and the PWA (applies to the first three)

Today the PWA keeps one bearer token in local storage and also sends it as an `access_token` query parameter where a header cannot be set, a "deliberate, documented tradeoff (tokens can appear in URLs/proxy logs)" (ADR-08). With per-person sign-in, a server-side session in a cookie becomes the natural carrier: the browser attaches it to image, stream and download requests by itself, and RFC 10017 sets the cookie rules (`Secure`, `HttpOnly`, a `__Host-` prefix) and the CSRF defences that cookies then need ([RFC 10017, 6.1.3](https://www.rfc-editor.org/rfc/rfc10017.html)). That choice belongs to the map's "Security details" item; it is noted here because every sign-in method above ends in a session.

| Crate | Version | Released | Maintenance signals |
|---|---|---|---|
| [`tower-sessions`](https://crates.io/crates/tower-sessions) | 0.15.0 | 2026-02-01 | Session middleware for axum with pluggable stores ([README](https://github.com/maxcountryman/tower-sessions)); 5 open issues and pull requests |
| [`axum-login`](https://crates.io/crates/axum-login) | 0.18.0 | 2025-07-20 | Built on `tower-sessions` 0.15 and axum 0.8, the axum line Hatchdoor uses ([Cargo.toml](https://github.com/maxcountryman/axum-login/blob/main/axum-login/Cargo.toml)); no release for 16 months but the repository was updated on 2026-11-11; 7 open issues and pull requests |
| [`tower-sessions-file-store`](https://crates.io/crates/tower-sessions-file-store) | 0.3.0 | 2026-10-25 | Third-party file store; about 11,000 downloads |

Where sessions live is open. In memory, every restart signs everyone out. In a file under the state directory, they survive. In SQLite, losing the cache signs everyone out, which is the kind of loss ADR-01 already accepts for disposable data. `axum-login` is optional: its value is a login and permission layer over `tower-sessions`, and ADR-13 asks that such a layer earn its place.

### Getting back in without email (applies to all four)

The options found, from lightest to heaviest:

1. **Admin resets another person** to a temporary password they must change (Immich, Karakeep). Needs one admin who can still sign in.
2. **A binary subcommand run through the container runtime** for the admin: reset a password, re-enable password login, grant admin (Immich `immich-admin`, Mealie scripts, Actual's script, Nextcloud `occ`, Paperless-ngx `createsuperuser`). Fits ADR-12, since `docker exec` can run the binary without a shell.
3. **A one-time sign-in link printed by a subcommand** or made by the admin (Pocket ID). Suits passkeys, where there is no password to reset.
4. **A file the server writes and the operator reads** (Jellyfin's PIN). Proves access to the server's disk instead of running a command; limited to requests from the home network.
5. **Recovery codes** printed once for each person. None of the eight apps' docs read describe them as a way back into an account; they belong with second factors.
6. **An environment variable that creates or resets the first admin** at start (Paperless-ngx). Fits ADR-14's "no browser step", but a password left in a compose file is a secret at rest.

## What this means for the decision (#718)

The options below are the realistic combinations. They are not ranked.

**Option A: passwords only, with admin reset and a recovery subcommand.**
- For the household: the familiar login box; phones fill it in.
- For the operator: nothing new to run; works on a plain-HTTP LAN address as today.
- To build: smallest. `argon2`, an account file in the state directory, a session, an admin reset screen, a subcommand.
- Trade-off: the weakest sign-in against guessing and reuse; no single sign-on for households that already run a provider.

**Option B: passwords, plus an optional outside provider (OIDC).**
- What six of the eight comparable apps do.
- For the household: either way works; a household with a provider gets one sign-in for all its apps, and passkeys through the provider (Pocket ID) without Hatchdoor building them.
- For the operator: optional; those who want it run a provider and register Hatchdoor.
- To build: Option A plus the redirect flow, account linking (to people the admin added, or creating them), an optional admin claim, and the escape hatch that keeps the admin's local way in.
- Trade-off: a second HTTP-client path and claim-mapping rules to test; sign-in leaves the installed app briefly; iOS behaviour needs a device test.

**Option C: passwords plus passkeys built into Hatchdoor.**
- For the household: the strongest and easiest daily sign-in, once set up.
- For the operator: a stable domain with HTTPS becomes required for anyone using passkeys; plain-HTTP LAN installs keep passwords only.
- To build: the largest. `webauthn-rs`, credentials per person, management screens, lost-device recovery by one-time link.
- Trade-off: passkeys in an iOS Home Screen web app are `unchecked`; only two comparable apps offer it, one as of 2026-11-10.

**Option D: an outside provider only (no Hatchdoor passwords).**
- The Actual Budget model for multi-user.
- For the household: one sign-in shared with their other apps.
- For the operator: every multi-user install must run a provider; the hardest start for a household that has none.
- To build: no password storage, but a local way in for the admin is still needed when the provider fails, so a subcommand or a one-time link remains.
- Trade-off: makes a second service a prerequisite for 3.0's headline feature.

**Option E: a proxy-header mode, as an extra next to any of the above.**
- For the household: no Hatchdoor login at all where the operator already uses forward auth.
- For the operator: expert setup; safe only when Hatchdoor is reachable solely through the proxy, plus a trusted-proxy address or shared secret, plus exemptions for agents.
- To build: little code, a new ADR (it reverses ADR-34's "nothing else" rule for forwarded headers), and careful interaction with ADR-07's loopback exemption.
- Trade-off: the easiest to misconfigure into an open instance; every comparable app that offers it carries a warning.

**Questions #718 will have to settle alongside the method.**
- Does multi-user Hatchdoor still support the plain-HTTP LAN address, or does it require HTTPS with a domain? The answer rules passkeys in or out.
- Should the admin's way back in be a subcommand that edits the account file directly, or one that asks the running server over loopback?
- How is the first admin created without a browser step (ADR-14): an environment variable, a subcommand, or both?
- If a provider is offered: may a first sign-in create a person, or only link to people the admin added?

## Sources not read directly

- The published Paperless-ngx docs site answered with a Cloudflare challenge; the same pages were read from the project's repository.
- Audiobookshelf's recovery for a lost root password came only from community discussions and is marked `unchecked`.
- Passkeys inside installed PWAs on iOS and Android, and the exact iOS handling of an out-of-scope sign-in page, have no first-party statement that was found; marked `unchecked`.
