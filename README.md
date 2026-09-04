<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/hatchdoor-wordmark-dark.png">
    <img src="assets/hatchdoor-wordmark.png" alt="Hatchdoor" width="340">
  </picture>
</p>

<p align="center">
  <a href="https://hatchdoor.battercloud.cc"><img alt="Live demo" src="https://img.shields.io/badge/live_demo-hatchdoor.battercloud.cc-e4572e"></a>
  <a href="https://docs-hatchdoor.battercloud.cc"><img alt="Documentation" src="https://img.shields.io/badge/docs-docs--hatchdoor.battercloud.cc-6f42c1"></a>
  <a href="https://hub.docker.com/r/battermanz/hatchdoor"><img alt="Docker Hub" src="https://img.shields.io/docker/v/battermanz/hatchdoor?sort=semver&label=docker%20hub&color=2496ed"></a>
  <a href="https://github.com/BattermanZ/Hatchdoor/blob/main/Dockerfile"><img alt="Rootless and distroless image" src="https://img.shields.io/badge/image-rootless_%26_distroless-2ea44f"></a>
  <a href="LICENSE"><img alt="License: AGPL-3.0" src="https://img.shields.io/badge/license-AGPL--3.0-blue"></a>
</p>

# Hatchdoor

Hatchdoor is a self-hosted, **agent-native** web app for your Obsidian-style
Markdown vault. Browse, search, and edit your notes in a fast web UI, and give
AI agents first-class access to the very same vault over the Model Context
Protocol (MCP).

Point an MCP client like Claude, Claude Code, Codex, Cursor, or Hermes at
Hatchdoor and your agent can read, search (keyword and semantic), create, edit,
move, and link notes. Every action goes through the same safe, atomic vault
operations the UI uses, with optional automatic git commit-and-push. The web UI
and your agents are two front doors to one vault.

Your Markdown files stay the source of truth. Hatchdoor builds a disposable
SQLite read model for fast browsing, links, backlinks, keyword search, semantic
search, graph data, and metadata. If the cache is deleted, Hatchdoor rebuilds it
from the vault.

Hatchdoor was built with AI coding agents, primarily Claude Code and Codex,
under close human review, with tests and a documented safety model.

<p align="center">
  <a href="https://hatchdoor.battercloud.cc">
    <img src="assets/screenshots/hero-light.png" width="900"
      alt="Hatchdoor browsing a note: vault explorer on the left, rendered Markdown with wikilinks in the centre, and an on-this-page outline on the right">
  </a>
</p>

<p align="center">
  <b><a href="https://hatchdoor.battercloud.cc">&#9654;&nbsp; Try the live demo</a></b>, a read-only public vault, or
  read the <b><a href="https://docs-hatchdoor.battercloud.cc">user documentation</a></b> for setup and usage guides.
</p>

<details>
<summary><b>Contents</b></summary>

- [What You Get](#what-you-get)
- [Screenshots](#screenshots)
- [Who It Is For](#who-it-is-for)
- [What Is Next](#what-is-next)
- [Quick Start With Docker](#quick-start-with-docker)
- [Data And Safety Model](#data-and-safety-model)
- [Configuration](#configuration)
- [MCP Agent Access](#mcp-agent-access)
- [Versioning and Git Sync](#versioning-and-git-sync)
- [Running Without Docker](#running-without-docker)
- [Troubleshooting](#troubleshooting)
- [Security Notes](#security-notes)
- [Development](#development)
- [Project Docs](#project-docs)
- [License](#license)

</details>

## What You Get

- A web UI for browsing folders and Markdown notes.
- Clean note URLs at `/n/:slug`.
- Obsidian-style wikilinks for `[[Note]]`, `[[Folder/Note]]`, and
  `[[Note|Alias]]`.
- Markdown rendering with GitHub-flavored Markdown, math, Mermaid diagrams,
  frontmatter, images, attachments, and broken-link styling.
- Keyword search and semantic search.
- Recent notes, backlinks, outbound links, stats, and graph views.
- Browser write support when the vault mount is writable.
- Attachment uploads, local asset serving, and inline previews for linked PDF
  vault assets.
- A first-class MCP server so AI agents can read, search, create, edit, and link
  notes with the same safety as the UI.
- Optional automatic git commits and pushes for Hatchdoor writes.
- PWA assets and service worker caching for common read paths.
- Distroless, rootless container image (no shell, runs as `nonroot`) that
  deploys with either Docker or Podman.

## Screenshots

<table>
  <tr>
    <td width="50%" valign="top">
      <img src="assets/screenshots/graph-light.png" width="100%" alt="Interactive knowledge graph of notes, links, and tags">
      <p align="center"><sub><b>Knowledge graph</b>: notes, links, and tags</sub></p>
    </td>
    <td width="50%" valign="top">
      <img src="assets/screenshots/search-light.png" width="100%" alt="Search results with snippets for a natural-language query">
      <p align="center"><sub><b>Semantic + keyword search</b></sub></p>
    </td>
  </tr>
  <tr>
    <td width="50%" valign="top">
      <img src="assets/screenshots/hero-dark.png" width="100%" alt="Note view rendered in dark mode">
      <p align="center"><sub><b>Dark mode</b></sub></p>
    </td>
    <td width="50%" valign="top" align="center">
      <img src="assets/screenshots/mobile-light.png" width="260" alt="Responsive mobile layout on a phone">
      <p align="center"><sub><b>Responsive &amp; installable (PWA)</b></sub></p>
    </td>
  </tr>
</table>

## Who It Is For

Hatchdoor is useful if you have a folder of Markdown notes and want a private
web interface for them.

It is beginner-friendly enough to run with Docker Compose, but it also includes
advanced features for people who want agent access, git-backed vault sync,
semantic search, and local development.

Hatchdoor is not a hosted sync service, not a multi-user collaboration platform,
and not a replacement for Obsidian. It is a self-hosted companion for a Markdown
vault you control.

## What Is Next

The next large feature is multi-account handling with permission scoping. Today
Hatchdoor assumes a single trusted user on a private deployment, so whoever
reaches the instance sees every vault it serves. The plan is to let several
people and agents share one instance, each scoped to the vaults they are
granted, with an admin role that manages the rest. A household or a small team
could then run one Hatchdoor instead of one per person.

v2.5.0 already made vaults independent, addressable things the product manages,
including vault scoping for MCP agents. What is missing is the half that was
deliberately deferred: accounts, authentication, and an access boundary the
server enforces rather than the UI implies. Targeted at v3.

Two other things are wanted but not slotted to a version yet:

- **Agent provenance and undo.** Which agent changed a note, a before/after diff
  beside the commit it produced, and one-click revert. Today git is the whole
  answer, and reading a commit log is not the same as seeing what an agent did
  to your note.
- **Vault integrity checks.** A vault-scoped lint for orphaned notes, broken
  wikilinks, and layer markers whose declaration has vanished, so a vault an
  agent maintains unattended cannot quietly drift out of shape.

The [product roadmap](docs/roadmap/product-roadmap.md) has the full set of
workstreams and where each one stands.

## Quick Start With Docker

### 1. Requirements

You need:

- Docker and Docker Compose (Podman and `podman compose` also work)
- A Markdown vault folder, or an empty folder if you want Hatchdoor to create a
  starter vault

### 2. Create Your Config

Copy the example environment file:

```bash
cp .env.example .env
```

The defaults create a starter vault beside the Compose file. To use an existing
vault, uncomment its host path in `.env`:

```env
HOST_VAULT_PATH=/absolute/path/to/your/markdown-vault
```

What these mean:

- `HOST_VAULT_PATH` is your Markdown vault on the host machine.
- `HOST_CACHE_PATH`, `HOST_STATE_PATH`, and `HOST_MODELS_PATH` are optional
  host-side locations for the generated cache, authoritative Vault registry,
  and downloaded models; Compose defaults them beside the project.

Before the first managed-Vault start, create the default authoritative state
directory with access for the image's numeric `nonroot` user. Docker otherwise
may create a missing bind source as root, leaving the registry unwritable:

```bash
mkdir -p data/state
chmod 700 data/state
sudo chown 65532:65532 data/state
```

For rootless Podman, use `podman unshare chown 65532:65532 data/state` instead
of `sudo chown`. Apply the same ownership rule to a custom `HOST_STATE_PATH`.

Do not add ordinary Settings values to `.env`: an unset value can be changed
live in Settings. See [Configuration](#configuration) for the few deployment
values that always remain environment-only.

### 3. Start Hatchdoor

```bash
docker compose up -d
```

Docker Compose binds Hatchdoor to a non-loopback container interface, so a
first run without a web token stops safely and prints a fresh, recoverable
token. Retrieve it with:

```bash
docker compose logs hatchdoor
```

Copy the printed `HATCHDOOR_WEB_BEARER_TOKEN=...` assignment into `.env`, then
start again with `docker compose up -d`. The token is deliberately not stored
by Hatchdoor; use the one from that refusal or generate a new long random token.
Once the server is running, open `http://localhost:42824` and enter it in the
browser prompt.

### 4. Choose Your Search Model

Hatchdoor images include no model weights. On first launch, before it
downloads anything, Hatchdoor asks you to pick one: **Gemma** (multilingual,
the default, requires accepting its terms) or **Nomic Embed Text v1.5**
(English-only, no terms to accept). Either way the model and its acceptance
receipt stay in `HOST_MODELS_PATH` and persist across restarts; Hatchdoor
never sends vault content anywhere. Vault features stay unavailable until
setup finishes.

### 5. Container Image And Paths

The image is published on [Docker Hub](https://hub.docker.com/r/battermanz/hatchdoor):

```text
battermanz/hatchdoor:latest          # also version tags, e.g. 2.6.0
battermanz/hatchdoor:podman-latest   # for Podman users (podman-<version> too)
```

The runtime image is **distroless and rootless**. It is built on
`gcr.io/distroless/cc-debian13:nonroot`, ships no shell or package manager, and
runs as an unprivileged `nonroot` user. Hatchdoor also runs unchanged under
Podman (rootless included); swap `docker` / `docker compose` for `podman` /
`podman compose` **and** the image tag for `podman-latest` (or
`podman-<version>`) — the `latest` tag above is Docker-only.

Docker Compose mounts:

| Container path | Purpose |
| --- | --- |
| `/data/vault` | Markdown vault, source of truth |
| `/data/cache` | Generated SQLite cache |
| `/data/state` | Authoritative Vault identities and source definitions |
| `/models` | Downloaded search model and local Gemma terms receipt |

## Data And Safety Model

Hatchdoor is designed around a simple rule: your Markdown vault is the source of
truth.

- Markdown files live in each Vault's own folder, as recorded in the Vault
  registry. `VAULT_PATH` is not that location: it is read once on a first start
  to seed the registry with a first local Vault, and ignored from then on.
- Vault identities and source definitions live in `/data/state/vaults.json`.
  A Vault's Git HTTPS credential is stored there too, so the file is created
  with `0600` permissions on Unix and belongs in a backup you treat as secret.
  The API never returns it: a Vault reports only `credential_configured`, and
  an edit that means to keep a stored secret says so with `https_credentials:
  {"action": "keep"}` rather than resending it.
- SQLite is a generated cache and can be rebuilt.
- The SQLite cache should live outside the vault.
- Hatchdoor scans `.md` files under the vault while excluding built-in and
  configured noise paths (including `.hatchdoor-trash`).
- Delete actions move notes, and the assets kept inside their own folder, into `.hatchdoor-trash`.
- Archive actions move notes under `HATCHDOOR_ARCHIVE_PREFIX`.
- Browser write actions are available only when the vault is writable.
- MCP is disabled by default.
- MCP requires its own bearer token whenever it is enabled.
- Versioning is off by default; it can keep local Git history or safely sync an
  existing remote.

Upgrading an existing single-Vault deployment requires persistent
`/data/state`; see the [legacy single-Vault upgrade
guide](docs/migrations/legacy-single-vault.md) for detection, recovery, and
rollback constraints.

If the folder `VAULT_PATH` points at contains no Markdown files, Hatchdoor
creates a small starter vault there (a lightweight PARA-style structure with
onboarding notes) before the first index build. Existing vaults are never
seeded or modified. The starter notes are ordinary Markdown you can edit, move,
or delete like any other.

For write access: browser writes, MCP writes, attachment uploads, and git sync
all require the vault mount, cache directory, and state directory to be
writable by the container's non-root runtime user. Read-only browsing works
with a read-only vault mount as long as the cache and state directories stay
writable. If write features are unexpectedly disabled, check those mount
permissions.

Hatchdoor doesn't require any particular vault layout — PARA, Zettelkasten,
Andrej Karpathy's [LLM wiki
pattern](https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f),
or none of the above all work. The [user
documentation](https://docs-hatchdoor.battercloud.cc) compares them, and [How
to run an LLM wiki in
Hatchdoor](https://docs-hatchdoor.battercloud.cc/v/bef3df28-8c2e-4722-89ad-bd4d0bcb3def/n/how-to-run-an-llm-wiki-in-hatchdoor)
walks through the layer-based setup (raw sources on a separate
`.hatchdoor-layer`, default surface for the curated wiki).

## Configuration

Copy `.env.example` to `.env`. Its values are all commented out: Docker Compose
and Hatchdoor supply the ordinary defaults, and Settings owns live server
configuration. A non-empty value for a server-wide Settings key in `.env` is
an intentional **environment pin**: it wins over the saved Settings value for
that process, and shows as **Set in .env** in Settings until the pin is
removed and the container restarts. Vault definitions themselves are managed
per Vault through Settings, the HTTP API, or MCP, not through `.env`.

Two defaults worth knowing before you deploy:

- Hatchdoor refuses to start on `HOST=0.0.0.0` or another non-loopback bind
  unless `HATCHDOOR_WEB_BEARER_TOKEN` is set. On refusal it prints a fresh
  token and the `.env` line to add; that's the fix, not a bug.
- `HATCHDOOR_DEMO_MODE=true` runs a read-only, unauthenticated instance for
  public browsing. It has no rate limiting of its own (search embeds every
  query, note downloads bundle attachments in memory), so put a
  rate-limiting reverse proxy in front before exposing it publicly.

Every deployment variable, every live Settings-editable value, layer and
exclusion rules, and how the search index and cache work are documented in
full in [Settings and environment variables
reference](https://docs-hatchdoor.battercloud.cc/v/bef3df28-8c2e-4722-89ad-bd4d0bcb3def/n/settings-and-environment-variables-reference)
and [The layer
system](https://docs-hatchdoor.battercloud.cc/v/bef3df28-8c2e-4722-89ad-bd4d0bcb3def/n/the-layer-system).

## MCP Agent Access

The embedded MCP endpoint is disabled by default, at `http://127.0.0.1:42824/mcp`.
It has its own bearer token, separate from the web token, required even for
read-only access, because `/mcp` bypasses the web auth layer. Turn it on and
generate a token in **Settings → Agent access (MCP)**; turn on write access
separately, only once you trust what the agent will do with it. Changes apply
to new MCP requests immediately, no restart required.

Full client setup (Claude Code, Codex, OpenClaw, Hermes), the Vault-scope
contract every tool call needs, and the attachment-upload paths are in
[Connect your
agent](https://docs-hatchdoor.battercloud.cc/v/bef3df28-8c2e-4722-89ad-bd4d0bcb3def/n/connect-your-agent)
and [MCP tools
reference](https://docs-hatchdoor.battercloud.cc/v/bef3df28-8c2e-4722-89ad-bd4d0bcb3def/n/mcp-tools-reference).

## Versioning and Git Sync

Versioning is configured per Vault, not per server: choose No Git, Local
history, Pull-only, or Two-way on that Vault's Settings page. Merge conflicts
are always kept for human resolution; Hatchdoor never force-checks out over
uncommitted manual vault edits.

See [How to set up a Git-backed
Vault](https://docs-hatchdoor.battercloud.cc/v/bef3df28-8c2e-4722-89ad-bd4d0bcb3def/n/how-to-set-up-a-git-backed-vault)
for setup, and
[`docs/migrations/legacy-single-vault.md`](docs/migrations/legacy-single-vault.md)
if you're upgrading a pre-registry single-Vault deployment.

## Running Without Docker

If [`just`](https://github.com/casey/just) is installed, `just dev-start` builds
on top of the manual steps below to also track PIDs and prevent duplicate
servers or stale build-cache directories from piling up; `just dev-stop` shuts
both down cleanly, and `just --list` shows the rest (`dev-status`,
`dev-clean`, `prod-check`). See the `justfile` for what each recipe does. Build
artifacts are shared through the primary checkout across linked worktrees;
explicit `CARGO_TARGET_DIR`, `CARGO_HOME`, and `HATCHDOOR_TMPDIR` values can
override the portable defaults.

Otherwise, build the frontend once:

```bash
cd frontend
npm ci
npm run build
cd ..
```

Run the backend:

```bash
cargo run
```

By default, local source runs bind to `127.0.0.1:42824` and seed their first
Vault from `./vault`. Point that first start at a real vault with:

```bash
VAULT_PATH=/path/to/notes cargo run
```

For frontend dev mode:

```bash
# terminal 1
cargo run

# terminal 2
cd frontend
npm run dev
```

The first-run model choice also applies to local development. Hatchdoor stores
models in `./models` by default, so no model-prefetch command is required.

## Troubleshooting

### Hatchdoor refuses to start on `0.0.0.0`

Set `HATCHDOOR_WEB_BEARER_TOKEN`, bind to `127.0.0.1`, or enable
`HATCHDOOR_DEMO_MODE=true` for a read-only public demo. This is intentional: a
non-loopback bind can expose your vault to the network.

### The app starts with a starter vault

Hatchdoor seeds starter notes only on a first start, and only when the folder
`VAULT_PATH` points at holds no Markdown files. If you expected an existing
vault, this almost always means the container mounted an empty directory:
double-check `HOST_VAULT_PATH` in `.env` isn't a typo or a stale Docker volume
shadowing the mount.

For write permission issues, MCP `401`/`403`, git sync problems, and more, see
[How to troubleshoot common
problems](https://docs-hatchdoor.battercloud.cc/v/bef3df28-8c2e-4722-89ad-bd4d0bcb3def/n/how-to-troubleshoot-common-problems)
in the user documentation. Every HTTP endpoint is documented in [HTTP API
reference](https://docs-hatchdoor.battercloud.cc/v/bef3df28-8c2e-4722-89ad-bd4d0bcb3def/n/http-api-reference).

## Security Notes

- Use a long random `HATCHDOOR_WEB_BEARER_TOKEN`.
- Do not expose Hatchdoor publicly without HTTPS in front of it.
- Use `HATCHDOOR_DEMO_MODE=true` only for browse-only public test instances.
- Keep MCP disabled unless you need it.
- Treat MCP write mode as powerful: it can create, edit, move, delete, and
  import content.
- Keep the SQLite cache outside the vault.
- Keep `.env` out of git.
- Review Docker volume paths before starting the container.

## Development

Backend checks:

```bash
cargo fmt --check
CARGO_BUILD_JOBS=1 cargo clippy --all-targets -- -D warnings
CARGO_BUILD_JOBS=1 cargo test
```

Frontend checks:

```bash
cd frontend
npm run format:check
npm run typecheck
npm run lint
npm test
npm run build
```

Build and publish the Docker image:

```bash
docker build -t battermanz/hatchdoor:latest .
docker tag battermanz/hatchdoor:latest battermanz/hatchdoor:2.6.0
docker push battermanz/hatchdoor:2.6.0
docker push battermanz/hatchdoor:latest
```

## Project Docs

- [User documentation](https://docs-hatchdoor.battercloud.cc): setup, configuration,
  and day-to-day usage guides for running Hatchdoor, hosted in a Hatchdoor
  vault itself.
- [Documentation index](docs/README.md): architecture, collaboration, roadmap,
  research, maintenance, and historical records.
- [Product roadmap](docs/roadmap/product-roadmap.md): draft overall product direction
  and the workstreams it breaks into.
- [Design system](docs/design/design-system.html): visual tokens, component patterns,
  layout rules, and interaction states used by the frontend.
- [Semantic search strategy](docs/adr/semantic-search-strategy.md): decision
  record for shipping pure semantic search instead of hybrid retrieval or a
  cross-encoder reranker in the runtime path.

## License

Hatchdoor is licensed under the GNU Affero General Public License v3.0 only.
See [LICENSE](LICENSE).

Third-party material — bundled icons, and the embedding models downloaded at
runtime — is recorded in [THIRD_PARTY_NOTICES](THIRD_PARTY_NOTICES.md).
