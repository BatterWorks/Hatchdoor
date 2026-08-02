# Managed Git authentication in Hatchdoor's rootless runtime

> Research record.
>
> Researched: 2026-08-02
> Scope: Authentication and credential storage for public, HTTPS-token, and
> SSH Git remotes used through `git2`/libgit2 in Hatchdoor's rootless,
> distroless container
> Status: Decision input, not an implementation specification

## Executive answer

Hatchdoor can support all three relevant remote cases, but they do not have the
same deployment cost:

1. **Public HTTPS repositories** need no credential. libgit2 calls the
   credential callback only when a remote asks for authentication, so anonymous
   clone/fetch should be a first-class auth mode rather than a fake empty token.
2. **HTTPS token authentication** already fits Hatchdoor's build. `git2` exposes
   username/password credentials, which Git hosts use for username/token
   authentication. The missing problem is safe acquisition and storage of the
   token, not transport support.
3. **SSH is feasible but is not present in the current binary.** Hatchdoor must
   enable `git2`'s `ssh` feature, provide a key file, in-memory key, or reachable
   agent, and provision trusted host keys. SSH cannot be treated as “it will use
   the host's normal Git setup” inside the current container.

libgit2 provides credential constructors and an operation-time callback; it is
not a credential vault. `git2` can invoke Git credential helpers, but the
helpers are external processes. Hatchdoor's runtime contains no shell, Git,
credential manager, `ssh-agent`, or browser, so helper- and interactive-login
flows are unavailable unless Hatchdoor deliberately adds and operates those
components. The distroless project explicitly describes its images as omitting
shells and other ordinary OS programs, and Hatchdoor copies only its binary and
frontend into that base ([distroless README](https://github.com/GoogleContainerTools/distroless#distroless-container-images),
[Hatchdoor Dockerfile](../../../Dockerfile#L29-L42)).

The safe deployment baseline is therefore:

- anonymous HTTPS for public repositories;
- HTTPS tokens supplied at operation time from an environment value or,
  preferably, a read-only mounted secret file;
- SSH private-key files and known-hosts files supplied as read-only mounts, or
  an explicitly mounted agent socket, after SSH support is compiled in;
- no credentials embedded in remote URLs or stored in repository config;
- no claim that a UI-persisted secret is secure until Hatchdoor defines a
  separate encryption-key and rotation model.

## Current Hatchdoor baseline

The canonical branch currently has a narrower implementation than the desired
multi-vault product:

- `git2 = 0.21` is built with `default-features = false` and only
  `vendored-libgit2`, `vendored-openssl`, and `https`. The `ssh` feature is
  absent ([Cargo.toml](../../../Cargo.toml#L24)). The lockfile resolves
  `libgit2-sys` 0.18.5 with libgit2 1.9.4 and contains no `libssh2` package.
- Git sync requires `HATCHDOOR_GIT_HTTPS_TOKEN` when enabled and reads the token
  once at startup into a `String` ([git/config.rs](../../../src/git/config.rs#L39-L74)).
- Every remote callback currently returns the configured username/token as a
  `Cred::userpass_plaintext`, without inspecting the remote URL, its username,
  or libgit2's allowed credential types
  ([git/sync.rs](../../../src/git/sync.rs#L89-L96)).
- The runtime is `gcr.io/distroless/cc-debian13:nonroot`, runs as
  `nonroot:nonroot`, and includes no copied authentication tools
  ([Dockerfile](../../../Dockerfile#L29-L42)). Compose mounts the vault, cache,
  and models only; it does not currently mount Git secrets, SSH material, or an
  agent socket ([docker-compose.yml](../../../docker-compose.yml#L13-L18)).

This means today's supported network-auth shape is one static HTTPS token for
one repository. Public anonymous remotes are artificially excluded by startup
configuration, dynamic/expiring credentials cannot refresh without restarting,
and SSH URLs cannot work in the shipped build.

## What `git2` and libgit2 actually provide

### Transport features are compile-time choices

`git2` 0.21 has no network transports in its default feature set. Its own
manifest defines `https` and `ssh` separately; `ssh` enables
`libgit2-sys/ssh`, while `https` enables `libgit2-sys/https`, OpenSSL, and the
credential-helper API. The crate documentation recommends enabling both for
user-provided repository URLs
([git2 0.21 feature manifest](https://docs.rs/crate/git2/0.21.0/source/Cargo.toml.orig),
[git2 crate documentation](https://docs.rs/crate/git2/0.21.0)). Enabling SSH
adds libssh2/OpenSSL build dependencies; it does not add an OpenSSH executable
or `ssh-agent` to the runtime.

This distinction matters for distroless: libgit2's libssh2 transport runs in
the Hatchdoor process, while libgit2's alternative “exec” SSH backend launches
OpenSSH. The upstream build documentation lists both backends, but the latter
cannot work in Hatchdoor's current image because no OpenSSH program is present
([libgit2 build options](https://github.com/libgit2/libgit2#optional-dependencies)).

### Credentials are requested per network operation

`RemoteCallbacks::credentials` receives:

- the URL being accessed;
- any username embedded in that URL; and
- a bitmask of credential types the transport will accept.

The accepted types include username/password, SSH key, SSH key from memory,
default Negotiate credentials, and username-only credentials
([git2 `Credentials`](https://docs.rs/git2/0.21.0/git2/type.Credentials.html),
[`CredentialType`](https://docs.rs/git2/0.21.0/git2/struct.CredentialType.html)).
The callback may be invoked repeatedly when credentials are rejected. SSH may
first request only a username and then request a supported authentication
method; the username cannot change during that session
([libgit2 authentication guide](https://libgit2.org/docs/guides/authentication/)).

A robust Hatchdoor callback therefore must inspect `allowed_types`, use the
URL's username where appropriate, avoid retrying the same rejected credential
forever, and return an explicit error when the configured auth method and the
remote's accepted method do not match. The current unconditional
username/token callback is not a reusable multi-transport design.

### Available credential mechanisms

| Mechanism | `git2` API | Runtime requirement | Hatchdoor assessment |
|---|---|---|---|
| Anonymous HTTPS | No credential callback result needed | Trusted CA bundle and network | Supported by libgit2; current Hatchdoor configuration must stop requiring a token for this mode. |
| HTTPS username/token | `Cred::userpass_plaintext` | Token resolver | Already used. Provider-specific username and scopes must remain configurable. |
| NTLM/Kerberos | `Cred::default` | Host identity/configuration and backend support | Exposed by libgit2, but not a reasonable portable baseline for a minimal container. |
| SSH key file | `Cred::ssh_key` | `ssh` feature, readable private key, optional passphrase | Suitable for read-only secret mounts. Keep the key outside the vault and repository. |
| SSH key in memory | `Cred::ssh_key_from_memory` | `ssh` feature and a supported crypto backend | Available, but libgit2 warns that SSH-memory credentials may not work with every crypto backend; test supported key formats before promising them. |
| SSH agent | `Cred::ssh_key_from_agent` | `ssh` feature and reachable agent socket | Viable as an advanced deployment option. The container does not run an agent, so the socket and permissions must be supplied externally. |
| Git credential helper | `Cred::credential_helper` | Git config plus an executable helper | The API parses `credential.helper`, invokes processes, and reads a username/password result. No helper executable exists in the current image. |

The constructors and helper behavior are documented by
[`git2::Cred`](https://docs.rs/git2/0.21.0/git2/struct.Cred.html). libgit2 notes
that SSH-memory support can vary with the crypto backend
([credential types](https://libgit2.org/docs/reference/v1.9.0/credential/git_credential_t.html)).

### Server identity is separate from client authentication

An accepted client credential does not make the server trustworthy.

- For HTTPS, libgit2/OpenSSL validates the server certificate. `git2` 0.21 can
  set a custom CA file or directory before worker threads start, which is needed
  for self-hosted Git servers using a private CA
  ([`git2::opts`](https://docs.rs/git2/0.21.0/git2/opts/)). A certificate callback
  can override built-in checks; returning `CertificateOk` blindly would disable
  that protection, while passthrough preserves libgit2's result
  ([`CertificateCheckStatus`](https://docs.rs/git2/0.21.0/git2/enum.CertificateCheckStatus.html)).
  The CA setters modify libgit2 global state and must run before threads start,
  so arbitrary per-vault CA stores are not naturally isolated. Multiple private
  CAs would need one deliberately composed trust bundle or a separately designed
  verification layer.
- For SSH, libgit2 versions since 1.5.1 check host keys by default. Hatchdoor's
  resolved libgit2 1.9.4 is newer, so the deployment must provide a trusted
  `known_hosts` source or implement deliberate key pinning. It must not bypass
  host-key verification to make first connection easier
  ([libgit2 security notice](https://libgit2.org/security/)).

The rootless user must be able to read all CA, known-hosts, key, and token files.
It needs write access only to the local vault checkout and `.git`; authentication
material should be mounted read-only.

## Credential storage in a distroless deployment

libgit2 intentionally hands credential acquisition to its caller. Its APIs do
not define encrypted persistence, access policy, rotation, or secret deletion.
Those remain Hatchdoor/deployment responsibilities.

### Storage options

| Source | Persistence | Security properties | Fit for Hatchdoor |
|---|---|---|---|
| No secret | None | Best option for public HTTPS | First-class auth mode. |
| Environment variable | Container lifetime/config dependent | Easy and backward-compatible, but Docker warns environment values may be exposed to other processes or logs | Keep for operator-managed compatibility, not as the preferred secret path. |
| Docker/OCI secret file | Deployment managed | Granted per service and mounted as a file; can be read only | Preferred operator-managed token/passphrase source. |
| Read-only private-key file | Deployment managed | Secret remains outside app settings; filesystem permissions and mount ownership are critical | Preferred initial SSH key source. |
| Mounted SSH-agent socket | Agent lifetime | Private key stays outside the container, but any process with socket access can request signatures | Useful opt-in for advanced operators; operationally fragile across restarts and UID/socket changes. |
| Git `credential-store` | Persistent plaintext | Git explicitly documents that passwords are unencrypted and protected only by filesystem permissions | Do not recommend or silently enable. |
| OS keychain/helper | External store | Can be secure when a supported helper and OS service exist | Not available in the shipped image; adding it is a deployment/product expansion. |
| Hatchdoor settings database | Persistent | Safe only if secrets are encrypted with a key stored separately from the ciphertext and rotation/deletion are defined | Requires a separate product/security decision; libgit2 does not solve it. |
| Remote URL / `.git/config` | Persistent plaintext and easy to leak | Git providers warn tokens in clone URLs are written into `.git/config` | Prohibit. Store a clean remote URL plus a separate credential reference. |

Docker recommends Compose secrets over environment variables and mounts them at
`/run/secrets/<name>` with per-service access
([Docker Compose secrets](https://docs.docker.com/compose/how-tos/use-secrets/)).
For file-backed Compose secrets, UID/GID remapping is not supported because the
secret is a bind mount, so Hatchdoor's nonroot readability must be validated on
the target host
([Compose services reference](https://docs.docker.com/reference/compose-file/services/#secrets)).

Git's built-in `credential-store` writes credentials unencrypted, while secure
credential helpers are separate programs integrating with an OS store
([`git-credential-store`](https://git-scm.com/docs/git-credential-store),
[`gitcredentials`](https://git-scm.com/docs/gitcredentials)). Neither model can
be assumed inside Hatchdoor's current image.

For a UI-entered token or private key, “redacted in API responses” is necessary
but not sufficient. A later ticket must decide whether Hatchdoor:

1. accepts plaintext-at-rest application storage as an explicit self-hosted
   tradeoff;
2. encrypts with an instance master key supplied outside the settings database
   (preferably by secret file); or
3. stores only a reference to an operator-managed secret and never persists the
   secret value itself.

Encryption whose key is stored beside the ciphertext does not materially
improve protection against filesystem compromise. Any UI design must also make
replacement possible without redisplaying the existing secret and erase stored
values when a vault/auth profile is removed.

## Provider constraints later decisions must respect

### GitHub

- Personal access tokens are used as the HTTPS password. GitHub requires a
  non-empty username, although the token—not the username—authenticates the
  request ([GitHub PAT documentation](https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens#using-a-personal-access-token-on-the-command-line)).
- Fine-grained PATs can be limited to selected repositories and minimal
  permissions, but organizations can require approval and enforce lifetime
  policies. GitHub advises automation with many tokens to use a GitHub App
  instead
  ([fine-grained PAT creation](https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens#creating-a-fine-grained-personal-access-token)).
- A deploy key belongs to one repository, is read-only by default, may be given
  write access, does not expire, and cannot be reused for another repository.
  A multi-vault instance using deploy keys therefore needs a distinct key pair
  per GitHub repository
  ([GitHub deploy keys](https://docs.github.com/en/authentication/connecting-to-github-with-ssh/managing-deploy-keys#deploy-keys)).
- GitHub App installation tokens can cover one or multiple repositories with
  fine-grained permissions, but expire after one hour and must be regenerated
  on demand. They work for Git over HTTPS as the password, with
  `x-access-token` as the username
  ([GitHub App installation authentication](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/authenticating-as-a-github-app-installation#using-an-installation-access-token-to-authenticate-as-an-app-installation)).

Consequently, a static `GitConfig` token read only at process startup cannot
later support GitHub App authentication correctly. The credential source must
be resolved per operation and be able to refresh before retrying.

### GitLab

- Personal, group, and project access tokens can use `read_repository` for
  clone/pull or `write_repository` for pull/push over HTTPS
  ([GitLab access-token scopes](https://docs.gitlab.com/security/tokens/access_token_scopes/)).
- Project access tokens authenticate Git over HTTPS with any non-blank username
  and the token as password. On GitLab.com they require Premium or Ultimate,
  while self-managed availability differs
  ([GitLab project access tokens](https://docs.gitlab.com/user/project/settings/project_access_tokens/)).
- GitLab deploy tokens support `read_repository` for Git clone but do not offer
  a repository-write scope; they are appropriate for pull-only vaults, not
  Hatchdoor's two-way mode
  ([GitLab deploy tokens](https://docs.gitlab.com/user/project/deploy_tokens/#scope)).
- GitLab SSH deploy keys can instead be read-only or read-write and may be
  shared across explicitly authorized projects, but protected-branch pushes
  also depend on the key owner's project access and branch rules
  ([GitLab deploy keys](https://docs.gitlab.com/user/project/deploy_keys/)).
- OAuth access tokens use a conventional `oauth2` username and can carry
  `read_repository` or `write_repository`. GitLab points to credential helpers
  for automatic refresh, but Hatchdoor would need to implement refresh itself
  because no helper exists in the image
  ([GitLab OAuth](https://docs.gitlab.com/api/oauth2/#access-git-over-https-with-access-token)).

Provider differences mean Hatchdoor must not hardcode one token username, one
scope label, or one assumption about read/write capability.

### Generic and self-hosted servers

Generic HTTPS basic-auth and token flows fit `Cred::userpass_plaintext`, but the
username is part of the credential and may be provider-defined. Self-hosted
servers may also require a private CA bundle. Generic SSH requires a username in
the URL or a username-only callback response before the key is supplied
([libgit2 authentication guide](https://libgit2.org/docs/guides/authentication/)).

Hatchdoor should test capabilities by performing a non-destructive remote read
and, for two-way mode, validating write permission without putting user content
at risk. A successful clone/fetch proves only read access; it does not prove
push access or branch-policy acceptance.

## Constraints for the Wayfinder map

Later product and architecture decisions should preserve these constraints:

1. **Auth belongs to a vault, not the instance.** Each remote-backed vault needs
   an explicit auth mode and credential reference; one vault's failure must not
   poison another vault's callbacks or state.
2. **Anonymous is an explicit mode.** Public repositories must work without a
   dummy token, and Hatchdoor must not send configured credentials to an
   unrelated host after redirects or URL changes.
3. **Resolve credentials at operation time.** This supports secret-file
   rotation and expiring provider tokens without restarting the instance.
4. **Keep remote URLs credential-free.** Never persist, log, return, or display
   a URL containing a token or password. Error and diagnostics paths need the
   same redaction rule.
5. **Separate secret values from settings.** Environment/UI settings may choose
   auth type, username, and a secret reference. Persisting the secret value
   itself requires an explicit encryption and lifecycle decision.
6. **Do not inherit host Git behavior implicitly.** Credential helpers, global
   Git config, `$HOME/.ssh`, private CAs, known hosts, and agent sockets exist
   only when the deployment deliberately provides them.
7. **SSH requires both client and server trust.** Enabling the Cargo feature and
   accepting a private key are insufficient without known-host verification.
8. **Permission matches sync mode.** Pull-only accepts read credentials;
   two-way requires repository write permission. Provider token labels and
   product modes are not interchangeable.
9. **Auth failure is degraded remote state, not loss of the local vault.** A
   failed or expired credential should produce an explicit per-vault warning
   while local files and local Git history remain usable.
10. **Build an integration matrix.** At minimum: anonymous HTTPS; private HTTPS
    read and write; wrong/expired token; private-CA rejection and acceptance;
    SSH key file; SSH agent; unknown and changed SSH host key; provider read-only
    credential used in two-way mode; and two vaults where only one auth path
    fails.

## Recommended support sequence

This research supports a staged product decision rather than one universal
credential abstraction:

1. **Baseline:** anonymous HTTPS and static HTTPS username/token, with both
   environment-value compatibility and a read-only secret-file source.
2. **SSH:** enable and test `git2`'s SSH feature; add per-vault key-file or agent
   auth plus explicit known-host provisioning. Treat in-memory pasted keys as a
   separate storage decision.
3. **Managed provider login:** add OAuth/GitHub App-style integrations only when
   Hatchdoor is ready to own browser/device authorization, refresh-token or app
   private-key storage, token renewal, and provider-specific error handling.

This sequence keeps the rootless/distroless invariant intact: credentials are
fed directly to libgit2 callbacks, while no shell, full Git CLI, or desktop
keychain is assumed at runtime.
