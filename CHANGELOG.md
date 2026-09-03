# Changelog

## v2.6.0 - 2026-09-03

The MCP release. Agents get five new tools, a modern protocol revision, and a `get_tree` that no longer hands over the entire Vault when you asked about one folder. Everyone else gets a Git schedule that actually fires, a read-only mount that actually mounts, and a write layer that stops refusing to rename a note because of the picture sitting next to it.

### ⚠️ Breaking changes — action required on upgrade
- **`GET /api/index-status`, `GET /api/git-status` and `GET /api/vault-status` are gone**, and all three return `404`. Each described a single Vault back when there was only ever one. The two Settings consoles they fed, **Search index** and **Versioning**, leave the Settings page with them, along with their two-second polling. Each Vault's own settings page keeps its condition, its last error, and its **Sync now**, **Try again** and **Rebuild search index** buttons, and the scope zone and explorer still show a Vault as indexing.
  **Action:** point uptime checks at `/api/startup-status`, which is unchanged and remains the unauthenticated startup probe, and anything asking about a specific Vault at `GET /api/v1/vaults`, which reports each Vault's condition, its last search or versioning error, and whether it is indexing. Nothing in Hatchdoor's configuration brings the old routes back. [#183]
- **`PATCH /api/settings` no longer asks you to confirm `git_init` or `git_downgrade`.** Those consequences belonged to the instance-wide versioning lifecycle that the retired **Versioning** console explained, and no current deployment reaches it. A save that turns on local versioning, or switches off remote versioning, now applies on the first request instead of answering `409` and waiting for a resend.
  **Action:** remove those two values from any API client, since an unknown consequence is refused as a validation error. `reindex` is unaffected and still confirmed exactly as before, and per-Vault Git changes keep their own confirmations on `/api/v1/vaults/{vault_id}`.
- **MCP protocol revisions `2025-03-26` and `2025-06-18` are no longer served.** The endpoint advertises and accepts exactly `2026-07-28` and `2025-11-25`. A client pinned to a dropped revision is refused on the protocol-version header rather than silently downgraded.
  **Action:** if an MCP client stops connecting after this upgrade, check which revision it pins and update it. Nothing in Hatchdoor's own configuration restores the dropped ones.
- **Notes inside a `get_tree` result no longer carry `vault_id`.** It was a third of the payload, and the tree already names its Vault once at the top.
  **Action:** read the Vault from the tree root. Flat results that mix Vaults in one list, `search_notes` and `recently_modified`, keep their per-note `vault_id` and need no change. [#211]
- **The instance-wide Git and exclusion environment variables no longer do anything at runtime.** `HATCHDOOR_GIT_SYNC_ENABLED`, `HATCHDOOR_GIT_REMOTE`, `HATCHDOOR_GIT_BRANCH`, `HATCHDOOR_GIT_HTTPS_USERNAME`, `HATCHDOOR_GIT_HTTPS_TOKEN`, `HATCHDOOR_GIT_DEBOUNCE_SECONDS` and `HATCHDOOR_EXCLUDE` stay in Settings and keep their values, but nothing acts on them while the server runs. They are inputs to importing a pre-2.5.0 deployment on first boot, and are otherwise only checked for validity at startup. Each Vault's own exclusion patterns and Git mode are what apply, and saving one of these no longer creates or reconfigures a Git repository as a side effect.
  **Action:** set exclusions and Git mode per Vault, in that Vault's settings. `HATCHDOOR_GIT_AUTHOR_NAME` and `HATCHDOOR_GIT_AUTHOR_EMAIL` are unaffected. They remain the commit identity a Vault without its own falls back to, and a change to either still reaches the next Git turn without a restart. [#185]

### Added
- **Read and write a note's properties without touching its body.** `get_frontmatter` returns the tags, aliases and other keys alone; `update_frontmatter` merges keys in and leaves your Markdown byte-for-byte identical. [#175]
- **`get_frontmatter` also hands back the note's `content_hash`.** Every hash-protected write wants one, and the only way to get one was to pull the whole note body over the wire. A migration over hundreds of notes no longer reads every body to learn a 16-character string. [#227]
- **`get_attachment` pulls a file back out of a Vault**, the mirror image of importing one. A download URL by default, base64 inline for clients that cannot fetch out of band. [#176]
- **`batch` runs up to 50 reads or 20 writes as one call and one commit.** Content hashes chain inside it, so an agent can create a note and edit it again later in the same batch without reading it back. Best-effort, no rollback. [#177]
- **`get_tree` takes `folder`, `max_depth` and `include_notes`.** Asking about one folder used to cost you the whole Vault. On a 530-note Vault the orientation call is 13x smaller than the full tree. [#192], [#211]
- **`refresh_vault` asks one Vault for its next index turn.** Collection reads have always said when they have fallen behind the Markdown, and an agent had no way to act on that: `sync_vault` and `retry_vault` both resolve a Git poll interval first and refuse any Vault without a remote, so a plain local Vault had no MCP path to a rebuild at all. It answers `queued` on an idle Vault and `coalesced` when a turn is already pending, so repeat requests cannot pile turns onto one Vault. [#228]
- **Every MCP tool ships an `outputSchema`**, so a client can validate a result instead of inferring its shape. [#167]
- **The MCP endpoint speaks protocol revision `2026-07-28`**: stateless discovery, no handshake, and one opt-in `subscriptions/listen` stream in place of SSE subscribe and unsubscribe. Clients on `2025-11-25` are unaffected. [#169], [#170]
- **MCP now has rate limits.** 120 calls a minute per token, 8 concurrent calls and 2 concurrent searches, refused with `429` and a `Retry-After`. Set `HATCHDOOR_MCP_RATE_LIMITS_ENABLED=false` to switch them off. [#171]
- **The Vault asset route accepts an MCP bearer token**, so an agent can fetch the URL `get_attachment` hands it without also holding the web token. It spends the same quota and size limits as any MCP call. [#174]
- **The public documentation Vault gained Guides, Concepts and a settings reference**, and the README links into it rather than duplicating it.
- **The public demo and the documentation site link to each other now.** Each demo Vault points at the documentation page behind its layout, and the documentation's layout table links to the live Vault demonstrating each method.

### Fixed
- A Vault on a `:ro` Docker bind mount refused to come up at all, which is an awkward position for a product that supports read-only browsing. Read-only mounts answer the write probe with `EROFS`, and only "permission denied" counted as present but not writable. Such a Vault now activates read-only. [#178]
- If you redeployed more often than your Git poll interval, a scheduled sync never fired once. Every process start re-armed the countdown from zero, so the only Git turns that ever ran were activations and manual syncs. The schedule was, in the strict sense, decorative. Each Vault now remembers when its last turn completed and resumes the countdown across a restart. [#200]
- Straight after a restart, a Vault whose Git credentials were failing reported `pending` with no error, for up to a day on the default schedule. Activation now republishes the last known outcome, message included. [#200]
- A Vault's read model could wedge permanently, and one new note was enough to do it. A slug comes from a filename alone, so two notes with the same stem in different folders compete for it, and the index build tried to give it to the arriving note while the departing one still held it. That failed the whole Index turn, and the next turn seeded from the same snapshot and failed at the same note, forever: `get_tree`, `get_graph` and `get_stats` reported `partial` at a frozen `collection_revision` while exact note reads stayed correct. The build now releases a moving slug before any note claims it, and a wedged cache recovers on its first turn with nothing deleted and no cache wiped. [#226]
- An Index turn held the Vault's write lock for the whole turn, embedding included, so a write arriving during a multi-minute pass on a CPU-only host did not slow down, it parked. The caller's transport gave up on a write that then landed anyway, which is an at-least-once hazard the moment the caller retries. The lock now covers the turn's read phase only, and a Vault that took a write while it was released publishes itself stale rather than fresh and lets the watcher's catch-up turn sort it out. [#223]
- A note holding a real image, PDF or archive could not be moved, renamed, archived or deleted, and neither could the file itself. The check run after an asset move read the file as text, so any byte that was not valid UTF-8 rolled the whole write back with "refusing to move unsafe source". The suite stayed green on this because every attachment fixture wrote the ASCII string `png` into a file named `.png`. The check now asserts an ordinary file and reads nothing. [#220]
- A note referencing an asset outside its own folder could not be moved at all. The planner assumed every referenced asset was a sibling and built its destination by joining the reference onto the note's folder, so a link such as `../Attachments/image.png` carried its `..` all the way down to the move primitives, which refuse anything but plain names. An asset now travels with a note only when it already lives inside that note's own folder, or a subfolder of it. One kept elsewhere, such as the shared attachments folder of the usual Obsidian layout, stays exactly where it is; only the moving note's own link is repointed, and no other note is touched. A note in the Vault root has the whole Vault as its folder, so everything it references still travels when it moves. [#225], [#231]
- `rename_note` refused any note that kept its picture beside it, and every note in the Vault root. A rename leaves the note in its folder, so an asset in that folder was already sitting at the destination computed for it, and the collision guard could not tell that apart from a genuine collision. A move to nowhere is now recognised as one: nothing moves, nothing is rewritten, and `moved_assets` reports `0`. A different file at an asset's destination still refuses, as before. [#238]
- One rename could rewrite a Vault's prose out of the style it was written in. Every backlink to the moved note was retargeted to its full vault-relative path, so `[[Some Note]]` came back as `[[folder/subfolder/Some Note]]`. The links still resolved, but the prose got noisier on every rename and nothing rewrote it back. A backlink now keeps the form it was authored in, with the full path as the fallback where the new title is one another note already carries. Aliases, anchors and fenced code are untouched. [#235]
- A writer saving faster than the watcher's quiet window deferred freshness for the whole burst. Every qualifying event restarted the 500 ms timer with no upper bound, so no Index turn was armed and the published snapshot could not advance until the writing stopped. The documented ceiling on that window is now enforced, and whichever of the two fires first wins. [#229]
- An agent that edited a note was told Hatchdoor was still being set up, on instances that finished setting up months ago, and was then handed model-setup tools that could change nothing. Following that advice produced a second, differently wrong answer. The routine reindex behind the write was sharing one status field with first-run model setup. [#191]
- After the backend restarted, the Vault list and its counts stopped updating until you reloaded the page. A new server counts revisions from zero again, and the client read that as stale. [#194]
- Opening a Vault's settings page mid-refresh could leave it describing a Vault the Settings index disagreed with for the rest of the visit. [#194]
- Dragging the sidebar resizer moved the note path in the topbar and left the pane itself where it was. The layout grid redeclared `--sidebar-width` at its 280px default, and a local declaration beats the live value inherited from the shell.
- Asking for a folder that no Vault has picked one Vault at random and blamed it by id. The refusal now names none of them, because none is more at fault than the others. [#211]
- Uploading a file called `.hatchdoor-layer` through the HTTP API silently reclassified a whole subtree as a demoted layer. It now returns `400 layer_marker_write`, as the MCP tools always did.
- In demo mode, an attachment in a hidden layer was still served to anyone who knew its URL. Asset requests now go through the same browse surface as note reads.
- Every semantic search embedded your query twice, once to retrieve and again during diversity backfill. It embeds it once.
- Setting `HOST=localhost` failed to parse and the server refused to start. It is accepted now, and the container health probe follows the listener's address family instead of guessing at it.
- A large note written through `update_note` or `archive_note` held up other MCP traffic while it was written. Both now write off the request thread, the way the HTTP routes always have, which also made three of their error payloads agree with their HTTP twins.
- The length of your configured bearer token could be measured from outside, because the comparison returned early on a length mismatch. Both sides are hashed to a fixed width first now.

### Under the hood
No behaviour change in any of these, but they are why the list above is as short as it is.

- The `/mcp` protocol boundary is the `rmcp` library rather than a hand-written JSON-RPC layer. Tool names, arguments, response shapes and the per-request security ordering are unchanged. [#168]
- The legacy single-Vault indexing and Git lane is deleted. Hatchdoor had been carrying a second, unreachable copy of both since v2.5.0. [#185]
- Every write, read and management operation crosses one Vault-qualified core now, with HTTP and MCP as thin adapters over it. That is why three MCP error payloads above could stop disagreeing with their HTTP twins. [#184], [#186], [#187], [#188]
- Index turns and Git turns run behind a single Vault work executor. [#197]
- The frontend reads the Vault collection through one client instead of several. [#198]
- The eval harness is behind a non-default `eval` cargo feature, cutting 17 crates from a default build. [#195]
- YAML parsing moved from the archived `serde_yaml` to `serde_yaml_ng`. [#196]
- The unreachable search filters, and the second semantic retrieval path they gated, are deleted. [#210]

[#167]: https://github.com/BatterWorks/Hatchdoor/issues/167
[#168]: https://github.com/BatterWorks/Hatchdoor/issues/168
[#169]: https://github.com/BatterWorks/Hatchdoor/issues/169
[#170]: https://github.com/BatterWorks/Hatchdoor/issues/170
[#171]: https://github.com/BatterWorks/Hatchdoor/issues/171
[#174]: https://github.com/BatterWorks/Hatchdoor/issues/174
[#175]: https://github.com/BatterWorks/Hatchdoor/issues/175
[#176]: https://github.com/BatterWorks/Hatchdoor/issues/176
[#177]: https://github.com/BatterWorks/Hatchdoor/issues/177
[#178]: https://github.com/BatterWorks/Hatchdoor/issues/178
[#183]: https://github.com/BatterWorks/Hatchdoor/issues/183
[#184]: https://github.com/BatterWorks/Hatchdoor/issues/184
[#185]: https://github.com/BatterWorks/Hatchdoor/issues/185
[#186]: https://github.com/BatterWorks/Hatchdoor/issues/186
[#187]: https://github.com/BatterWorks/Hatchdoor/issues/187
[#188]: https://github.com/BatterWorks/Hatchdoor/issues/188
[#191]: https://github.com/BatterWorks/Hatchdoor/issues/191
[#192]: https://github.com/BatterWorks/Hatchdoor/issues/192
[#194]: https://github.com/BatterWorks/Hatchdoor/issues/194
[#195]: https://github.com/BatterWorks/Hatchdoor/issues/195
[#196]: https://github.com/BatterWorks/Hatchdoor/issues/196
[#197]: https://github.com/BatterWorks/Hatchdoor/issues/197
[#198]: https://github.com/BatterWorks/Hatchdoor/issues/198
[#200]: https://github.com/BatterWorks/Hatchdoor/issues/200
[#210]: https://github.com/BatterWorks/Hatchdoor/issues/210
[#211]: https://github.com/BatterWorks/Hatchdoor/issues/211
[#220]: https://github.com/BatterWorks/Hatchdoor/issues/220
[#223]: https://github.com/BatterWorks/Hatchdoor/issues/223
[#225]: https://github.com/BatterWorks/Hatchdoor/issues/225
[#226]: https://github.com/BatterWorks/Hatchdoor/issues/226
[#227]: https://github.com/BatterWorks/Hatchdoor/issues/227
[#228]: https://github.com/BatterWorks/Hatchdoor/issues/228
[#229]: https://github.com/BatterWorks/Hatchdoor/issues/229
[#231]: https://github.com/BatterWorks/Hatchdoor/issues/231
[#235]: https://github.com/BatterWorks/Hatchdoor/issues/235
[#238]: https://github.com/BatterWorks/Hatchdoor/issues/238

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