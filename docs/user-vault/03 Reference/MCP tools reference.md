---
tags: [type/reference, topic/mcp]
---

# MCP tools reference

Every tool Hatchdoor's MCP endpoint (`/mcp`) advertises, once [[Connect your agent|MCP is connected]]. This is a lookup reference, not a tutorial — for the first read-only connection, start with [[Connect your agent]].

## Permission model

Two independent gates decide what a call can do:

- **`HATCHDOOR_MCP_ENABLED`** — MCP as a whole. Off by default. A disabled instance answers no MCP calls at all.
- **`HATCHDOOR_MCP_WRITE_ENABLED`** — write tools specifically. Off by default even when MCP is on. A write tool called while this is off returns the JSON-RPC error `MCP write tools are disabled by HATCHDOOR_MCP_WRITE_ENABLED`.

A third, per-Vault gate sits underneath write mode: a Vault's own `capabilities.mutate` (from its source type and lifecycle phase — a `pull_only` Git Vault, or one not yet `ready`, refuses writes even with `HATCHDOOR_MCP_WRITE_ENABLED=true`). Check `list_vaults` for a Vault's current capabilities before writing to it.

> [!note]
> The full tool catalogue is always advertised, even before the model-setup completes and even at zero Vaults, so a client that caches tools at connection time never needs to reconnect. Before setup finishes, only `get_model_setup_status`, `accept_gemma_terms`, `decline_gemma_terms`, the manual tools (`read_docs` and `search_docs`), and the Vault collection discovery/management tools (`list_vaults` and friends) actually run; every other tool returns "Hatchdoor is still being set up." until a model is selected. `refresh_vault` is the one management tool outside that exception: the index turn it asks for cannot run without a search model, so before setup finishes it answers like any other content tool.

There is no selected, sole, or default Vault. Every tool below that touches content takes an explicit `vault_id`; every collection-level tool takes an explicit `scope` (a Vault ID or the literal `all`).

## Model setup

Always available, regardless of `HATCHDOOR_MCP_ENABLED`'s write posture. Along with the manual tools below and the Vault collection tools, these are the only tools that run before first-run setup completes.

| Tool | Purpose |
| --- | --- |
| `get_model_setup_status` | Report setup state, the Gemma terms/policy links, and the Nomic fallback notice. No parameters. |
| `accept_gemma_terms` | Accept Gemma's terms, then download it and begin indexing. No parameters. |
| `decline_gemma_terms` | Decline Gemma, remove any partial Gemma download, download Nomic Embed Text v1.5 instead, and begin indexing. No parameters. |

> [!warning]
> Once a model is selected, calling either `accept_gemma_terms` or `decline_gemma_terms` again returns an error — changing models after setup is not supported.

## Hatchdoor's manual

Hatchdoor carries its own manual, the pages you are reading now, for the version that is running. Two read-only tools read it. They take no Vault, need no search model, and answer whether or not write mode is on and while model setup is still pending, so an agent can look something up before any Vault exists.

| Tool | Parameters | Returns |
| --- | --- | --- |
| `read_docs` | `page` (optional) | With no `page`: the Home page as Markdown, plus `pages`, the `name` and `title` of every page. With a `page`: that page's `name`, `title` and `markdown`. |
| `search_docs` | `query` | `results`: up to five best-matching pages, best first, each with `name`, `title` and an `excerpt`, a line of the page that matched. |

Every page has a short name made from where it sits in the manual, such as `guides/how-to-set-up-a-git-backed-vault`. Links between pages come back as ordinary Markdown links to those names, so `[[The layer system]]` reads `[The layer system](concepts/the-layer-system)`, and a link to a heading adds an anchor, as in `reference/mcp-tools-reference#batch`. Pass either form back to `read_docs`; the anchor is ignored. A name that matches no page is refused with the error code `docs_page_not_found`. The page `whats-new` lists what changed in each release, newest first, and the opening instructions point agents to it. On an install upgraded to 2.8.0, the opening instructions of one session also carry a single sentence saying the optional usage report exists; [[Usage report reference#Where you are asked]] explains it.

`search_docs` matches whole words, ignoring case, a plural `s`, and common words such as `how` and `the`. A page whose title has more of your words ranks above one with fewer, then headings decide, then how often the page mentions them. A word with punctuation inside it, as tool names, setting names, error codes and file names have, is also looked for exactly as typed, ignoring case, in each page's title and text, code blocks included: pages that hold it rank above every page that only holds its parts, and their `excerpt` is the first line that shows it, taken from a code block only when no other line does. So search for a tool, setting or error code by its exact name. It is not the semantic search `search_notes` uses, so search for the words a page would use rather than for a paraphrase. A query that matches nothing returns an empty `results` list, not an error.

The manual is never part of your Vaults. It does not show up in `search_notes`, `get_tree`, `get_stats`, `get_graph` or `list_vaults`, and no tool can change it. Neither tool can go inside `batch`.

## Vault collection: discovery and management

`list_vaults` is always available. The other nine require `HATCHDOOR_MCP_WRITE_ENABLED`; without it they return the same "MCP write tools are disabled" error as content write tools.

A listed Vault with a remote to poll also carries two RFC 3339 UTC timestamps describing its Git schedule: `last_checked_at`, when Hatchdoor last tried to check the remote — whether that check succeeded or failed, so read it alongside the Vault's Git status rather than as a successful sync — and `next_attempt_at`, when the next scheduled check is due. `last_checked_at` is absent until the first check completes; both are absent for a Vault with no remote and in demo mode. They are described in full under **Git schedule fields on a listed Vault** in [[HTTP API reference]], whose Vault shape `list_vaults` returns verbatim.

A listed Vault's `index_turn` is `running` while it indexes and `waiting` while its indexing is queued behind another Vault's or paused to let one through; it is absent when nothing is queued. A `waiting` Vault is not stuck, and its `search` status still says what it can answer meanwhile. The same holds while it is `running`: a Vault that was already searchable reads `search: "stale"` with `capabilities.search: true` for the whole reindex, and `search: "indexing"` only ever means a first index with nothing to search yet. **The `index_turn` field on a listed Vault** in [[HTTP API reference]] describes it.

Each listed Vault also carries its link style: `link_style` (`wikilink` or `markdown`) and `link_path_form` (`relative`, `absolute` or `shortest`). Write new note links and embeds in that form, `[[Note]]` and `![[file.png]]` for `wikilink`, `[Note](path/Note.md)` and `![](path/file.png)` for `markdown`, so the Vault keeps one style; Hatchdoor stores what you send as-is and never converts it. Both fields are absent for a Vault Hatchdoor cannot read and in demo mode. **Link style fields on a listed Vault** in [[HTTP API reference]] describes them, and [[Supported Markdown reference#Which link style Hatchdoor writes]] explains how the style is read.

| Tool | Gating | Purpose |
| --- | --- | --- |
| `list_vaults` | Always | Every Vault's ID, name, status, redacted source, capabilities, Git schedule, and link style, plus the registry's `registry_revision`. Call this first — every write below needs a fresh `expected_registry_revision`. |
| `create_vault` | Write mode | Create a Vault definition. The registry assigns the Vault ID; read it back from `list_vaults`. |
| `edit_vault` | Write mode | Replace one Vault definition wholesale (not a patch — send back every field you want to keep). |
| `enable_vault` | Write mode | Enable a disabled Vault definition. |
| `disable_vault` | Write mode | Disable a Vault without deleting its files. |
| `disconnect_vault` | Write mode | Remove a Vault from the registry without deleting local files, checkouts, Git history, or credentials outside the registry record. |
| `sync_vault` | Write mode | Request one Vault's Git work now: a remote sync if it has a remote, a local commit if it only keeps history. |
| `retry_vault` | Write mode | Retry that same operation for one eligible Vault. |
| `publish_recovery_branch` | Write mode | Publish a Two-way Vault's side of a sync conflict to its recovery branch, so the conflict can be resolved on the Git host. |
| `refresh_vault` | Write mode | Request one Vault's next index turn, so the snapshot the collection reads project from is rebuilt from its Markdown. |

### `create_vault`

| Field | Required | Notes |
| --- | --- | --- |
| `expected_registry_revision` | Yes | Most recent `registry_revision` from `list_vaults`. A stale value rejects the create rather than racing another writer. |
| `name` | Yes | |
| `enabled` | No | Default `true`. |
| `source` | Yes | See [[#Vault source shapes]]. |
| `exclude_patterns` | No | Glob patterns, in gitignore syntax, this Vault's index ignores. Default `[]`. |
| `https_credentials` | No | `{ "username"?: string, "token": string }`. Write-only — never echoed back; `list_vaults` reports only whether one is configured. |
| `archive_folder` | No | Per-Vault override of the instance-wide archive folder used by `archive_note`. Absent inherits the instance default. |
| `commit_identity` | No | `{ "name": string, "email": string }`. Per-Vault override of the instance-wide Git author identity. Absent inherits the instance default. |

### `edit_vault`

Same fields as `create_vault`, plus `vault_id` (the Vault to edit) in place of `enabled`. This is a **wholesale replace**, not a patch: read the Vault from `list_vaults`, change what you mean to change, and send the rest back unchanged. Leaving `exclude_patterns`, `archive_folder`, or `commit_identity` absent *clears* the stored value rather than preserving it.

`https_credentials` is the one exception — it takes a three-state action instead of a plain value, so a stored secret never has to be resent just to survive an edit:

| `action` | Effect |
| --- | --- |
| `keep` | Leave the stored credential untouched. Default if the field is omitted. |
| `remove` | Delete the stored credential — a remote that needs auth will then fail to sync. |
| `replace` | Set a new `{ "username"?, "token" }`. |

> [!warning]
> Changing a Vault's `source` path, repository URL, branch, or subdirectory is an **identity change** — it repoints the Vault at different content. It is refused unless `confirm_identity_change: true` **and** the Vault is already disabled (`disable_vault` first); an identity change on an enabled Vault is refused regardless of the confirm flag. Changing `mode`, `poll_interval_secs`, `name`, credentials, exclusions, archive folder, or commit identity is not an identity change and needs neither.

### `enable_vault` / `disable_vault` / `disconnect_vault`

All three take just `vault_id` and `expected_registry_revision`.

### `sync_vault` / `retry_vault`

Both take just `vault_id`. On a Vault with a remote, `sync_vault` requests an immediate poll instead of waiting for `poll_interval_secs`, and `retry_vault` retries an operation the scheduler admitted but that failed (say a transient network error), rather than waiting for its own backoff.

On a Vault with no remote but with Git history (an `existing_git` Vault in `local_history` mode), both request an immediate local commit instead. No remote is contacted, and asking explicitly also lifts the five-minute pause that follows a failed commit. Only a Vault with no Git at all (a plain `local` source) is refused, with `capability_unavailable`. For rebuilding the search index of any Vault, Git-backed or not, use `refresh_vault`.

### `publish_recovery_branch`

Takes just `vault_id`. When a Two-way Vault's sync stops on a conflict (`git_error.code` is `managed_git_conflict`), Hatchdoor keeps its own side only in its checkout. This tool pushes that side to a branch of its own on the Vault's remote, `hatchdoor-recovery/<configured branch>/<vault_id>`, so a person or an agent with access to the Git host can merge it into the configured branch. Once the configured branch contains the resolution, the Vault's next sync goes through on its own; `retry_vault` makes that happen now.

It is allowed only while the Vault's `capabilities.publish_recovery` is true, and refused with `capability_unavailable` otherwise. Pending saves are committed first, so the branch carries everything written so far. The push only ever fast-forwards the recovery branch: it never force-pushes, never touches the configured branch, and never deletes a branch, including after the conflict clears. Calling it again later publishes newer saves to the same branch.

Like `sync_vault`, it returns as soon as the request is admitted (`schedule` is `queued`, or `coalesced` with a request already pending). Read the outcome from `list_vaults`: the Vault's `recovery_branch` names the `branch`, the `published_commit`, the `conflicting_commit` on the remote and `published_at`, or carries an `error`. `managed_git_recovery_diverged` means someone added commits to the recovery branch, so Hatchdoor left it alone rather than overwrite them; `managed_git_recovery_push_rejected` carries the remote's own reason, such as a token that may not create branches. `recovery_branch` clears once a sync succeeds, and is not kept across a restart.

The tool is not allowed inside `batch`.

### `refresh_vault`

Takes just `vault_id`. It asks Hatchdoor to re-scan that Vault's Markdown and republish the snapshot the collection reads: the one `get_tree`, `get_graph`, `get_stats`, `recently_modified` and `search_notes` project from. It contacts no Git remote, and unlike `sync_vault` it works on a Vault with no Git at all.

**Call it when a collection read reports itself stale.** Those reads carry `partial` and a `participants` list (see [[#Read-only content tools]] below). A Vault whose entry reads `stale` is answering from a snapshot known to be behind its files; `refresh_vault` is how an agent asks for that to be fixed instead of waiting and hoping.

It returns as soon as the turn is admitted, not when the turn finishes:

| `schedule` | Meaning |
| --- | --- |
| `queued` | The Vault's next index turn was admitted. |
| `coalesced` | An index turn for this Vault was already pending; this request joined it rather than queueing a second one. |

So the response tells you the request landed, not that the index is rebuilt. To see the outcome, re-read a collection read and check its freshness fields again.

A Vault that is not currently browsable — its `capabilities.browse` is false, which covers a missing or unreadable directory and a Vault whose runtime has not come up — is refused with `capability_unavailable`, marked retryable because the Vault may become browsable again.

> [!note]
> `refresh_vault` rebuilds the read model from the Markdown that is on disk. It does not repair a Vault whose index turn is failing for its own reasons: a turn that fails deterministically will fail the same way again.

## Vault source shapes

`source` on `create_vault`/`edit_vault` is tagged on `type`; each shape rejects unknown fields (`additionalProperties: false`), so a guessed field name fails loudly rather than being silently ignored.

**`local`** — a plain directory on this machine. Hatchdoor never runs Git for it.

```json
{ "type": "local", "path": "/data/vault" }
```

**`existing_git`** — a Git working copy that already exists on this machine; Hatchdoor uses it in place and never clones it.

```json
{
  "type": "existing_git",
  "repository_path": "/data/vault",
  "repository_url": "https://example.com/notes.git",
  "branch": null,
  "vault_subdirectory": null,
  "mode": "pull_only",
  "poll_interval_secs": 900
}
```

`mode` is one of `local_history` (commits locally, never contacts a remote), `pull_only` (also fetches), or `two_way` (also pushes). `repository_url` is required for `pull_only`/`two_way`; it may be `null` only for `local_history`.

**`managed_git`** — a remote repository Hatchdoor clones and owns the checkout of. No `local_history` mode — a managed Vault exists specifically to track a remote.

```json
{
  "type": "managed_git",
  "repository_url": "https://example.com/notes.git",
  "branch": "main",
  "vault_subdirectory": "notes",
  "mode": "pull_only",
  "poll_interval_secs": 900
}
```

`mode` is `pull_only` or `two_way`. `repository_url` must be a credential-free HTTPS URL — embedded credentials are rejected; supply a token through `https_credentials` instead.

`branch` and `vault_subdirectory` are `null`/absent-able on every Git shape: `null` tracks the remote's default branch, or uses the repository root, respectively. `poll_interval_secs` has a floor of `60` and defaults to `86400`; it is ignored for `local_history`, which has no remote to poll.

## Read-only content tools

Available whenever MCP is enabled, independent of write mode.

| Tool | Required parameters | Purpose |
| --- | --- | --- |
| `search_notes` | `scope`, `query` | Search one Vault or all enabled Vaults. Optional: `mode` (`semantic` default or `keyword`), `limit` (1–50, default 10), `per_note_cap` (1–10, default 2), `layers` (array of layer names to include), `detail` (`compact` default or `full`). Hits are compact by default: each carries `vault_id`, `note_slug`, `note_title`, `note_path`, `heading_path`, `score`, `layer` and a `snippet`, which is enough to pick a note and read it with `get_note`. See [Compact and full search hits](#compact-and-full-search-hits). Each result's `score` runs 0 to 1: in semantic mode it is the cosine similarity between the query and the chunk, so it can be compared across searches; in keyword mode it is relative to the best hit of that search, which scores 1. A query that is a single `#tag` (letters, digits, `-`, `_` and `/` after the `#`) runs as a tag match whatever `mode` was requested: every hit scores 1 and the response reports `"mode": "tag"`. Every other query reports the mode it asked for. `tag` is never accepted as a requested mode. |
| `get_note` | `vault_id`, `slug` | Read one exact note's authoritative Markdown. `note.metadata` holds the frontmatter's `tags`, `aliases` and every remaining top-level key under `properties`, the same values `get_frontmatter` returns, so a note with no frontmatter block answers with two empty lists and an empty `properties` object. `note.metadata` is `null` when the frontmatter does not parse: the note still comes back whole, with its `content` and `content_hash`, so you can repair it, where `get_frontmatter` refuses with `invalid_frontmatter`. Tags written in the note's text as `#tag` are not included. Also lists the note's saved queries under `saved_queries`, by name, without evaluating them. |
| `get_note_outline` | `vault_id`, `slug` | List one exact note's headings with the size of each section, plus the note's `content_hash` and its sizes, without any note text. See [[#Reading part of a long note]]. |
| `get_note_section` | `vault_id`, `slug`, `headings` | Read whole sections of one exact note, picked by heading text or heading path. `headings` is a list of 1 to 10 strings. See [[#Reading part of a long note]]. |
| `get_note_links` | `vault_id`, `slug` | Outgoing links and backlinks for one exact note, wikilinks and Markdown links to `.md` files alike. |
| `resolve_wikilink` | `vault_id`, `target` | Resolve a wikilink target within one Vault. |
| `get_tree` | `scope` | Grouped explorer tree for one Vault or all enabled Vaults. Optional: `folder` (a Vault-relative folder to return as the root), `max_depth` (how far below it to descend, minimum 1), `include_notes` (default `true`). |
| `get_stats` | `scope` | Grouped statistics for one Vault or all enabled Vaults. Also reports `hatchdoor_version`, the running instance's version string. |
| `get_graph` | `scope` | Grouped link graph for one Vault or all enabled Vaults. |
| `get_frontmatter` | `vault_id`, `slug` | Read one exact note's frontmatter metadata — `tags`, `aliases`, and every remaining top-level key under `properties` — without returning the Markdown body. A note with no frontmatter block answers `has_frontmatter: false` with empty collections rather than an error. It also returns the note's `content_hash` — the same string `get_note` reports for that note at that instant, and covering the whole file, so a note with no frontmatter block still has one — which means the metadata can be written straight back without pulling the body over the wire first. |
| `recently_modified` | `scope` | Recently modified notes. Optional `limit` (1–25, default 5). |
| `query_notes` | `scope`, `conditions` | Select the notes whose tags, path, or frontmatter properties satisfy stated conditions. Optional: `properties` (names to return on each row), `limit` (1–200, default 50). See [[#Selecting notes by tag, path or property]]. |
| `find_text` | `scope`, `text` | Find every note that contains one literal string, with how many times each contains it. Optional: `case_sensitive` (default `false`), `layers`, `path_prefix`, `limit` (1–500, default 50), `snippets_per_note` (0–3, default 3). See [[#Finding every note that contains a string]]. |
| `evaluate_saved_query` | `vault_id`, `slug` | Evaluate one saved query in a note and return the notes it selects as rows. Optional `name`, required when the note holds more than one. See [[#Reading a note's saved queries]]. |
| `list_note_attachments` | `vault_id`, `slug` | List the attachments one note references, without the note's full content. Every non-Markdown file the note points at counts, not only the ones Hatchdoor can display. |
| `get_attachment` | `vault_id`, `relative_path` | Fetch one attachment's bytes, addressed by the same `relative_path` `list_note_attachments` reports. Fetchable types are narrower than the managed set — `png`, `jpg`, `jpeg`, `gif`, `webp`, `svg`, `avif`, `bmp`, `pdf` — so a video or data file is listed but refused here. Optional `encoding`: `url` (the default) returns a `download_url` transfer link that needs no token, `base64` returns the bytes inline. |
| `get_attachment_import_config` | `vault_id` | Report whether uploads are currently possible for this Vault, the available methods, their byte limits, and the allowed file extensions. Call this before uploading. |

Collection-scoped results (`search_notes`, `get_tree`, `get_stats`, `get_graph`, `recently_modified`, `query_notes`, `find_text` with `scope: "all"`) carry `scope`, `collection_revision`, `partial`, and `participants` — an agent should branch on the structured error `code`, never on message text, and should treat `partial: true` as "at least one Vault's part of this result is not current," not as an error. It is true whenever any entry in `participants` reads something other than `fresh`: `stale` (answered, but possibly behind that Vault's Markdown, usually a prior index generation while a turn catches up), `not_searchable` (semantic search only: some or all of the notes searched in that Vault have no embeddings, because they are not embedded yet or sit in a demoted layer with `HATCHDOOR_EMBED_LAYERS` off; any hits it returned still count, and Keyword search reaches every note), or `unavailable` (sat out, with the reason on the entry). A Vault can sit out for more than one reason — its snapshot was not readable, or, on `get_tree`, it simply does not have the folder that was asked for — so read the reason off that Vault's entry in `participants` rather than inferring it from `partial` alone. A Vault whose entry reads `stale` is a case an agent can act on rather than only report: its snapshot is known to be behind its Markdown, and `refresh_vault`, under [[#Vault collection: discovery and management]], asks for the index turn that republishes it. Usually that turn is already queued, so a second read a few seconds later is often enough. A write can take up to about five seconds to reach the index, so a read straight after one can still report `fresh` without it.

`collection_revision` is not a content version and cannot tell whether a result includes a write. It counts changes to the Vault collection's status: a Vault added, edited, enabled, disabled or disconnected, or one Vault's index, Git, watcher or local-file status moving. A note write does not advance it. The index turn that follows usually moves it by two, only because the Vault's index status passes through `stale` and back. Read `participants` to judge freshness.

`get_tree` with nothing but `scope` returns the whole Vault, which on a few hundred notes is large enough to overflow a client's per-result budget. Three optional arguments narrow it. `include_notes: false` is the cheap one to open with: it returns every folder at every level with its note count and no notes at all, so a several-hundred-note Vault's shape costs on the order of a kilobyte instead of seventy. `folder` returns one subtree — `"40-reference/Parenting"`, matched case-insensitively, surrounding slashes ignored. `max_depth` stops the descent: the starting folder is depth 0, a folder at the limit is listed with its count but not opened, and one that had something inside it is marked `truncated` so it cannot be mistaken for an empty leaf. Every folder reports `note_count`, the notes held directly inside it, not counting its subfolders; a subtree total is the sum of those. A `folder` naming something the Vault does not have answers the structured error `folder_not_found` rather than an empty tree, so a typo never reads as an empty folder. With `scope: "all"` that refusal is per-Vault: the Vaults that do have the folder still answer, each Vault that does not appears in `participants` carrying `folder_not_found`, and the result is marked `partial`. Only when no Vault has it does the whole call refuse, and that refusal names no Vault, because none of them is more at fault than the others; the per-Vault refusals stay on `participants`, where each one does name its own Vault.

Notes inside a tree carry `title` and `slug` but no `vault_id`: the tree they sit in already names its Vault, once. Flat results that mix Vaults in a single list — `search_notes`, `recently_modified`, `query_notes`, `find_text` — still qualify every hit.

### Compact and full search hits

`search_notes` returns small hits unless you ask for more, because most searches only locate a note that `get_note` then reads. Before 2.8.0 every hit carried the whole matched chunk, and a search near the defaults cost 20 to 50 KB.

A compact hit, the default, has eight fields: `vault_id`, `note_slug`, `note_title`, `note_path`, `heading_path`, `score`, `layer` and `snippet`. It has no `content`, `outbound_links`, `metadata` or `chunk_id`.

The `snippet` is at most 200 characters, and its length cannot be changed. Except for a `#tag` query, it is copied from the matched chunk:

- In keyword mode it is centred on the first query word the chunk contains.
- In semantic mode it is the start of the chunk.
- For a `#tag` query it is the line `Matched tag: #<tag>`, which names the tag and is not copied from the note. A tag match has no matched chunk to quote.
- `…` marks each side where text was left out. A chunk of 200 characters or fewer comes back whole, with no `…`.
- A cut falls between words. Text with no spaces near the cut, such as Chinese or Japanese or a very long URL, is cut between characters instead, never inside one.

Pass `detail: "full"` to get the earlier shape: every hit carries the whole chunk in `content`, the note's `outbound_links`, its `metadata` (tags and aliases) and the `chunk_id`, and no `snippet`. For a `#tag` query, `content` holds the line `Matched tag: #<tag>` instead of note text. Any other `detail` value is refused as invalid input.

Both shapes return the same hits, with the same scores, in the same order. `detail` changes only what each hit carries. A `search_notes` item inside `batch` follows the same default.

### Selecting notes by tag, path or property

`query_notes` answers the requests that a note's tags, folder or properties decide on their own. Every note tagged `project/active`. Everything under `40-reference`. Notes whose `review-date` has passed. It is not a search: a note either satisfies the conditions or it does not, the results are unranked and come back in the same order every time, and no embedding is involved, so a Vault that is still being indexed answers in full. Reach for `search_notes` when the question is about what a note *says*, and this when it is about what a note *is*.

`conditions` is a list, and every one of them must hold for a note to be selected. There is no whole-Vault query: at least one condition is required, and `get_tree` or `recently_modified` is the tool for reading a Vault without selecting. Three kinds:

- `{"type": "tag", "tag": "project/active"}` — the leading `#` is optional, case does not matter, and nested tags come along: `project` selects `project` and `project/active`, never `projects`.
- `{"type": "path_prefix", "prefix": "40-reference/Parenting"}` — a Vault-relative folder, matched case-insensitively and by whole path segment, so `notes` never selects `notes-archive`.
- `{"type": "property", "name": "review-date", "operator": "lt", "value": "2026-09-06"}` — one frontmatter property, tested by one operator.

The property name is matched exactly, capitalisation included. `tags` and `aliases` are refused as property names rather than quietly answering "missing" for every note: Hatchdoor parses both out of the frontmatter into their own lists, so select tags with a tag condition instead. Both can still be *returned* by naming them in `properties`.

One more difference from search: `query_notes` reads every [[The layer system|layer]] and takes no `layers` argument, so a demoted note it selects is one an ordinary search would not have returned. That matches the explorer and `get_tree` rather than `search_notes` — demotion moves a note off the default *search* surface, and a query is not a search.

Ten operators. `eq`, `ne`, `lt`, `lte`, `gt` and `gte` each need a `value`; `exists`, `missing`, `empty` and `not_empty` must not carry one. The rules worth knowing before you rely on them:

- Only `missing` selects a note that never mentions the property. `ne` does not: a note with no `status` does not have a status different from `draft`.
- `eq` against a multi-value property matches when any entry equals the value, so `for: [reference, archive]` is equal to `reference`.
- Ordered comparison works between two numbers, two strings, or two booleans, and selects nothing across types rather than guessing. Strings compare byte by byte, which orders ISO-8601 dates and timestamps correctly — that is what makes "before today" work, with today's date passed as the value.
- `empty` means present but holding nothing: bare `status:`, a blank string, an empty list, an empty mapping. A note that has no `status` line at all is `missing`, not `empty`.

`properties` names the frontmatter to return on each row. A name a note does not carry is left off that row rather than returned as null, so "no value" and "the value is null" stay apart. Rows are ordered by path, then Vault, then slug, and `limit` is applied to that order across the whole collection, so two identical calls return identical rows. When more notes qualified than `limit` allowed through, the result says `truncated: true` rather than leaving a full page to read as a complete answer.

A query Hatchdoor cannot answer — no conditions, a comparison with no value, an operator handed one it does not take — is refused with the structured error `invalid_query` naming what is wrong, before any Vault is touched.

### Finding every note that contains a string

`find_text` answers exact questions. Does anything still say the old name? Did a bulk edit land everywhere? Which notes cite this path or ID? It returns every note that contains the string you give it, and how many times. `search_notes` cannot answer these: both of its modes rank, keyword mode accepts a note that holds any one word of the query, and the number of hits it returns is not a count of occurrences.

`text` is one literal string. Every character means itself, so `[[a.b]]`, `*` and `(` are matched as written, and there are no wildcards, no patterns and no second string in the same call. An empty string is refused with the structured error `invalid_text_match`.

Case is ignored unless you pass `case_sensitive: true`. Accents always count, so `resume` does not match `résumé`. The two ways Unicode can store an accented letter, as one character or as a letter plus a combining accent, are always treated as the same text.

Hatchdoor looks in three places and counts each occurrence under its place:

- `body`: the Markdown below the frontmatter block.
- `frontmatter`: the frontmatter block as written, so aliases, tags and property values are covered.
- `path`: the note's file path inside its Vault, `.md` included.

Each matching note appears once, with its `vault_id`, `slug`, `title`, `relative_path` and `layer`, its `occurrences`, the split by place under `places`, and up to three `snippets`. A snippet is one matched line with its `place` and its `line` number in the file, cut to about 200 characters around the match when the line is longer. A line that holds the string twice gives one snippet and counts twice. Occurrences are counted without overlap, so `aaa` holds `aa` once. A string that runs from the frontmatter into the body counts under `frontmatter`, where it starts. A path match has no line number. `snippets_per_note: 0` returns the counts alone, which keeps a wide sweep small.

Nothing is ranked and there is no `score`. Notes are ordered by path, then Vault, then slug, so two identical calls return the same list. `limit` cuts that list, but `total_notes` and `total_occurrences` always count the whole scope, and `truncated: true` says the list was cut. There is no paging. To see more, raise `limit` up to 500 or narrow the sweep with `path_prefix`, which names a Vault-relative folder and is matched the way `query_notes` matches one.

Two things differ from `search_notes` on purpose:

- `find_text` covers every [[The layer system|layer]] unless `layers` narrows it. A sweep that skipped demoted notes would report a clean result while the string still sat in an archived one. Name layers to narrow, with `default` for the default surface. A name that no Vault in scope declares is refused with `invalid_layer_selection`, as it is in `search_notes`.
- `find_text` reads each note's Markdown file when the call runs, so it sees an edit at once, before the index has caught up. Only the list of notes comes from the index. A note created in the last few seconds can be missing from that list. Its Vault's entry in `participants` usually reads `stale` meanwhile, but a write can take about five seconds to mark it, so give a brand-new note a moment before you rely on a sweep that should include it.

A note whose file could not be read is listed under `unread`, with `reason` `missing` (the file is gone) or `unreadable`, and counted in `total_unread`. That note's text was not checked, so an empty `notes` list proves the string is gone only when `total_unread` is 0. `unread` lists at most 50 notes; `total_unread` counts them all. A file that is not valid UTF-8 is still read and matched.

Reading every file is slower than an index lookup. On a large Vault, narrow with `path_prefix` when you can.

### Reading part of a long note

`get_note` returns the whole file. For a long rules note or runbook where you want two sections, that is most of the cost for none of the benefit. `get_note_outline` and `get_note_section` read part of a note instead.

A **section** is a heading line and everything under it, up to the next heading of the same or a higher level. Its subsections are inside it. This is the same span `replace_section` replaces, so what you read is what a write to that heading would overwrite. A `#` line inside a fenced code block is not a heading.

A **heading path** is the headings above a heading and its own text, joined with ` > `, such as `Rules > Filing > Inbox`. A `search_notes` hit carries one as `heading_path`, built from the first three heading levels.

`get_note_outline` returns no note text. It returns:

- `vault_id`, `slug`, `relative_path` and `content_hash`, the same hash `get_note` reports.
- `size_bytes`, the size of the whole file.
- `frontmatter_bytes`, the size of the frontmatter block with its `---` lines, or `0`.
- `opening_text_bytes`, the size of the text between the frontmatter and the first heading.
- `headings`, in document order. Each has `text` (the heading without its `#` characters), `level` (1 to 6), `heading_path` and `size_bytes`, the size of its section.

A note with no headings returns an empty `headings` list, not an error. The frontmatter, the opening text and the sections of the headings that sit under no other heading add up to `size_bytes`.

`get_note_section` takes `headings`, a list of 1 to 10 strings, and reads the note once. Each string is a heading's exact text or a heading path. If exactly one heading has that text, it is selected. Otherwise the string is read as a heading path and must match exactly one. Matching is exact, case included, with only the spaces around your string ignored. Write the text without `#` characters: `Filing`, not `## Filing`.

The reply carries the note's `content_hash` and `sections`, one entry per string in the order you asked:

- A found entry has `requested` (your string), the `heading_path` it resolved to, `level`, and the text under `section`, byte for byte as the file holds it.
- An entry that selected nothing has `requested` and `error`, with `code` `heading_not_found` or `heading_ambiguous`. An ambiguous one lists every matching heading path under `matches`, so you know what to send next. Two headings with the same full path cannot be told apart by any request; read the enclosing section, or the whole note, for those.

One bad heading never fails the call: the other entries still come back. Only a malformed list does. An empty list, an empty string or more than 10 strings is refused with the structured error `invalid_heading_selection`. A note that does not exist is `note_not_found`, as with `get_note`.

The frontmatter and the opening text are not sections and cannot be requested. Use `get_frontmatter` for the properties, or `get_note` for everything.

The text comes back in a field named `section`, never `content`. It is part of a note. Do not pass it to `update_note`, which replaces the whole note. To change a section, pass the reply's `content_hash` to `replace_section` or `edit_note`. `replace_section` names its heading with the `#` characters, which the outline's `level` gives you: a level 2 heading with the text `Filing` is `## Filing`.

Both tools read the note's file when the call runs, as `get_note` does.

### Reading a note's saved queries

A note can hold saved queries, fenced `base` blocks that describe which notes to list (see [[Supported Markdown reference]]). The note page draws each one as a table. An agent gets the same rows as data in two steps.

`get_note` returns the note exactly as its file holds it, `base` blocks and name markers included, and never anything computed. Beside the note it returns `saved_queries`, one entry per block in the order they appear, each with the `name` its `<!-- hatchdoor-query: name -->` marker gives it, or `null` when it has none. That is how an agent learns what it can ask for without reading the definitions.

`evaluate_saved_query` then evaluates one of them and returns the notes it selects. Pass the note's `vault_id` and `slug`, plus `name`. You can leave `name` out only when the note holds exactly one saved query. The agent never reads or rewrites the definition, so it cannot drop one of its conditions along the way. That is why this tool exists instead of leaving agents to rebuild the definition as a `query_notes` call.

A saved query always reads the Vault its note lives in. The tool takes no `scope`, and a call that passes one is refused as an invalid argument.

The answer sits inside the usual `scope`, `partial` and `participants` envelope, because the rows come from the Vault's index. Its `data` has a `status` of `populated` or `empty`. `populated` always carries at least one row. `empty` means every note was checked and none qualified, which is a real answer. `columns` lists the definition's columns, and each row carries the matched note's `vault_id`, `title`, `slug` and `relative_path`, plus `cells` with one value per column in the same order. A property the note does not have is `null`. When more notes qualified than the rows hold, `truncated` says why: `definition_limit` when the view's own `limit` held them back, `ceiling` when Hatchdoor's cap of 500 rows did. `ignored` names presentation instructions such as `groupBy` that were not carried out. The rows are complete without them.

If the call cannot produce an answer, it fails with a structured error. It never returns an empty table instead:

| `code` | Meaning |
| --- | --- |
| `no_saved_queries` | The note holds no saved query. |
| `saved_query_name_required` | `name` was left out and the note holds several. The message lists the names. |
| `saved_query_not_found` | No saved query in the note has that name. An unnamed saved query cannot be reached by any name. |
| `saved_query_name_ambiguous` | Two or more saved queries in the note share the name, so it picks neither. Rename one. |
| `saved_query_refused` | The definition uses something Hatchdoor cannot evaluate. The message names it, as the note page does. |
| `saved_query_stopped` | Evaluating it would pass one of Hatchdoor's limits: 20,000 notes scanned, or more than 10 saved queries in the note. |

The tool evaluates only the saved query you name, so it gets the whole 20,000-note scan budget to itself. The note page evaluates all of a note's saved queries together and splits that budget between them. On a large Vault, where a note's saved queries cannot all scan it within 20,000 notes between them, a later one can show **Stopped.** on the page and still return rows here. With two saved queries that happens past 10,000 notes, with three past about 6,700.

A saved query is never picked by its position in the note. Moving blocks around changes nothing an agent gets back for a given name.

> [!note]
> `get_attachment_import_config`'s `enabled` field is the AND of two independent gates: `HATCHDOOR_MCP_WRITE_ENABLED` (instance-wide) and the target Vault's own `capabilities.mutate` (source mode and lifecycle phase). The response explains which one is currently false when `enabled` is `false`.

`get_attachment` mirrors the inbound upload flow in the opposite direction. `encoding: "url"` returns `content.download_url`, a **transfer link**: a full address, built on **Public address** (`HATCHDOOR_PUBLIC_URL`) when that is set and otherwise on the address the client reached the server on, as a proxy reports it in `Forwarded` or `X-Forwarded-Proto`/`X-Forwarded-Host`, that carries its own credential for this one file. Fetch it as it is, with no `Authorization` header, for example `curl -o manual.pdf '<download_url>'`. It answers any number of times until `content.expires_at` (Unix seconds), five minutes after it was issued; `path_note` and `auth` restate this. An agent inside an MCP client holds neither the MCP password nor the server's address, and this is the route that needs neither. A client that cannot make an out-of-band HTTP request calls `get_attachment` again with `encoding: "base64"` and gets `content.content` inline instead, bounded by the same `HATCHDOOR_MCP_MAX_BASE64_BYTES` cap `import_attachment` uses on the way in. A file over it is refused with the structured error `attachment_too_large_for_base64`, whose message gives the measured size; call again with `encoding: "url"`. Either way the result carries `vault_id`, `relative_path`, `size_bytes`, and `content_type`.

> [!note]
> A download link is held to the same limits as fetching the bytes over `/mcp`: the same `HATCHDOOR_MCP_MAX_BASE64_BYTES` ceiling (a larger attachment returns `413` with `asset_too_large`; raise the setting) and the same rate quota, drawn from the same counter, answering `429` with `Retry-After`. The link is a cheaper transport, not a larger allowance. It is refused with `403` for any other file or Vault (`transfer_link_invalid`), after it expires (`transfer_link_expired`), after a restart or an MCP password change (`transfer_link_invalid`), and while MCP is off (`mcp_disabled`). Ask again for a fresh one. See [[The security model]].

## Write content tools

Every tool below requires `HATCHDOOR_MCP_WRITE_ENABLED=true` and takes `vault_id` in addition to the parameters listed. Every mutating tool that targets an existing note also requires `expected_content_hash`, the hash most recently read from `get_note`, or from `get_frontmatter`, `get_note_outline` or `get_note_section` when the whole body is not needed. This is optimistic concurrency: a stale hash means someone else changed the note since you read it, and the write is rejected rather than silently overwriting.

One limit on that promise is worth knowing. Hatchdoor normally commits a save by swapping the new copy of the note with the old one in a single step, which is what makes a change landing mid-save detectable. Filesystems that cannot do that swap, ZFS before 2.2 and anything mounted through FUSE, get a save that checks the note and then replaces it as two steps, and a change landing between those two is overwritten rather than reported. A stale hash is still refused either way, and Hatchdoor's own writes are serialised whatever the filesystem. This is not visible over MCP: `list_vaults` capabilities do not carry it, and the answer lives on the HTTP route `GET /api/v1/vaults/{vault_id}/write-capabilities` and in one line per Vault in the server log.

A write is refused before it touches the file, so a rejected write has changed nothing and can be retried against a fresh hash. The exception is `write_recovery_required`, which means the opposite: the new content was saved and then could not be checked or undone, because something outside Hatchdoor changed the Vault directory mid-write. Do not retry it. The message names the note and the leftover file holding the previous content, and a person has to decide what the note should say.

| Tool | Required parameters (beyond `vault_id`) | Purpose |
| --- | --- | --- |
| `create_note` | `relative_path`, `content` | Create a Markdown note. Parent folders are created automatically. Fails if the note exists unless `overwrite: true`. |
| `update_note` | `slug`, `content`, `expected_content_hash` | Replace a note's full content. |
| `append_to_note` | `slug`, `content`, `expected_content_hash` | Append content to a note. |
| `edit_note` | `slug`, `old_string`, `new_string`, `expected_content_hash` | Surgical string replacement. `old_string` must match exactly and be unique unless `replace_all: true`; otherwise the edit is rejected without writing. Prefer this over `update_note` for small changes. |
| `replace_section` | `slug`, `heading`, `mode`, `content`, `expected_content_hash` | Replace or insert around a Markdown section identified by its heading. `mode` is `replace` (overwrite the section — `content` should include the heading), `before`, or `after`. The section spans the heading through the next same-or-higher heading; headings inside fenced code blocks are ignored, and the heading must match exactly and be unique. |
| `update_frontmatter` | `slug`, `frontmatter`, `expected_content_hash` | Shallow top-level merge into the note's YAML frontmatter, leaving the body untouched. Only the keys you name change: the rest of the block keeps the formatting you gave it, so your key order, one-line lists, indentation, quoting and comments all survive a write. An explicit `null` deletes a key; a nested mapping is replaced wholesale rather than merged into. A list you replace keeps the shape it had, and a key that did not exist is added at the end of the block with any list on one line. The whole call is refused, writing nothing, when a key you named cannot be changed without guessing, which in practice means a note whose block writes that key twice. A note with no frontmatter block gets one created, and deleting its last key removes the block rather than leaving an empty one. Rejects an empty `frontmatter` object, and a creation whose values are all `null`. |
| `rename_note` | `slug`, `new_title`, `expected_content_hash` | Rename within the current folder; rewrites backlinks in both wikilink and Markdown link form. The note keeps its folder, so the assets kept inside that folder stay exactly where they are and `moved_assets` comes back `0`. |
| `move_note` | `slug`, `target_folder`, `expected_content_hash` | Move to a target folder; same backlink handling as rename, and carries along the assets kept inside the note's own folder. |
| `move_rename_note` | `slug`, `target_relative_path`, `expected_content_hash` | Move and rename in one operation. |
| `archive_note` | `slug`, `expected_content_hash` | Move to the configured archive folder (the Vault's own `archive_folder`, set via `create_vault`/`edit_vault` above, or the instance default). |
| `delete_note` | `slug`, `expected_content_hash` | Trash a note under `.hatchdoor-trash`; removes backlinks to it (a Markdown link keeps its text as plain words) and trashes the assets kept inside its own folder. |
| `rename_tag` | `old_tag`, `new_tag` | Rename a tag, and every tag nested under it, across the whole Vault. Called without `expected_plan_hash` it only plans; called with the `plan_hash` that plan returned, it applies it. See [[#Renaming a tag across a Vault]]. |
| `delete_tag` | `tag` | Remove one tag from every frontmatter `tags` list in the Vault. Planned and applied in two calls like `rename_tag`. Refused while any note carries a tag nested under it or carries it inline in its body. See [[#Deleting a tag across a Vault]]. |
| `import_attachment` | `content` (base64), `target_relative_path` | Upload an attachment by sending its bytes base64-encoded. This is the **fallback** for clients that cannot make an out-of-band HTTP request — size-limited (`HATCHDOOR_MCP_MAX_BASE64_BYTES`, default 5 MiB decoded). Prefer `create_upload_link` whenever the client can make an HTTP request; call `get_attachment_import_config` first to see current limits. |
| `create_upload_link` | `target_relative_path` | Mint an upload transfer link for one file, the recommended upload route for an attachment and the only way to import an existing Markdown file as a note: a target ending in `.md` is a note upload. Optional `overwrite` (default `false`) allows replacing an existing file; replacing a note also needs `expected_content_hash`. Returns `upload_url`, `method` (`POST`), `upload_kind` (`note` or `attachment`), `expected_content_hash` (echoed, or `null`), `expires_at` (Unix seconds), and `max_bytes` (`HATCHDOOR_MAX_ATTACHMENT_BYTES`). Send the file to `upload_url` as `multipart/form-data` in a field named `file`, with no token: `curl -F file=@scan.pdf '<upload_url>'`. A `target_relative_path` form field is optional and must match. See [[#Upload links]]. |
| `move_attachment` | `source_relative_path`, `target_relative_path` | Move an attachment and rewrite every note reference to it. |
| `rename_attachment` | `source_relative_path`, `new_filename` | Rename an attachment in place and rewrite every note reference to it. |
| `delete_attachment` | `source_relative_path` | Trash an attachment under `.hatchdoor-trash` and rewrite every note reference to it. |

`move_attachment`, `rename_attachment` and `delete_attachment` act on bytes the Vault already stores, so the upload allowlist does not gate them: any file that is not Markdown qualifies, whatever its extension, including files with no extension at all. Four things are refused. A `.md` target, because moving a note this way would skip backlink rewriting, slug handling and the hash check — use the note tools. A `.hatchdoor-layer` marker, because trashing one would silently promote a whole folder back onto the default surface. Anything under `.git`, which is the Vault's repository rather than its content. And any path the Vault excludes as noise, on the source side as well as the destination, which is what keeps these tools out of `.obsidian/`. Reference rewriting still keys on the extension, so a link to an extensionless file is left as written when that file moves. See [[How to import and work with attachments]] for the three lists — uploadable, managed, fetchable — and how they differ.

A backlink to a note that `rename_note`, `move_note`, `move_rename_note` or `archive_note` retargets keeps the form it was written in. A link written without a folder path, `[[Some Note]]`, stays bare and picks up the note's new title, spelled as the note's filename; that includes a link that only reached the note through its slug, such as `[[some-note]]` or a title typed with an em dash where the filename has a hyphen. A link written as a full path picks up the new full path. The one exception is a new title that another note already carries, where the link falls back to the full path so it keeps pointing at the note that moved. A move that does not change the note's title therefore leaves bare-title links alone, though a slug-form or punctuation-drifted link still comes back spelled as the title. The note being renamed or moved is one more note holding links to it, so a link in its own body that points at itself follows the same rules: after a rename a bare-title self-link picks up the new title, and after a move a path-qualified self-link picks up the new folder while a bare one keeps its bare form. `rewritten_notes` counts only the other notes that changed, so a rename whose one stale link was the note's own comes back as `0`; the note's own new text is covered by the `content_hash` in the same response. `delete_note` is the exception, and leaves the trashed copy's link to itself as written. A link inside a table cell writes its alias pipe escaped, as `[[Some Note\|alias]]`, because a bare `|` would end the cell. That escape is part of the form as well: the link is retargeted and the backslash is handed back, so the cell stays a valid table row.

A Markdown link to a note, `[text](../folder/Some%20Note.md)`, is a backlink too and is retargeted in the same write as any wikilink in that file. It keeps its form: a relative path is worked out again from the linking note, a path starting with `/` keeps it, a path written from the Vault root stays so, and a bare filename stays bare while that name still reaches the note, otherwise the relative path is written. Only the path changes, never the link text, heading anchor or title, and a reference-style link changes on its definition line. The moved note's own Markdown links are repointed from its new folder so they reach the same notes. `delete_note` removes each Markdown link to the deleted note and leaves its text as plain words. [[Supported Markdown reference]] has the full rules.

An asset travels with a note only when it already lives inside that note's own folder, or a subfolder of it. An asset the note merely points at from somewhere else, such as a shared `_system/` or `Attachments/` folder sitting beside the note's folder, stays exactly where it is: `rename_note`, `move_note`, `move_rename_note`, `archive_note` and `delete_note` leave it alone and rewrite the moved note's own link so it still resolves from the note's new home. Other notes pointing at it are left untouched too, since nothing about it changed. `moved_assets` in the response counts only the assets that actually moved.

A note Hatchdoor cannot read as text does not block these tools. One it cannot open at all, such as a `.md` symlink whose target is gone, is skipped, the way the search index skips it. One that is not valid UTF-8, a Latin-1 export from an older tool for example, is checked for links all the same, and left byte for byte as it was when it has none to what is moving. When it does link to the note being renamed, moved, archived or deleted, or to an attachment being moved, renamed or deleted, Hatchdoor cannot fix that link without rewriting the bytes that are not text. The whole call is then refused with `link_rewrite_unsupported`, nothing is written, and the message names every such note at once so they can all be fixed, by re-saving them as UTF-8, before trying again.

A note rewritten only to keep its links pointing at the right place keeps the modification time it had. That covers every note `rewritten_notes` counts, for `rename_note`, `move_note`, `move_rename_note`, `archive_note`, `delete_note` and the attachment tools, and it holds when a failed call puts those notes back. So `recently_modified` and the Stats page do not show a batch of renames as a wave of edits. The note you rename or move keeps its own modification time too. A note you edit directly with a write tool takes the current time as always. If the filesystem refuses the old time, the write still succeeds and the note carries the time of the write.

> [!note]
> A note sitting in the Vault root has the whole Vault as its own folder, so every asset it references counts as living inside it and does travel with the note when it moves to another folder. A rename keeps the note where it is, so nothing travels. Keep notes that share an attachments folder in a folder of their own if you want that folder left alone.

Every write tool accepts an optional `commit_summary`, a one-line string describing what the change was for. On a Vault with versioning enabled it reaches the body of the commit that records that write. One commit usually covers several writes, since Hatchdoor commits on a schedule rather than per call: the subject names the first few operations and how many files they touched, and the body lists one `- ` line per summary, in the order the writes happened. A write with no summary still shapes the subject. Pass one on every write and the Vault's history reads as a log of why each note changed.

> [!warning]
> No write tool can create, rename, or move a file named `.hatchdoor-layer` (the layer marker) — that call is rejected outright, since a marker silently changes how a whole folder is classified and is meant to be edited directly in the Vault. Writes are also rejected if the target path matches the Vault's own noise-exclusion patterns, since such a file would be written to disk but stay invisible to every read surface.

### Upload links

`create_upload_link` checks the target before it answers and refuses at once, with the same codes `import_attachment` would use, when the path is invalid, the extension is not uploadable, or a file is already there and `overwrite` is `false`. So an agent never sends a file to a link that was doomed. The link is good for one upload to exactly that path under exactly that `overwrite` rule. It is spent by its first use, whether that upload succeeds or not, and it expires five minutes after it was minted. The existence check runs again when the file arrives, so a file that appeared in between is refused with `409 write_conflict` rather than overwritten. The upload answers `403` for any other target (`transfer_link_invalid`), a second use (`transfer_link_spent`), expiry (`transfer_link_expired`), a restart or an MCP password change (`transfer_link_invalid`), MCP being off (`mcp_disabled`), and MCP writes being off (`mcp_write_disabled`). A success returns the same shape as `import_attachment`.

A target ending in `.md`, in any case, is a **note upload**. The file becomes a note through the same write `create_note` performs, so its content never passes through the conversation and a long file cannot be cut short on the way in. It gets the same treatment as `create_note`: it must be UTF-8 with no NUL bytes (otherwise `400 invalid_write_input` and nothing is written), CRLF and CR line endings become LF, a missing final newline is added, and `quality_warnings` names each change. The content type the upload declares is ignored. A success returns what `create_note` returns: `slug`, `relative_path`, `content_hash`, `layer` and `quality_warnings`. A path with no extension is not a note upload, even though `create_note` would add `.md` to it. An upper-case extension is written in lower case, so `Report.MD` becomes `Report.md`.

Without `overwrite`, a note upload creates the note and is refused at minting, and again at upload with `409 write_conflict`, when a note is already there. Replacing a note takes `overwrite: true` and `expected_content_hash`, the note's current hash from `get_frontmatter` (which returns it without the body) or `get_note`. Minting refuses a replacing link with no hash (invalid params), a stale hash (`write_conflict`), or a note that does not exist (`note_not_found`). The hash is signed into the link, so it cannot be swapped. The upload writes only if the note still has that hash, and otherwise fails with the same `409 write_conflict` `update_note` gives, leaving the newer edit in place. The answer's `content_hash` is what the next replace needs. `expected_content_hash` anywhere else, on an attachment target or a link that does not replace, is refused as invalid params.

### Renaming a tag across a Vault

`rename_tag` changes a tag everywhere it appears: in frontmatter `tags` lists, and in namespaced `#hashtags` in note bodies. It always takes two calls.

1. Call it with `old_tag` and `new_tag`. Nothing is written. The answer is the plan: `notes` lists every note that would change, with its `slug`, `relative_path`, and whether the change is in its `frontmatter`, its `body`, or both; `frontmatter_notes` and `body_notes` count each; and `plan_hash` is the plan's fingerprint.
2. Call it again with the same tags and `expected_plan_hash` set to that `plan_hash`. Hatchdoor plans again and applies the plan only if it comes out identical. If any note changed in between, the call is refused with `tag_rename_plan_stale` and nothing is written; plan again and look at the new plan.

The fingerprint is a hash of the edits themselves, not something the server remembers, so a plan never expires on its own. It stops working the moment the Vault stops producing it.

Renaming a tag renames everything nested under it: `domain` renames `domain/homelab` to `topic/homelab`, even when no note carries `domain` itself. Matching ignores case, as tag search does, and `new_tag` has to be lowercase letters, digits, `-`, `_` and `/`, with no empty segment; a leading `#` is fine on either. A rename that would not settle is refused, because running it again would rename it again: a `new_tag` nested under `old_tag`, like `domain` to `domain/old`, and a rename into an ancestor that turns some note's tag into one still under `old_tag`, like `domain/x` to `domain` when a note carries `domain/x/x`. Any refusal of the names comes back as `invalid_tag_name`.

Renaming into a tag that already exists is a merge, and is allowed. A note that would end up carrying the target twice keeps it once, in the earlier of the two places. `already_tagged_notes` counts the notes that carried `new_tag`, or a tag under it, before the rename; above zero means this is a merge.

Only the renamed characters change. A one-line `tags: [a, b]` list stays on one line, a list written one item per line stays that way, and key order, quoting and every other byte of the note stay as they were. A hashtag inside a fenced code block or an inline code span is not a tag, so it is left alone, the same rule the index follows.

It is all or nothing. When a note carries the tag in a way that cannot be changed without restyling or guessing, the plan is refused with `tag_shape_unsupported`, the message names every such note and why, and no note is written. The cases are a frontmatter list the editor would reformat (quoted items, extra spaces inside the brackets, a comment on the line), frontmatter that is not valid YAML, a body hashtag interrupted by a code span, and a body hashtag that would lose its `/` and so stop being a tag. Fix those notes in the Vault and plan again. If a write fails partway through applying, every note already rewritten is put back. Each note the rename rewrites keeps its modification time, so retagging a Vault does not show up as a wave of edits. Hatchdoor holds the Vault's write lock for the whole call, so a Vault with versioning enabled records the rename as a single write in its next commit. The lock does not bind a person editing the same Vault in Obsidian, so each note is written only if it still matches what the plan read: a note saved by hand while the rename is running stops the call at that note, everything already rewritten is put back, and the hand-written save is left exactly as it was. On a Vault whose filesystem cannot swap two files in one step, see the note under **Write content tools**, a hand save landing inside the moment between that check and the write is overwritten instead of stopping the call.

A tag no note carries is not an error: the plan lists no notes and has no `plan_hash`, since there is nothing to confirm. Run the same rename twice and the second plan is empty. Deleting a tag is a separate tool, `delete_tag`, so a rename call with a missing argument can never delete anything.

### Deleting a tag across a Vault

`delete_tag` removes one tag from the frontmatter `tags` list of every note that carries it. What it promises is that afterwards a tag search for that tag finds no notes. It takes the same two calls as a rename.

1. Call it with `tag`. Nothing is written. The answer is the plan: `notes` lists every note that would change, with its `slug`, `relative_path` and `content_hash`, and `plan_hash` is the plan's fingerprint.
2. Call it again with the same `tag` and `expected_plan_hash` set to that `plan_hash`. If any note changed in between, the call is refused with `tag_delete_plan_stale` and nothing is written.

It deletes the exact tag and nothing else. Matching ignores case and a leading `#` is fine, so `#Draft` and `draft` are the same delete. A name that is not a tag comes back as `invalid_tag_name`.

Two situations refuse the whole delete, writing nothing, because in both a tag search would still find notes afterwards:

- **A tag is nested under it.** Tag search is hierarchical, so a search for `domain` also finds notes tagged `domain/work`. Deleting `domain` while any note carries `domain/work`, in frontmatter or in its body, is refused with `tag_has_nested_tags`. The message lists each nested tag and how many notes carry it. Delete or rename those first, deepest first, then delete `domain`. There is no cascading delete.
- **A note carries it inline.** `delete_tag` never edits note bodies. If any note has the tag as a hashtag in its text, the delete is refused with `tag_used_inline` and the message names each such note, including notes that carry it in frontmatter too. Edit the text first, then plan again. Only namespaced hashtags such as `#area/work` are tags in a body, and a hashtag inside code is not a tag, so neither of those blocks a delete.

When a note loses its last tag, it keeps an empty list, `tags: []`. The `tags` key stays and the frontmatter block is never removed. That differs from `update_frontmatter`, which removes the block when you delete its last key. A tag written as a single value, `tags: draft`, is treated as a one-item list and also becomes `tags: []`. Only the removed item changes: the rest of the list keeps its shape, and every other key and the body stay byte for byte as they were.

The rest works as it does for a rename. A note whose tags cannot be edited that way is refused with `tag_shape_unsupported`, naming each note. A failed write puts back every note already rewritten. A tag no note carries plans no notes and no `plan_hash`. It works on every Vault, and on a Vault with versioning enabled the delete lands as one write whose commit names the tag, as `delete tag "#draft"`, so the Vault's Git history is where to look if a tag needs to come back.

### Response shape

A successful note write returns `vault_id`, `slug`, `relative_path`, `content_hash` (use this for the next write), `layer`, `quality_warnings`, `rewritten_notes` (other notes whose backlinks were updated), `moved_assets`, and `trashed_path` (set only by `delete_note`). `create_note` and `update_note` normalise the whole note as they write it: CRLF and CR line endings become LF, and a non-empty note that does not end in a newline gets one. `quality_warnings` names whichever of the two was applied, and `content_hash` is the hash of the file as written, so it will not match a hash of the content you sent when either change happened. `append_to_note`, `edit_note` and `replace_section` change only the part they name and leave every other byte of the note as it was, line endings and a missing final newline included. Line breaks in the text you send are written in the note's own line ending, CRLF or LF, whichever the note mostly uses. `append_to_note` appends your text as you sent it, leading spaces and blank lines included. It adds a line break before your text when the note's last line has none, and another after it when your text does not end in one. `replace_section` adds a line break wherever your content would otherwise run into the next line. `quality_warnings` names each converted line ending and added line break, and `content_hash` is again the hash of the file as written. `update_frontmatter` normalises nothing and leaves the note's line endings as they were. A successful attachment write returns `vault_id`, `attachment`, `rewritten_notes`, `trashed_path`, and `cleanup_warning`. `rename_tag` returns `vault_id`, `applied` (false for a plan), `old_tag` and `new_tag` as it read them, `notes_affected`, `frontmatter_notes`, `body_notes`, `already_tagged_notes`, `plan_hash`, and `notes`, where each note's `content_hash` is its hash after the call: unchanged for a plan, the new one once applied. `delete_tag` returns `vault_id`, `applied`, `tag` as it read it, `notes_affected`, `plan_hash`, and `notes`, each with `slug`, `relative_path` and `content_hash` in the same sense.

A write conflict (stale `expected_content_hash`, or a registry revision that moved under a Vault-management call) is reported as a retryable tool error — re-read the current state and retry rather than assuming the operation is unsafe to repeat.

Any read or write refused this way says so twice: `isError` is true on the result itself, and the structured result carries `ok: false` beside the `code`, `message`, `retryable` and, where the failure names a Vault, `vault_id`. Branch on whichever of the two you already read. The second signal matters if you read the structured result as the tool's typed answer, since the schema each tool advertises describes only the success payload and a refusal arrives in a different shape. The rule is that an `ok` field present and false means the call did not happen: a successful write still returns `ok: true`, and a successful read carries no `ok` field at all. The exception is the handful of setup-state refusals, returned before setup finishes or once a search model is chosen, which carry a plain sentence and no structured result to read.

When the manual explains the refusal, the structured result also carries `docs`, naming the page to read: `page` is a name `read_docs` accepts and `heading` is the section on that page, as in `{"page": "guides/how-to-troubleshoot-common-problems", "heading": "git-sync-is-failing"}`. It comes with the codes for a Vault that is unavailable or disabled (`vault_unavailable`, `vault_read_unavailable`, `vault_disabled`), a folder Hatchdoor cannot read (`vault_path_unavailable`, `vault_path_unreadable`), a Vault that cannot do what was asked right now, such as refreshing one that cannot be browsed (`capability_unavailable`), a failed write (`write_failed`, `write_recovery_required`), a Git sync failure (`managed_git_conflict`, `managed_git_authentication_failed`, `managed_git_remote_unreachable`, `managed_git_install_failed`, `managed_git_push_rejected`, `managed_git_dirty_working_copy`, `managed_git_operation_in_progress`, `managed_git_pull_only_local_commits`, `existing_git_local_history_manual_recovery_required`), and a registry that needs recovery (`vault_registry_recovery_required`). Any other error has no `docs`, and neither has an item error inside `batch`. The writes-off refusal (`-32602`) and the setup-state refusals stay as they are.

## Batch

`batch` runs an ordered list of the tools above in a single call. There is one such tool, not a batching variant per tool: each item names an `op` and carries that tool's own `arguments` exactly as a standalone call would, `vault_id` included, so one batch can span several Vaults.

`operations` is the only thing `batch` itself takes. Unlike every other tool it has no top-level `vault_id` and no batch-level `commit_summary`; each goes inside the `arguments` of the items whose tool takes it. A call that sends a field `batch` does not know is refused before any item runs, and the refusal names every unknown field at once, at both levels, together with the shape it expects.

```json
{
  "operations": [
    {"op": "create_note", "arguments": {"vault_id": "<id>", "relative_path": "Inbox/Draft.md", "content": "# Draft\n"}},
    {"op": "update_frontmatter", "arguments": {"vault_id": "<id>", "slug": "inbox/draft", "frontmatter": {"tags": ["status/draft"]}, "expected_content_hash": "ignored-here"}},
    {"op": "get_note", "arguments": {"vault_id": "<id>", "slug": "inbox/draft"}}
  ]
}
```

**What may go in.** Every read tool except `list_vaults`, `read_docs` and `search_docs`, and every note and attachment write tool — `create_note` through `delete_attachment`, deletes included. `rename_tag` and `delete_tag` are the exceptions: each touches every note carrying a tag and promises all or nothing, which a best-effort batch cannot keep, so call them on their own. Vault-management tools (`create_vault`, `edit_vault`, `enable_vault`, `disable_vault`, `disconnect_vault`, `sync_vault`, `retry_vault`, `publish_recovery_branch`, `refresh_vault`) and the model-setup tools are not batchable, and neither is `batch` itself. An unknown or disallowed `op`, an empty `operations` array, more than **50** read-shaped items, or more than **20** write-shaped items rejects the whole call up front, before any item executes.

**Best-effort, in order, no rollback.** Items run one after another; an item that fails never stops the ones after it, and nothing already written is undone. There is no mid-batch visibility either — an item sees the Vault, not the batch's own bookkeeping, apart from the hash chaining below.

**Permission is still per item.** `batch` itself is not gated on write mode, because a batch may be entirely read-only. Each write-shaped item is gated exactly as the same call would be standalone: with `HATCHDOOR_MCP_WRITE_ENABLED=false` the read items succeed and the write items fail individually with the code `mcp_writes_disabled`, and a Vault whose `capabilities.mutate` is false refuses its writes the same way.

**`expected_content_hash` inside a batch.** Once a batch has written a note, later items in that same call targeting the same `vault_id` + `slug` have their `expected_content_hash` replaced with the hash that write produced — you cannot know the intermediate hash without the round trip a batch exists to avoid, so supply any placeholder for it and it is discarded. This applies to `create_note` too: create a note and edit it later in the same call. A note the batch has *not* already written validates its `expected_content_hash` normally, exactly like a standalone call. The relaxation never leaks outside the call — Hatchdoor takes the write lock on every Vault the batch writes to before the first item runs and holds them all until the call ends, so no other Hatchdoor writer can slip in behind a substituted hash. That lock binds Hatchdoor, not Obsidian: what stops an outside editor is the per-save swap described under **Write content tools**, and on a filesystem that cannot do it the same narrow race applies here.

**Result shape.** `items` (one entry per requested operation, carrying `index`, `op`, `ok`, and then either `result` — that tool's own normal result — or `error`), plus `succeeded` and `failed` counts. A batch that ran at all returns success at the tool level; read `failed` and the per-item `ok` flags, never the call's own status, to find out what happened. An item's `error` is the same `{code, message, retryable, vault_id?}` object a standalone refusal carries, and its `code` is always a string to branch on. A refusal that a standalone call reports as a JSON-RPC error, such as malformed arguments or a missing `vault_id`, becomes `invalid_arguments` inside a batch, with the JSON-RPC number kept as `jsonrpc_code`.

**Cost to everyone else.** A batch holds the write lock on every Vault it writes to, from before its first item until the call ends. Other writers to those Vaults — the Web UI, another agent, the HTTP API — wait for it. That is what makes the hash chaining safe and what keeps the batch's writes in one Git commit, but it means a 20-write batch against a busy Vault is a pause other writers feel. Reads are unaffected. Taking the locks up front, always in the same order, is also what stops two batches that name the same Vaults in opposite orders blocking each other forever. If one of those Vaults is busy — a long Git sync, say — the whole batch waits for it before running anything, read items included.

Changing a Vault's settings while a batch is writing to it does not interrupt the batch: the lock survives the change, and the remaining items keep it. A Vault that goes away entirely part-way through — disabled or disconnected — is a different matter: its remaining items are refused rather than written without the lock, their errors say so, and the items already written stand, as with any other partial batch.

Vault changes made by a batch are committed together on the Vault's next Git sync turn, the same as any other burst of writes. (On the legacy single-Vault sync path, a batch's writes may land in more than one commit; nothing is lost or reordered, only the commit boundary differs.)

A batch is a single `tools/call`, so it costs one call against the per-token rate limit (`HATCHDOOR_MCP_RATE_LIMITS_ENABLED` in [[Settings and environment variables reference]]) however many reads and writes it carries, which is most of the reason to reach for it. Searches are the exception, because they are the expensive part: each `search_notes` item costs one call, as it would standalone, and a batch holding any search waits for one of the two expensive-search slots like a standalone search does. A batch of 50 searches is therefore no cheaper than 50 searches. The item caps above are the tool's own, enforced separately.

---

Related: [[Connect your agent]] · [[How to deploy Hatchdoor with an agent]]
