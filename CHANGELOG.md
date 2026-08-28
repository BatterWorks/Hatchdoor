# Changelog

## Unreleased

- The MCP endpoint now speaks the current **2026-07-28** protocol revision
  alongside `2025-11-25`. On the modern revision there is no handshake: a client
  opens with a stateless `server/discover` call and carries its protocol version
  and per-request `_meta` on every request afterward. A client on `2025-11-25`
  keeps its `initialize` session, unchanged. Live updates arrive through a single opt-in
  `subscriptions/listen` stream instead of SSE subscribe/unsubscribe, and the
  server now advertises `tools.listChanged` honestly on that surface — a client
  on the older revision keeps negotiating it as `false` and reissuing
  `tools/list`, as before. Discovery and `tools/list` responses carry a
  five-minute private cache TTL so a stale catalogue heals itself.
- The `/mcp` protocol boundary is now the `rmcp` library rather than a
  hand-written JSON-RPC layer. Tool names, arguments, and response shapes are
  unchanged; per-request security ordering (enabled check, token configured,
  Origin allow-list, constant-time bearer compare, protocol-version header) is
  unchanged.
- Every MCP tool advertises an `outputSchema`, generated from the same typed
  structure its responses serialize from, so a client can validate a result
  against the contract it was given instead of inferring the shape.
- **Four new MCP tools**, all purely additive — no existing tool's name,
  arguments, or behavior changed:
  - `get_frontmatter` reads one note's tags, aliases, and other properties
    without its Markdown body. It answers under read permission, and a note with
    no frontmatter block returns an empty projection rather than an error.
  - `update_frontmatter` does a shallow top-level merge into a note's
    frontmatter and leaves the body byte-for-byte untouched. Keys you don't
    mention survive, an explicit `null` deletes one, a nested mapping is
    replaced wholesale, a note without a block gets one created, and deleting
    the last key strips the empty block. Same `expected_content_hash`
    concurrency as every other write.
  - `get_attachment` returns an attachment's bytes, mirroring the upload flow in
    reverse: an HTTP `download_url` by default, base64 inline as the fallback for
    a client that can't fetch out-of-band, bounded by the same
    `HATCHDOOR_MCP_MAX_BASE64_BYTES` cap as `import_attachment`.
  - `batch` runs an ordered list of note and attachment operations in one call:
    best-effort with a per-item result, no rollback, no mid-batch visibility.
    Vault-management tools can't go inside one, and a batch is capped at 50
    read-shaped and 20 write-shaped items. Within a single batch,
    `expected_content_hash` is chained between items touching the same note, so
    an agent can create or edit a note and reference it again later in the same
    call without an intermediate read; a note the batch hasn't written validates
    its hash normally. That relaxation never escapes the call — each touched
    Vault's mutation lock is held for the rest of it.
- **The Vault asset route now accepts a live MCP bearer token**, so an agent can
  fetch the `download_url` `get_attachment` hands it without also being given the
  web bearer token. `GET /api/v1/vaults/{vault_id}/assets/{*path}` takes either
  credential as an `Authorization: Bearer` header; the web token additionally
  keeps its `access_token` query form, which the browser needs for `<img>` tags,
  while the MCP token is header-only. Crucially the URL is a cheaper transport,
  not a larger allowance: a request admitted on the MCP token is held to the same
  `HATCHDOOR_MCP_MAX_BASE64_BYTES` ceiling `get_attachment`'s base64 encoding
  enforces (a larger attachment returns `413`) and spends the same per-token rate
  quota and concurrency budget an MCP tool call spends, from the same counter,
  answering `429` with `Retry-After` when exhausted. A web-token request is
  subject to neither. Disabling MCP revokes it on the very next request, no
  restart. Deployments with no web bearer token configured serve this route
  openly, exactly as before.
- MCP now has layered resource protection: a rolling 120-calls-per-minute quota
  per bearer token, plus process-wide concurrency caps (8 ordinary calls, 2
  concurrent `search_notes`). An over-limit request is rejected before dispatch
  with `429` and a `Retry-After` header; protocol, discovery, and list handling
  are exempt. The whole layer can be switched off at runtime with the new
  instant setting `HATCHDOOR_MCP_RATE_LIMITS_ENABLED` (default on).

### ⚠️ Breaking changes — action required on upgrade
- **Three instance-wide status routes are gone: `GET /api/index-status`,
  `GET /api/git-status`, and `GET /api/vault-status`.** They described a single
  Vault at a time when there was only ever one, and every question they answered
  is now answered per Vault by `GET /api/v1/vaults`, which reports each Vault's
  condition, its last search or versioning error, and whether it is indexing.
  All three now return `404`. The two Settings consoles they fed — **Search
  index** and **Versioning** — leave the Settings page with them, along with
  their two-second polling; each Vault's own settings page keeps its condition,
  its last error, and its **Sync now**, **Try again**, and **Rebuild search
  index** buttons, and the scope zone and explorer keep showing a Vault as
  indexing. `/api/startup-status` is unchanged and remains the unauthenticated
  startup probe.
  **Action:** if a script, dashboard, or uptime check polls any of the three,
  point it at `/api/startup-status` for process startup, or at
  `GET /api/v1/vaults` for per-Vault condition. Nothing in Hatchdoor's
  configuration brings the old routes back.
- **`PATCH /api/settings` no longer asks you to confirm `git_init` or
  `git_downgrade`.** Those two consequences belonged to the instance-wide
  versioning lifecycle that the retired **Versioning** console explained, and no
  current deployment reaches it. A save that turns on local versioning, or
  switches off remote versioning, now applies on the first request instead of
  answering `409` and waiting for a resend. `reindex` is unaffected and is still
  confirmed exactly as before, and per-Vault Git changes keep their own
  confirmations on `/api/v1/vaults/{vault_id}`.
  **Action:** if an API client sends `"confirm": ["git_init"]` or
  `"confirm": ["git_downgrade"]`, remove those values — an unknown consequence
  is refused as a validation error. A client that only ever sends `reindex`
  needs no change.
- **MCP protocol revisions `2025-03-26` and `2025-06-18` are no longer served.**
  The endpoint now advertises and accepts exactly `2026-07-28` and `2025-11-25`;
  a client pinned to one of the two dropped revisions is refused on the
  protocol-version header rather than silently downgraded. If an MCP client
  stops connecting after this upgrade, check which revision it pins and update
  it — nothing in Hatchdoor's own configuration restores the dropped ones.
- Hatchdoor no longer carries a second, unused copy of indexing and Git sync.
  The instance-wide debounced Git task and the single-Vault rebuild it went
  with were unreachable after the multi-Vault cutover; every Vault has done its
  own indexing and its own Git turns since v2.5.0. Behaviour is unchanged.
  `HATCHDOOR_GIT_SYNC_ENABLED`, `HATCHDOOR_GIT_REMOTE`, `HATCHDOOR_GIT_BRANCH`,
  `HATCHDOOR_GIT_HTTPS_USERNAME`, `HATCHDOOR_GIT_HTTPS_TOKEN`,
  `HATCHDOOR_GIT_DEBOUNCE_SECONDS`, and `HATCHDOOR_EXCLUDE` stay in Settings
  and keep their values, but nothing acts on them while the server runs: they
  are inputs to importing a pre-2.5.0 deployment on first boot, and are
  otherwise only checked for validity at startup. Each Vault's own exclusion
  patterns and Git mode are what actually apply. Saving one no longer creates
  or reconfigures a Git repository as a side effect. `HATCHDOOR_GIT_AUTHOR_NAME` and `HATCHDOOR_GIT_AUTHOR_EMAIL` are
  unaffected: they remain the commit identity a Vault without its own falls
  back to, and a change to either still reaches the next Git turn without a
  restart.
- The `update_note` and `archive_note` MCP tools now run their write off the
  request thread, the way the equivalent HTTP routes always have, so a large
  note no longer holds up other MCP traffic while it is written. Both tools
  and both routes share one implementation now, which makes three of their
  error payloads agree where they used to differ. From these two MCP tools: a
  "note not found" now names the Vault it looked in; refusing to archive into a
  folder the Vault's own exclusion patterns hide now uses the same wording the
  HTTP route uses; and a crash while scanning the Vault is now reported as the
  retryable `vault_read_unavailable` the HTTP route already returned, instead
  of a generic internal error. Success responses and every other field are
  unchanged.

## v2.5.0 - 2026-08-17

- Attachment embeds written the way Obsidian writes them now render. A bare
  `![[Some document.pdf]]` resolves by filename anywhere in the Vault, rather
  than only next to the note, so a vault using a single top-level attachments
  folder no longer shows broken images and PDFs. Note-relative paths keep
  working, a leading `/` reads from the Vault root, and where a filename is
  carried by several files the one nearest the note wins.
- First startup now makes each Vault browseable from its structure-only cache
  before vector embedding finishes. Model setup no longer launches a duplicate
  legacy single-Vault index that held the shared SQLite writer for the entire
  embedding pass.
- Existing-Git remote synchronization now selects the unique repository remote
  matching the Vault's configured HTTPS URL instead of assuming `origin`, so an
  unrelated operator-owned SSH `origin` no longer blocks a migrated Vault.

### ⚠️ Breaking changes — action required on upgrade
- **A read-only MCP token can no longer upload attachments over HTTP.** The
  multipart attachment endpoint now accepts an MCP bearer token only while MCP
  and MCP writes are both currently enabled. Previously it accepted that token
  whenever MCP was enabled, including in read-only mode, while the
  `import_attachment` MCP tool already refused: the same credential performing
  the same action got two different answers depending on which surface it came
  through. Disabling MCP, or MCP write mode, is now an immediate revocation of
  that credential's upload capability, checked per request.
  Unaffected: the web bearer token, which still works regardless of MCP write
  mode; the web UI's own paste and drop upload, which uses it; and deployments
  with no token configured, where the route stays open as before.
  **Action:** if an agent uploads attachments over HTTP using the MCP bearer
  token, either enable MCP write mode or move that workflow to the web bearer
  token. Call `get_attachment_import_config` to see the methods and limits
  currently available to a session.
- **A single-Vault deployment is imported into a Vault collection on first
  start.** Your existing vault becomes the first Vault in a registry Hatchdoor
  stores alongside the cache, and the per-vault environment variables it was
  configured with (`HATCHDOOR_EXCLUDE`, the `HATCHDOOR_GIT_*` family) are read
  once and stored as that Vault's own settings. Hatchdoor then serves a
  restricted recovery screen until those obsolete environment lines are removed
  and the container is restarted. They are
  per-Vault questions now, and a server-wide answer cannot survive a second
  Vault. Nothing on disk moves and no note is touched; the import only writes
  the registry. If it cannot be proven safe, Hatchdoor starts and says what
  stopped it rather than guessing.
  **Action:** leave the variables in place for the first upgraded start, then
  remove the variables named by Hatchdoor and start it again. Change them in
  Settings instead, per Vault.
- **Hatchdoor will not activate Vaults while imported per-Vault settings are
  still set in the environment.** Once your Vault owns them, an `.env` value does nothing:
  the file and the running server disagree, and every later change made in
  Settings looks overridden by a line that has no effect. Rather than ignore
  them quietly, Hatchdoor stops and names each one. `VAULT_PATH` is exempt,
  since Compose sets it on every deployment.
  **Action:** start once so the import runs, then delete the named
  `HATCHDOOR_GIT_*` and `HATCHDOOR_EXCLUDE` lines from your `.env` and start
  again.

### Added
- **Hatchdoor holds more than one Vault.** Add, pause, and disconnect Vaults
  from Settings, then browse them together or one at a time. A single-Vault
  install is unchanged: the collection interface appears only once there is a
  collection to show.
- **A Vault can be backed by Git.** Connect a repository you already have,
  clone one for Hatchdoor to manage, or keep local history inside a folder you
  own. Remotes that need one take an access token, and a sync console reports
  what actually happened rather than a status light.
- A **New note** button now sits at the bottom of the sidebar, always reachable
  without scrolling. The per-folder `+` stays for creating in a specific folder.
- A **changes panel**, opened from the sidebar rail, listing notes that changed
  on disk. It replaces the old "Last Modified" sidebar list. It deliberately
  carries no unread count yet: Hatchdoor cannot currently tell an agent's write
  from your own, and a count that ticked up every time you saved a note would
  mean the opposite of what it should.
- Recently viewed is now collapsible, and remembers whether you folded it away.

### Changed
- **The sidebar says what you are browsing.** A Scope zone at the top switches
  between one Vault and all of them, each row ending in a note count or the
  reason there is no count to give. On phones it moves into the topbar as a
  scope row and a bottom sheet, since the sidebar is a drawer there.
- **The explorer becomes a per-Vault accordion when you browse everything**,
  one Vault unfolded at a time. Narrow to a single Vault and the accordion
  disappears: a collection of one is just a vault.
- **The graph draws every Vault as its own labelled island** in one field,
  rather than merging separate collections into a single cloud of dots.
- **Search filters by Vault without changing what you are browsing.** A rail
  beside the results on desktop, a Scope field on phones. It narrows only the
  results in front of you, changes no ranking, and is forgotten when the dialog
  closes.
- **Search says so when a Vault could not answer**, naming it instead of
  quietly returning fewer results as though that were all there was.
- **Settings is the collection.** Every Vault is a section in the settings
  index, beside the sections that belong to the server itself.
- **The sidebar is restructured into three zones**: a fixed rail of whole-vault
  destinations at top, the scrolling note navigation in the middle, and the
  create action pinned at the bottom. Only the middle scrolls.
- **Stats and Graph moved** out of the sidebar header into that rail, as icons.
  Settings sits alongside them and is now a live link.
- Notes in the tree now carry a small index, so note rows and folder rows no
  longer look identical.
- The topbar's `···` menu on desktop is left-aligned sentence case with
  borderless rows, grouped into create/edit, utilities, and destructive actions
  — with Archive and Delete last. Mobile already looked like this; desktop did
  not.
- Interface icons are now Material Symbols Sharp rather than typed unicode
  characters, so they render consistently across platforms. Attribution is in
  the new `THIRD_PARTY_NOTICES.md`.
- The note **Properties** heading is now the disclosure itself; the separate
  Show/Hide button is gone.
- **The create-note dialog is rebuilt.** Labels are distinguishable from the
  fields, the fields are actually visible, the folder chooser is a dropdown with
  a "New folder…" option instead of a free-text box beside a wall of chips, and
  a live line shows the path you are about to create.

### Fixed
- The MCP server told agents to call `get_attachment_import_config` before
  uploading a file, but the Vault-scoping migration had removed the tool: an
  agent following the server's own instructions got "Unknown MCP tool". It is
  restored, now taking one `vault_id` and reporting the Vault-scoped upload
  path. It also reports the instance-wide write switch and the Vault's own
  mutation capability as separate fields, so an agent that cannot upload is
  told which of the two closed the door instead of guessing.
- An agent could not create its first Vault over MCP without guessing. The
  `source` and credential arguments of `create_vault` and `edit_vault` were
  advertised as bare objects described in a sentence, while the server rejects
  unknown fields: every guess came back as a rejection with nothing to correct
  against. Both now publish their real per-variant shapes, including which
  `mode` each source accepts, the poll-interval floor, and the fact that a
  managed Vault has no local-history mode.
- `edit_vault` replaces a Vault definition wholesale, so omitting a field
  cleared it. It said none of this; it now says to read the Vault from
  `list_vaults` and send back what you are not changing, and explains what
  `confirm_identity_change` consents to and that the Vault must be disabled
  first.
- `list_note_attachments` required MCP write mode, though it only reads. A
  read-only agent could not see what a note referenced without fetching the
  whole note. It is now a read tool, and works on Vaults that do not accept
  writes at all.
- Opening a note highlighted it in up to three sidebar lists at once. The
  highlight is now canonical in the folder tree only.
- Browsing no longer waits on the search index. A Vault's structure is
  published as soon as it is read, so you can open notes while its vectors are
  still being built.
- The explorer kept showing notes, and whole Vaults, after they had left the
  collection.
- A public demo deployment no longer reveals local filesystem paths, disabled
  Vaults, or notes on demoted layers.
- One unreadable file no longer fails the indexing run for the Vault
  containing it.

Your notes and folders are untouched, and the cache is not rebuilt. Two upgrade
notes apply: the one-time Vault import described above, which is automatic, and
the attachment authorization change, which matters only if an agent uploads over
HTTP with the MCP bearer token.

## v2.4.0 - 2026-07-27

### ⚠️ Breaking changes — action required on upgrade
- **The MCP attachment staging folder is removed.** Agents no longer import
  attachments by dropping a file into a shared, mounted inbox and calling
  `import_attachment` with a `staged_filename`. Instead, `import_attachment` now
  takes the file bytes directly as base64 (`content` + `target_relative_path`),
  and larger files use the existing multipart `POST /api/attachment`.
  **Action:** remove the `HATCHDOOR_MCP_ATTACHMENT_STAGING_PATH`,
  `HOST_ATTACHMENT_STAGING_PATH`, and `HATCHDOOR_MCP_ADVERTISE_HOST_PATHS`
  variables from your `.env`, and delete the attachments-inbox volume mount from
  your Docker Compose file. Any agent workflow that placed files in the inbox
  must switch to sending base64 via `import_attachment` (call
  `get_attachment_import_config` to see the methods and limits).
- **`HATCHDOOR_MCP_MAX_ATTACHMENT_BYTES` is renamed to
  `HATCHDOOR_MAX_ATTACHMENT_BYTES`** (it caps the web UI and HTTP uploads, not
  just MCP). **Action:** rename it in your `.env` if you set it; otherwise the
  old name is ignored and the default (10 MiB) applies.
- **The cache schema is upgraded from 7 to 8 for vault layers.** The generated
  SQLite cache is rebuilt on the first startup after upgrade. Source Markdown is
  unchanged, but the initial indexing run re-embeds the vault.

### Added
- First-run semantic-search setup. Hatchdoor now asks the single user to accept
  the Gemma terms before downloading the default multilingual EmbeddingGemma
  model, shows model-download and indexing progress in the UI and logs, and
  keeps a local acceptance receipt with the persistent model files. Declining
  Gemma removes its partial files and starts the Nomic Embed Text v1.5 fallback;
  Nomic is explicitly identified as English-only and lower quality for
  multilingual vaults. Public images ship neither model.
- Direct attachment upload for agents: the `import_attachment` MCP tool accepts
  base64 file bytes inline (universal fallback for any MCP client), capped by the
  new `HATCHDOOR_MCP_MAX_BASE64_BYTES` (default 5 MiB, measured on the decoded
  file). `get_attachment_import_config` now enumerates both upload methods, their
  size limits, and which to use.
- Vault layers: add a `.hatchdoor-layer` marker to a folder to place its notes
  on a named, demoted surface. Browser routes remain default-surface only; MCP
  clients can explicitly select named layers.
- `HATCHDOOR_EXCLUDE` for comma-separated gitignore-style noise patterns,
  `HATCHDOOR_EMBED_LAYERS` to opt demoted layers out of vector embedding, and
  diagnostics via `GET /api/diagnostics` or the `layer_diagnostics` MCP tool.
- Layer-aware note-write and attachment responses, so automation can tell which
  surface a created, moved, archived, or uploaded item belongs to.
- Cross-platform inline previews for linked PDF vault assets, with internal PDF
  links resolving as vault assets rather than ordinary note links.

### Changed
- The `/mcp` request-body limit is raised to fit base64 attachment inflation so a
  legitimately sized upload is not rejected before the tool's own size check.
- `POST /api/attachment` now accepts the MCP bearer token as an alternative to
  the web bearer token, so an agent uploading larger files over HTTP can reuse
  its existing MCP credential instead of needing `HATCHDOOR_WEB_BEARER_TOKEN`
  provisioned separately. The rest of the web API is unaffected — this route
  was pulled out of the shared protected-routes group so the MCP token is not
  granted any broader access.
- Built-in noise exclusions now omit `.obsidian/`, `.trash/`,
  `.hatchdoor-trash/`, `.DS_Store`, `*.tmp`, and `*.sync-conflict-*` from the
  index. In particular, Markdown under `.obsidian/` or `.trash/` and Syncthing
  conflict copies are no longer searchable unless a deployment negates the
  relevant default with `HATCHDOOR_EXCLUDE`.
- FastEmbed is upgraded from v4 to v5. Each chunk is now embedded with its note
  title and heading path as context, improving retrieval relevance while
  preserving chunk-level search results.

## v2.3.0 - 2026-07-19

### Added
- The UI is now available during initial indexing and shows live token-weighted progress, note/chunk counts, and a measured ETA while vault and MCP data remain unavailable until the index commits.
- Indexing logs a human-readable heartbeat every minute and detailed performance diagnostics at debug level.
- MCP `search_notes` now supports exact tag, path-prefix, property-existence, and typed property-equality filters with explicit property projection.
- Added the metadata-only `query_notes` MCP tool and structured tags, aliases, and frontmatter properties to `get_note`.

### Changed
- Chunks are embedded individually to avoid batch-longest padding and reduce peak memory pressure.
- The cache schema is upgraded to version 6 for note-level frontmatter metadata. The first 2.3.0 startup automatically rebuilds the generated SQLite cache from the Markdown vault.
- Rust and frontend dependencies are refreshed within their current compatibility lines, including git2 0.21, resolving the current RustSec and npm audit findings.

## v2.1.1 - 2026-06-13

Security, performance, and operational hardening from the 2026-06-11 codebase audit, plus an iOS PWA download fix.

### Fixed
- Markdown downloads no longer arrive as HTML on iOS standalone PWAs. The service worker's SPA navigation fallback was intercepting `/api/*`, `/vault-assets/*`, and `/health` navigations (iOS ignores the `<a download>` attribute and treats the click as a navigation) and serving the cached `index.html`. Added a `navigateFallbackDenylist` so those requests reach the network.

### Security
- Added optional web API authentication: when `HATCHDOOR_WEB_BEARER_TOKEN` is set, all `/api/*` routes, `/vault-assets/*`, and note downloads require the token (via `Authorization: Bearer` header or `access_token` query parameter). The PWA prompts for the token on a 401. (F-01)
- Changed the default bind host to `127.0.0.1`; exposing on `0.0.0.0` is now an explicit opt-in documented to require auth or a reverse proxy. (F-01)
- Coalesced concurrent `/api/refresh` requests so a request loop can no longer trigger overlapping full reindexes. (F-02)
- Compared bearer tokens (MCP and web) in constant time. (F-06)
- Served SVG vault assets with `Content-Security-Policy: sandbox` and `Content-Disposition: attachment` to neutralize script execution on direct navigation. (F-09)
- Stopped leaking absolute filesystem paths and raw internal error strings in HTTP error bodies; details now go to logs only. (F-10)
- Capped `POST /api/resolve-batch` at 200 targets. (F-11)

### Performance
- Moved vault reindexing, embedding, and query embedding off the async runtime via `spawn_blocking`, holding the cache write lock only for the final swap so reads no longer freeze during a refresh. (F-03)
- MCP write tools now resolve the target note from the SQLite cache instead of rebuilding the full vault index from disk on every write. (F-04)
- Enabled SQLite WAL mode and added a pooled set of read connections so concurrent reads run in parallel instead of serializing on a single mutex. (F-05)

### Correctness & operations
- Validated `McpConfig` once at startup (failing fast when write mode is enabled without a bearer token) instead of re-parsing the environment on every MCP request. (F-07)
- Git sync now refuses to force-checkout over uncommitted manual edits to tracked vault files, surfacing them as an error instead of silently discarding them. (F-08)
- Moved the hard-coded `90-archive/` prefix to `HATCHDOOR_ARCHIVE_PREFIX`. (F-12)
- Added a CI workflow (fmt, clippy, test for the backend; lint, typecheck, test, build for the frontend) and a Docker Compose `healthcheck`. (F-13)
- The SSE vault-events stream now emits the current revision on broadcast lag instead of silently dropping it, so a slow client always resyncs. (F-16)
- `/health` now runs a `SELECT 1` against the cache so it reports unhealthy if the database is unreachable, and the binary gained a `--healthcheck` mode for the container probe. (F-17)

## v2.1.0 - 2026-06-xx

### Added
- Optional automatic git sync of the vault: successful MCP write tools commit and push changes to the configured remote with debounced batching, conflict-abort semantics, and an immediate flush of stranded commits on startup.
- `get_git_sync_status` MCP tool and a git-sync warning on write-tool responses.

### Changed
- Richer git sync status reporting with plural-aware commit messages.

### Fixed
- Enabled the git2 `https` feature for TLS remote transport.
- The vault watcher now ignores `.git/` so sync churn does not trigger reindexing.

## v2.0.0 - 2026-xx-xx

### Added
- SQLite read model (FTS5 + sqlite-vec embeddings) backing the vault index, with chunking, embedding, and hybrid/semantic/keyword retrieval.
- Streamable-HTTP MCP endpoint exposing read tools always and write tools (create/update/edit/replace-section/append/move/rename/delete notes and attachments) gated by env flag and bearer token.

### Changed
- Pinned the Rust toolchain to 1.96.0 and reformatted the tree.

## v1.1.0 - 2026-02-20

Compared with `v1.0.0`.

### Added
- Added a `Download .md` action in the note actions menu.
- Added a server download endpoint: `GET /api/note/{slug}/download`.

### Changed
- Switched markdown download flow to server-driven delivery for native mobile handoff.
- Updated frontend download trigger to use an anchor `download` flow instead of popup navigation.
- Added UTF-8-aware filename handling in `Content-Disposition` for markdown downloads.

### Fixed
- Improved iOS/Safari file handoff behavior for `.md` downloads by using attachment headers.
- Added and updated frontend/backend tests for the download path and response headers.