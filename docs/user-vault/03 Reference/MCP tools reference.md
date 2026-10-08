---
tags: [type/reference, topic/mcp]
---

# MCP tools reference

Every tool Hatchdoor's MCP endpoint (`/mcp`) offers. To make the first read-only connection, start with [[Connect your agent]].

## Permission model

Two switches decide what a call can do:

- **`HATCHDOOR_MCP_ENABLED`** turns MCP on. It is off by default, and while it is off the instance answers no MCP call.
- **`HATCHDOOR_MCP_WRITE_ENABLED`** turns the write tools on. It is off by default, even when MCP is on. A write tool called while it is off returns the JSON-RPC error `MCP write tools are disabled by HATCHDOOR_MCP_WRITE_ENABLED`.

Each Vault adds a third check, its own `capabilities.mutate`, which follows from the Vault's source type and state. A `pull_only` Git Vault, or a Vault that is not `ready` yet, refuses writes even with `HATCHDOOR_MCP_WRITE_ENABLED=true`. Read a Vault's capabilities from `list_vaults` before writing to it.

> [!note]
> Hatchdoor always lists every tool, even before model setup finishes and with no Vaults, so a client that caches the tool list when it connects never needs to reconnect. Before setup finishes, only these run: `get_model_setup_status`, `accept_gemma_terms`, `decline_gemma_terms`, the manual tools `read_docs` and `search_docs`, and the Vault collection tools such as `list_vaults`. Every other tool returns "Hatchdoor is still being set up." until a model is selected. `refresh_vault` is the one Vault collection tool that also waits: the index turn it asks for cannot run without a search model.

There is no selected, sole or default Vault. Every tool below that touches content takes a `vault_id`, and every tool that reads across Vaults takes a `scope`, which is one Vault ID or the literal `all`.

## Model setup

These work with write mode on or off. With the manual tools below and the Vault collection tools, they are the only tools that run before first-run setup finishes.

| Tool | Purpose |
| --- | --- |
| `get_model_setup_status` | Report setup state, the Gemma terms/policy links, and the Nomic fallback notice. No parameters. |
| `accept_gemma_terms` | Accept Gemma's terms, then download it and begin indexing. No parameters. |
| `decline_gemma_terms` | Decline Gemma, remove any partial Gemma download, download Nomic Embed Text v1.5 instead, and begin indexing. No parameters. |

> [!warning]
> Once a model is selected, calling `accept_gemma_terms` or `decline_gemma_terms` again returns an error. Hatchdoor cannot change models after setup.

## Hatchdoor's manual

Hatchdoor carries its own manual, the pages you are reading now, for the version that is running. Two read-only tools read it. They take no Vault, need no search model, and answer whether or not write mode is on and while model setup is still pending, so an agent can look something up before any Vault exists.

| Tool | Parameters | Returns |
| --- | --- | --- |
| `read_docs` | `page` (optional) | With no `page`: the Home page as Markdown, plus `pages`, the `name` and `title` of every page. With a `page`: that page's `name`, `title` and `markdown`. |
| `search_docs` | `query` | `results`: up to five best-matching pages, best first, each with `name`, `title` and an `excerpt`, a line of the page that matched. |

Every page has a short name made from where it sits in the manual, such as `guides/how-to-set-up-a-git-backed-vault`. Links between pages come back as ordinary Markdown links to those names, so `[[The layer system]]` reads `[The layer system](concepts/the-layer-system)`, and a link to a heading adds an anchor, as in `reference/mcp-tools-reference#batch`. Pass either form back to `read_docs`, which ignores the anchor. A name that matches no page is refused with the error code `docs_page_not_found`. The page `whats-new` lists what changed in each release, newest first, and the opening instructions point agents to it. On an install upgraded to 2.8.0, the opening instructions of one session also carry a single sentence saying the optional usage report exists. [[Usage report reference#Where you are asked]] explains it.

`search_docs` matches whole words, ignoring case, a plural `s`, and common words such as `how` and `the`. A page whose title has more of your words ranks above one with fewer, then headings decide, then how often the page mentions them. A word with punctuation inside it, as tool names, setting names, error codes and file names have, is also looked for exactly as typed, ignoring case, in each page's title and text, code blocks included: pages that hold it rank above every page that only holds its parts, and their `excerpt` is the first line that shows it, taken from a code block only when no other line does. So search for a tool, setting or error code by its exact name. It is not the semantic search `search_notes` uses, so search for the words a page would use rather than for a paraphrase. A query that matches nothing returns an empty `results` list, not an error.

The manual is never part of your Vaults. It does not show up in `search_notes`, `get_tree`, `get_stats`, `get_graph` or `list_vaults`, and no tool can change it. Neither tool can go inside `batch`.

## Vault collection: discovery and management

`list_vaults` always works. The other nine need `HATCHDOOR_MCP_WRITE_ENABLED`. Without it they return the same "MCP write tools are disabled" error as the content write tools.

A listed Vault with a remote to poll also carries two RFC 3339 UTC timestamps describing its Git schedule: `last_checked_at`, when Hatchdoor last tried to check the remote, and `next_attempt_at`, when the next scheduled check is due. `last_checked_at` is set whether that check succeeded or failed, so read it next to the Vault's Git status. It is absent until the first check completes, and both are absent for a Vault with no remote and in demo mode. **Git schedule fields on a listed Vault** in [[HTTP API reference]] describes them in full. `list_vaults` returns the same Vault shape as that route.

A listed Vault's `index_turn` is `running` while it indexes and `waiting` while its indexing is queued behind another Vault's or paused to let one through. It is absent when nothing is queued. A `waiting` Vault is not stuck, and its `search` status still says what it can answer meanwhile. The same holds while it is `running`: a Vault that was already searchable reads `search: "stale"` with `capabilities.search: true` for the whole reindex, and `search: "indexing"` only ever means a first index with nothing to search yet. **The `index_turn` field on a listed Vault** in [[HTTP API reference]] describes it.

Each listed Vault also carries its link style: `link_style` (`wikilink` or `markdown`) and `link_path_form` (`relative`, `absolute` or `shortest`). Write new note links and embeds in that form, `[[Note]]` and `![[file.png]]` for `wikilink`, `[Note](path/Note.md)` and `![](path/file.png)` for `markdown`, so the Vault keeps one style. Hatchdoor stores what you send as it is and never converts it. Both fields are absent for a Vault Hatchdoor cannot read and in demo mode. **Link style fields on a listed Vault** in [[HTTP API reference]] describes them, and [[Supported Markdown reference#Which link style Hatchdoor writes]] explains how the style is read.

| Tool | Gating | Purpose |
| --- | --- | --- |
| `list_vaults` | Always | Every Vault's ID, name, status, redacted source, capabilities, Git schedule, and link style, plus the registry's `registry_revision`. Call this first, because every write below needs a fresh `expected_registry_revision`. |
| `create_vault` | Write mode | Create a Vault definition. The registry assigns the Vault ID, which you read back from `list_vaults`. |
| `edit_vault` | Write mode | Replace one whole Vault definition. Send back every field you want to keep. |
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
| `expected_registry_revision` | Yes | Most recent `registry_revision` from `list_vaults`. A stale value is refused, so two writers never overwrite each other. |
| `name` | Yes | |
| `enabled` | No | Default `true`. |
| `source` | Yes | See [[#Vault source shapes]]. |
| `exclude_patterns` | No | Glob patterns, in gitignore syntax, this Vault's index ignores. Default `[]`. |
| `https_credentials` | No | `{ "username"?: string, "token": string }`. Write-only: no read returns it, and `list_vaults` reports only whether one is configured. |
| `archive_folder` | No | This Vault's archive folder, where `archive_note` moves notes. Absent, the instance-wide folder applies. |
| `commit_identity` | No | `{ "name": string, "email": string }`. The name and address this Vault's commits are signed with. Absent, the instance-wide ones apply. |

### `edit_vault`

Same fields as `create_vault`, plus `vault_id` (the Vault to edit) in place of `enabled`. The call replaces the **whole definition**. Read the Vault from `list_vaults`, change what you mean to change, and send the rest back unchanged. Leaving out `exclude_patterns`, `archive_folder` or `commit_identity` clears the stored value.

`https_credentials` is the one exception. It takes one of three actions in place of a value, so an edit never has to resend a stored secret to keep it:

| `action` | Effect |
| --- | --- |
| `keep` | Leave the stored credential untouched. Default if the field is omitted. |
| `remove` | Delete the stored credential. A remote that needs sign-in then fails to sync. |
| `replace` | Set a new `{ "username"?, "token" }`. |

> [!warning]
> Changing a Vault's `source` path, repository URL, branch, or subdirectory is an **identity change**: it points the Vault at different content. It is refused unless you pass `confirm_identity_change: true` **and** the Vault is already disabled, so call `disable_vault` first. On an enabled Vault it is refused whatever the flag says. Changing `mode`, `poll_interval_secs`, `name`, credentials, exclusions, archive folder, or commit identity is not an identity change and needs neither.

### `enable_vault` / `disable_vault` / `disconnect_vault`

All three take `vault_id` and `expected_registry_revision`, nothing else.

### `sync_vault` / `retry_vault`

Both take only `vault_id`. On a Vault with a remote, `sync_vault` asks for a check of the remote now, without waiting for `poll_interval_secs`. `retry_vault` retries an operation that failed, such as one cut short by a network error, without waiting for its backoff.

On a Vault with no remote but with Git history (an `existing_git` Vault in `local_history` mode), both ask for a local commit now. No remote is contacted, and the request also lifts the five-minute pause that follows a failed commit. Only a Vault with no Git at all (a plain `local` source) is refused, with `capability_unavailable`. For rebuilding the search index of any Vault, Git-backed or not, use `refresh_vault`.

### `publish_recovery_branch`

Takes only `vault_id`. When a Two-way Vault's sync stops on a conflict (`git_error.code` is `managed_git_conflict`), Hatchdoor keeps its own side only in its checkout. This tool pushes that side to a branch of its own on the Vault's remote, `hatchdoor-recovery/<configured branch>/<vault_id>`, so a person or an agent with access to the Git host can merge it into the configured branch. Once the configured branch contains the resolution, the Vault's next sync goes through on its own, and `retry_vault` makes that happen now.

It is allowed only while the Vault's `capabilities.publish_recovery` is true, and refused with `capability_unavailable` otherwise. Pending saves are committed first, so the branch carries everything written so far. The push only fast-forwards the recovery branch: it never force-pushes, never touches the configured branch, and never deletes a branch, including after the conflict clears. Calling it again later publishes newer saves to the same branch.

Like `sync_vault`, it returns as soon as the request is admitted (`schedule` is `queued`, or `coalesced` with a request already pending). Read the outcome from `list_vaults`: the Vault's `recovery_branch` names the `branch`, the `published_commit`, the `conflicting_commit` on the remote and `published_at`, or carries an `error`. `managed_git_recovery_diverged` means someone added commits to the recovery branch, so Hatchdoor left it alone and did not overwrite them. `managed_git_recovery_push_rejected` carries the remote's own reason, such as a token that may not create branches. `recovery_branch` clears once a sync succeeds, and is not kept across a restart.

The tool is not allowed inside `batch`.

### `refresh_vault`

Takes only `vault_id`. It asks Hatchdoor to re-scan that Vault's Markdown and republish the snapshot the collection reads: the one `get_tree`, `get_graph`, `get_stats`, `recently_modified` and `search_notes` project from. It contacts no Git remote, and unlike `sync_vault` it works on a Vault with no Git at all.

**Call it when a collection read reports itself stale.** Those reads carry `partial` and a `participants` list (see [[#Read-only content tools]] below). A Vault whose entry reads `stale` is answering from a snapshot known to be behind its files, and `refresh_vault` asks Hatchdoor to bring it up to date.

It returns as soon as the turn is admitted, before the turn finishes:

| `schedule` | Meaning |
| --- | --- |
| `queued` | The Vault's next index turn was admitted. |
| `coalesced` | An index turn for this Vault was already pending, and this request joined it. |

The response means the request landed. It does not mean the index is rebuilt. To see the outcome, run a collection read again and check its freshness fields.

A Vault that cannot be browsed right now is refused with `capability_unavailable`, marked retryable because the Vault may become browsable again. Its `capabilities.browse` is false, which covers a missing or unreadable directory and a Vault that has not started.

> [!note]
> `refresh_vault` rebuilds the read model from the Markdown that is on disk. It does not repair a Vault whose index turn keeps failing: a turn that fails for a fixed reason fails the same way again.

## Vault source shapes

`source` on `create_vault` and `edit_vault` has one of three shapes, picked by `type`. Each shape refuses fields it does not know (`additionalProperties: false`), so a guessed field name is an error and is never ignored.

**`local`** is a plain directory on this machine. Hatchdoor never runs Git for it.

```json
{ "type": "local", "path": "/data/vault" }
```

**`existing_git`** is a Git working copy that already exists on this machine. Hatchdoor uses it in place and never clones it.

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

`mode` is one of `local_history` (commits locally, never contacts a remote), `pull_only` (also fetches), or `two_way` (also pushes). `repository_url` is required for `pull_only` and `two_way`, and may be `null` only for `local_history`.

**`managed_git`** is a remote repository that Hatchdoor clones, in a checkout it owns. It has no `local_history` mode, because a managed Vault exists to track a remote.

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

`mode` is `pull_only` or `two_way`. `repository_url` must be an HTTPS URL with no credentials in it. Send a token through `https_credentials`.

`branch` and `vault_subdirectory` may be `null` or absent on every Git shape. A `null` branch tracks the remote's default branch, and a `null` subdirectory uses the repository root. `poll_interval_secs` is at least `60` and defaults to `86400`. `local_history` ignores it, having no remote to poll.

## Read-only content tools

These work whenever MCP is on, with write mode on or off.

| Tool | Required parameters | Purpose |
| --- | --- | --- |
| `search_notes` | `scope`, `query` | Search one Vault or all enabled Vaults. Optional: `mode` (`semantic` default or `keyword`), `limit` (1 to 50, default 10), `per_note_cap` (1 to 10, default 2), `layers` (array of layer names to include), `detail` (`compact` default or `full`). Hits are compact by default: each carries `vault_id`, `note_slug`, `note_title`, `note_path`, `heading_path`, `score`, `layer` and a `snippet`, which is enough to pick a note and read it with `get_note`. See [Compact and full search hits](#compact-and-full-search-hits). Each result's `score` runs 0 to 1. In semantic mode it is the cosine similarity between the query and the chunk, so it can be compared across searches. In keyword mode it is relative to the best hit of that search, which scores 1. A query that is a single `#tag` (letters, digits, `-`, `_` and `/` after the `#`) runs as a tag match whatever `mode` was requested: every hit scores 1 and the response reports `"mode": "tag"`. Every other query reports the mode it asked for. `tag` is never accepted as a requested mode. |
| `get_note` | `vault_id`, `slug` | Read one exact note's Markdown, straight from its file. `note.metadata` holds the frontmatter's `tags`, `aliases` and every remaining top-level key under `properties`, the same values `get_frontmatter` returns, so a note with no frontmatter block answers with two empty lists and an empty `properties` object. `note.metadata` is `null` when the frontmatter does not parse: the note still comes back whole, with its `content` and `content_hash`, so you can repair it, where `get_frontmatter` refuses with `invalid_frontmatter`. Tags written in the note's text as `#tag` are not included. Also lists the note's saved queries under `saved_queries`, by name, without evaluating them. |
| `get_note_outline` | `vault_id`, `slug` | List one exact note's headings with the size of each section, plus the note's `content_hash` and its sizes, without any note text. See [[#Reading part of a long note]]. |
| `get_note_section` | `vault_id`, `slug`, `headings` | Read whole sections of one exact note, picked by heading text or heading path. `headings` is a list of 1 to 10 strings. See [[#Reading part of a long note]]. |
| `get_note_links` | `vault_id`, `slug` | Outgoing links and backlinks for one exact note, wikilinks and Markdown links to `.md` files alike. |
| `resolve_wikilink` | `vault_id`, `target` | Resolve a wikilink target within one Vault. |
| `get_tree` | `scope` | Grouped explorer tree for one Vault or all enabled Vaults. Optional: `folder` (a Vault-relative folder to return as the root), `max_depth` (how far below it to descend, minimum 1), `include_notes` (default `true`). |
| `get_stats` | `scope` | Grouped statistics for one Vault or all enabled Vaults. Also reports `hatchdoor_version`, the running instance's version string. |
| `get_graph` | `scope` | Grouped link graph for one Vault or all enabled Vaults. |
| `get_frontmatter` | `vault_id`, `slug` | Read one exact note's frontmatter without its Markdown body: `tags`, `aliases`, and every other top-level key under `properties`. A note with no frontmatter block answers `has_frontmatter: false` with empty lists, which is not an error. It also returns the note's `content_hash`, the same string `get_note` reports for that note at that moment. The hash covers the whole file, so a note with no frontmatter block has one too, and you can write metadata back without reading the body first. |
| `recently_modified` | `scope` | Recently modified notes. Optional `limit` (1 to 25, default 5). |
| `query_notes` | `scope`, `conditions` | Select the notes whose tags, path, or frontmatter properties satisfy stated conditions. Optional: `properties` (names to return on each row), `limit` (1 to 200, default 50). See [[#Selecting notes by tag, path or property]]. |
| `find_text` | `scope`, `text` | Find every note that contains one literal string, with how many times each contains it. Optional: `case_sensitive` (default `false`), `layers`, `path_prefix`, `limit` (1 to 500, default 50), `snippets_per_note` (0 to 3, default 3). See [[#Finding every note that contains a string]]. |
| `evaluate_saved_query` | `vault_id`, `slug` | Evaluate one saved query in a note and return the notes it selects as rows. Optional `name`, required when the note holds more than one. See [[#Reading a note's saved queries]]. |
| `list_note_attachments` | `vault_id`, `slug` | List the attachments one note references, without the note's full content. Every non-Markdown file the note points at counts, including those Hatchdoor cannot display. |
| `get_attachment` | `vault_id`, `relative_path` | Fetch one attachment's bytes, addressed by the same `relative_path` `list_note_attachments` reports. Only `png`, `jpg`, `jpeg`, `gif`, `webp`, `svg`, `avif`, `bmp` and `pdf` can be fetched, so a video or data file is listed but refused here. Optional `encoding`: `url` (the default) returns a `download_url` transfer link that needs no token, `base64` returns the bytes inline. |
| `get_attachment_import_config` | `vault_id` | Report whether this Vault accepts uploads right now, the available methods, their byte limits, and the allowed file extensions. Call this before uploading. |

Collection-scoped results (`search_notes`, `get_tree`, `get_stats`, `get_graph`, `recently_modified`, `query_notes`, `find_text` with `scope: "all"`) carry `scope`, `collection_revision`, `partial`, and `participants`. Branch on the structured error `code`, never on message text. `partial: true` is not an error: it means at least one Vault's part of the result is not current. It is true whenever any entry in `participants` reads something other than `fresh`: `stale` (answered, but possibly behind that Vault's Markdown, usually a prior index generation while a turn catches up), `not_searchable` (semantic search only: some or all of the notes searched in that Vault have no embeddings, because they are not embedded yet or sit in a demoted layer with `HATCHDOOR_EMBED_LAYERS` off; any hits it returned still count, and Keyword search reaches every note), or `unavailable` (sat out, with the reason on the entry). A Vault can sit out for more than one reason: its snapshot could not be read, or, on `get_tree`, it does not have the folder that was asked for. So read the reason from that Vault's entry in `participants` and do not guess it from `partial`. An agent can act on an entry that reads `stale`: that Vault's snapshot is known to be behind its Markdown, and `refresh_vault`, under [[#Vault collection: discovery and management]], asks for the index turn that republishes it. Usually that turn is already queued, so a second read a few seconds later is often enough. A write can take up to about five seconds to reach the index, so a read straight after one can still report `fresh` without it.

`collection_revision` is not a content version and cannot tell whether a result includes a write. It counts changes to the Vault collection's status: a Vault added, edited, enabled, disabled or disconnected, or one Vault's index, Git, watcher or local-file status moving. A note write does not advance it. The index turn that follows usually moves it by two, only because the Vault's index status passes through `stale` and back. Read `participants` to judge freshness.

`get_tree` with nothing but `scope` returns the whole Vault, which at a few hundred notes can be more than a client accepts in one result. Three optional arguments narrow it. Start with `include_notes: false`: it returns every folder at every level with its note count and no notes, so the shape of a Vault of several hundred notes costs about a kilobyte where the full tree costs seventy. `folder` returns one subtree, such as `"40-reference/Parenting"`, matched without regard to case and with surrounding slashes ignored. `max_depth` stops the descent: the starting folder is depth 0, a folder at the limit is listed with its count but not opened, and one that had something inside it is marked `truncated` so it cannot be mistaken for an empty leaf. Every folder reports `note_count`, the notes directly inside it without its subfolders, and a subtree's total is the sum of those. A `folder` the Vault does not have answers the structured error `folder_not_found`, so a typo never reads as an empty folder. With `scope: "all"` that refusal is per-Vault: the Vaults that do have the folder still answer, each Vault that does not appears in `participants` carrying `folder_not_found`, and the result is marked `partial`. Only when no Vault has it does the whole call refuse, and that refusal names no Vault. The per-Vault refusals stay on `participants`, where each one names its own Vault.

Notes inside a tree carry `title` and `slug` but no `vault_id`: the tree they sit in already names its Vault, once. Results that mix Vaults in one flat list still put `vault_id` on every hit: `search_notes`, `recently_modified`, `query_notes` and `find_text`.

### Compact and full search hits

`search_notes` returns small hits unless you ask for more, because most searches only locate a note that `get_note` then reads. A hit that carried the whole matched chunk would make a search near the defaults cost 20 to 50 KB.

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

`query_notes` answers the requests that a note's tags, folder or properties decide on their own. Every note tagged `project/active`. Everything under `40-reference`. Notes whose `review-date` has passed. It does not search. A note either meets the conditions or it does not, the results are unranked and come back in the same order every time, and no embedding is involved, so a Vault that is still indexing answers in full. Use `search_notes` when the question is about what a note says, and `query_notes` when it is about what a note is.

`conditions` is a list, and every one of them must hold for a note to be selected. At least one condition is required. To read a Vault without selecting, use `get_tree` or `recently_modified`. There are three kinds:

- `{"type": "tag", "tag": "project/active"}`. The leading `#` is optional, case does not matter, and nested tags are included: `project` selects `project` and `project/active`, never `projects`.
- `{"type": "path_prefix", "prefix": "40-reference/Parenting"}`. A folder relative to the Vault, matched without regard to case and by whole path segment, so `notes` never selects `notes-archive`.
- `{"type": "property", "name": "review-date", "operator": "lt", "value": "2026-09-06"}`. One frontmatter property, tested by one operator.

The property name is matched exactly, capitalisation included. `tags` and `aliases` are refused as property names, because Hatchdoor keeps both in lists of their own and a property test would report them missing on every note. Select tags with a tag condition. You can still have both returned by naming them in `properties`.

`query_notes` also differs from search in that it reads every [[The layer system|layer]] and takes no `layers` argument, so it can select a demoted note that an ordinary search would not return. The explorer and `get_tree` behave the same way. Demotion takes a note off the default search surface only.

There are ten operators. `eq`, `ne`, `lt`, `lte`, `gt` and `gte` each need a `value`. `exists`, `missing`, `empty` and `not_empty` must not carry one. Four rules to know:

- Only `missing` selects a note that never mentions the property. `ne` does not: a note with no `status` does not have a status different from `draft`.
- `eq` against a multi-value property matches when any entry equals the value, so `for: [reference, archive]` is equal to `reference`.
- Ordered comparison works between two numbers, two strings or two booleans, and selects nothing across types. Strings compare byte by byte, which puts ISO-8601 dates and timestamps in the right order. That is how "before today" works, with today's date passed as the value.
- `empty` means present but holding nothing: bare `status:`, a blank string, an empty list, an empty mapping. A note that has no `status` line at all is `missing`, not `empty`.

`properties` names the frontmatter to return on each row. A name a note does not carry is left off that row and never returned as null, so "no value" and "the value is null" stay apart. Rows are ordered by path, then Vault, then slug, and `limit` is applied to that order across the whole collection, so two identical calls return identical rows. When more notes qualified than `limit` allowed through, the result says `truncated: true`, so a full page is never mistaken for a complete answer.

A query Hatchdoor cannot answer is refused with the structured error `invalid_query`, which names what is wrong, before any Vault is read. Examples are a query with no conditions, a comparison with no value, and an operator given a value it does not take.

### Finding every note that contains a string

`find_text` answers exact questions. Does anything still say the old name? Did a bulk edit land everywhere? Which notes cite this path or ID? It returns every note that contains the string you give it, and how many times. `search_notes` cannot answer these: both of its modes rank, keyword mode accepts a note that holds any one word of the query, and the number of hits it returns is not a count of occurrences.

`text` is one literal string. Every character means itself, so `[[a.b]]`, `*` and `(` are matched as written, and there are no wildcards, no patterns and no second string in the same call. An empty string, or one longer than 4,096 bytes, is refused with the structured error `invalid_text_match`.

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

`get_note` returns the whole file. For a long rules note or runbook where you want two sections, most of that is wasted. `get_note_outline` and `get_note_section` read part of a note.

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
- An entry that selected nothing has `requested` and `error`, with `code` `heading_not_found` or `heading_ambiguous`. An ambiguous one lists every matching heading path under `matches`, so you know what to send next. No request can tell apart two headings with the same full path. Read the enclosing section, or the whole note, for those.

One bad heading never fails the call: the other entries still come back. Only a malformed list does. An empty list, an empty string or more than 10 strings is refused with the structured error `invalid_heading_selection`. A note that does not exist is `note_not_found`, as with `get_note`.

The frontmatter and the opening text are not sections and cannot be requested. Use `get_frontmatter` for the properties, or `get_note` for everything.

The text comes back in a field named `section`, never `content`. It is part of a note. Do not pass it to `update_note`, which replaces the whole note. To change a section, pass the reply's `content_hash` to `replace_section` or `edit_note`. `replace_section` names its heading with the `#` characters, which the outline's `level` gives you: a level 2 heading with the text `Filing` is `## Filing`.

Both tools read the note's file when the call runs, as `get_note` does.

### Reading a note's saved queries

A note can hold saved queries, fenced `base` blocks that describe which notes to list (see [[Supported Markdown reference]]). The note page draws each one as a table. An agent gets the same rows as data in two steps.

`get_note` returns the note as its file holds it, `base` blocks and name markers included, and nothing computed. Beside the note it returns `saved_queries`, one entry per block in the order they appear, each with the `name` its `<!-- hatchdoor-query: name -->` marker gives it, or `null` when it has none. That is how an agent learns what it can ask for without reading the definitions.

`evaluate_saved_query` then evaluates one of them and returns the notes it selects. Pass the note's `vault_id` and `slug`, plus `name`. You can leave `name` out only when the note holds exactly one saved query. The agent never reads or rewrites the definition, so it cannot drop one of its conditions along the way. An agent that rebuilt the definition as a `query_notes` call could drop one, which is why this tool exists.

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
> The `enabled` field of `get_attachment_import_config` is true only when two things are: `HATCHDOOR_MCP_WRITE_ENABLED`, for the whole instance, and the target Vault's own `capabilities.mutate`. When `enabled` is `false`, the response says which of the two is false.

`get_attachment` is the upload flow in reverse. `encoding: "url"` returns `content.download_url`, a **transfer link**: a full address, built on **Public address** (`HATCHDOOR_PUBLIC_URL`) when that is set and otherwise on the address the client reached the server on, as a proxy reports it in `Forwarded` or `X-Forwarded-Proto`/`X-Forwarded-Host`, that carries its own credential for this one file. Fetch it as it is, with no `Authorization` header, for example `curl -o manual.pdf '<download_url>'`. It answers any number of times until `content.expires_at` (Unix seconds), five minutes after it was issued. `path_note` and `auth` say the same. An agent inside an MCP client holds neither the MCP password nor the server's address, and a transfer link needs neither. A client that cannot make an out-of-band HTTP request calls `get_attachment` again with `encoding: "base64"` and gets `content.content` inline, bounded by the same `HATCHDOOR_MCP_MAX_BASE64_BYTES` cap `import_attachment` uses on the way in. A file over it is refused with the structured error `attachment_too_large_for_base64`, whose message gives the measured size. Call again with `encoding: "url"`. Either way the result carries `vault_id`, `relative_path`, `size_bytes`, and `content_type`.

> [!note]
> A download link is held to the same limits as fetching the bytes over `/mcp`: the same `HATCHDOOR_MCP_MAX_BASE64_BYTES` ceiling and the same rate quota, drawn from the same counter. A larger attachment returns `413` with `asset_too_large` until you raise the setting, and a request over the quota answers `429` with `Retry-After`. The link costs less to use and allows no more. It is refused with `403` for any other file or Vault (`transfer_link_invalid`), after it expires (`transfer_link_expired`), after a restart or an MCP password change (`transfer_link_invalid`), and while MCP is off (`mcp_disabled`). Ask again for a fresh one. See [[The security model]].

## Write content tools

Every tool below requires `HATCHDOOR_MCP_WRITE_ENABLED=true` and takes `vault_id` in addition to the parameters listed. Every mutating tool that targets an existing note also requires `expected_content_hash`, the hash most recently read from `get_note`, or from `get_frontmatter`, `get_note_outline` or `get_note_section` when the whole body is not needed. A stale hash means someone else changed the note since you read it, and the write is refused so it cannot overwrite their change.

That protection has one limit. Hatchdoor normally commits a save by swapping the new copy of the note with the old one in a single step, which is how it detects a change that lands during the save. Some filesystems cannot do that swap: ZFS before 2.2, anything mounted through FUSE, and a Windows folder used through Docker Desktop. There a save checks the note and then replaces it, in two steps, and a change that lands between the two is overwritten without being reported. A stale hash is refused either way, and Hatchdoor runs its own writes one at a time on every filesystem. MCP does not show which case a Vault is in: `list_vaults` capabilities do not carry it. The answer is on the HTTP route `GET /api/v1/vaults/{vault_id}/write-capabilities` and in one line per Vault in the server log.

A write is refused before it touches the file, so a rejected write has changed nothing and can be retried against a fresh hash. The exception is `write_recovery_required`, which means the opposite: the new content was saved and then could not be checked or undone, because something outside Hatchdoor changed the Vault directory mid-write. Do not retry it. The message names the note and the leftover file holding the previous content, and a person has to decide what the note should say.

| Tool | Required parameters (beyond `vault_id`) | Purpose |
| --- | --- | --- |
| `create_note` | `relative_path`, `content` | Create a Markdown note. Hatchdoor creates missing parent folders. Fails if the note exists unless `overwrite: true`. |
| `update_note` | `slug`, `content`, `expected_content_hash` | Replace a note's full content. |
| `append_to_note` | `slug`, `content`, `expected_content_hash` | Append content to a note. |
| `edit_note` | `slug`, `old_string`, `new_string`, `expected_content_hash` | Replace one exact string. `old_string` must match exactly and occur once, unless `replace_all: true`. Otherwise the edit is refused and nothing is written. Use this in place of `update_note` for small changes. |
| `replace_section` | `slug`, `heading`, `mode`, `content`, `expected_content_hash` | Replace or insert around a Markdown section identified by its heading. `mode` is `replace`, `before` or `after`. With `replace`, `content` overwrites the section and should include the heading. The section runs from the heading to the next heading of the same or a higher level. Headings inside fenced code blocks are ignored, and the heading must match exactly and occur once. |
| `update_frontmatter` | `slug`, `frontmatter`, `expected_content_hash` | Merge top-level keys into the note's YAML frontmatter and leave the body alone. Only the keys you name change. The rest of the block keeps its formatting: key order, one-line lists, indentation, quoting and comments. A `null` deletes a key. A nested mapping is replaced whole, never merged. A list you replace keeps the shape it had, and a key that did not exist is added at the end of the block with any list on one line. The whole call is refused, writing nothing, when a key you named cannot be changed without guessing, which in practice means a note whose block writes that key twice. A note with no frontmatter block gets one created, and deleting its last key removes the block. Rejects an empty `frontmatter` object, and a creation whose values are all `null`. |
| `rename_note` | `slug`, `new_title`, `expected_content_hash` | Rename within the current folder, and rewrite backlinks in wikilink and Markdown link form. The note keeps its folder, so the assets inside that folder stay where they are and `moved_assets` comes back `0`. |
| `move_note` | `slug`, `target_folder`, `expected_content_hash` | Move to a target folder, with the same backlink handling as a rename. The assets inside the note's own folder move with it. |
| `move_rename_note` | `slug`, `target_relative_path`, `expected_content_hash` | Move and rename in one operation. |
| `archive_note` | `slug`, `expected_content_hash` | Move to the configured archive folder (the Vault's own `archive_folder`, set via `create_vault`/`edit_vault` above, or the instance default). |
| `delete_note` | `slug`, `expected_content_hash` | Trash a note under `.hatchdoor-trash`, remove backlinks to it and trash the assets inside its own folder. A Markdown link to it keeps its text as plain words. |
| `rename_tag` | `old_tag`, `new_tag` | Rename a tag, and every tag nested under it, across the whole Vault. Called without `expected_plan_hash` it only plans; called with the `plan_hash` that plan returned, it applies it. See [[#Renaming a tag across a Vault]]. |
| `delete_tag` | `tag` | Remove one tag from every frontmatter `tags` list in the Vault. Planned and applied in two calls like `rename_tag`. Refused while any note carries a tag nested under it or carries it inline in its body. See [[#Deleting a tag across a Vault]]. |
| `import_attachment` | `content` (base64), `target_relative_path` | Upload an attachment by sending its bytes base64-encoded. This is the **fallback** for clients that cannot make an HTTP request outside MCP, limited to `HATCHDOOR_MCP_MAX_BASE64_BYTES` (default 5 MiB decoded). Use `create_upload_link` whenever the client can make an HTTP request, and call `get_attachment_import_config` first to see the current limits. |
| `create_upload_link` | `target_relative_path` | Mint an upload transfer link for one file, the recommended upload route for an attachment and the only way to import an existing Markdown file as a note: a target ending in `.md` is a note upload. Optional `overwrite` (default `false`) allows replacing an existing file, and replacing a note also needs `expected_content_hash`. Returns `upload_url`, `method` (`POST`), `upload_kind` (`note` or `attachment`), `expected_content_hash` (echoed, or `null`), `expires_at` (Unix seconds), and `max_bytes` (`HATCHDOOR_MAX_ATTACHMENT_BYTES`). Send the file to `upload_url` as `multipart/form-data` in a field named `file`, with no token: `curl -F file=@scan.pdf '<upload_url>'`. A `target_relative_path` form field is optional and must match. See [[#Upload links]]. |
| `move_attachment` | `source_relative_path`, `target_relative_path` | Move an attachment and rewrite every note reference to it. |
| `rename_attachment` | `source_relative_path`, `new_filename` | Rename an attachment in place and rewrite every note reference to it. |
| `delete_attachment` | `source_relative_path` | Trash an attachment under `.hatchdoor-trash` and rewrite every note reference to it. |

`move_attachment`, `rename_attachment` and `delete_attachment` act on bytes the Vault already stores, so the list of uploadable types does not apply to them: any file that is not Markdown qualifies, whatever its extension, including files with no extension at all. Four things are refused. A `.md` target, because moving a note this way would skip backlink rewriting, slug handling and the hash check. Use the note tools. A `.hatchdoor-layer` marker, because trashing one would put a whole folder back on the default surface without anyone asking. Anything under `.git`, which is the Vault's repository and not its content. And any path the Vault excludes as noise, on the source side as well as the destination, which is what keeps these tools out of `.obsidian/`. Reference rewriting still keys on the extension, so a link to an extensionless file is left as written when that file moves. [[How to import and work with attachments]] explains the three lists (uploadable, managed and fetchable) and how they differ.

A backlink to a note that `rename_note`, `move_note`, `move_rename_note` or `archive_note` retargets keeps the form it was written in. A link written without a folder path, `[[Some Note]]`, stays bare and picks up the note's new title, spelled as the note's filename. That includes a link that only reached the note through its slug, such as `[[some-note]]` or a title typed with an em dash where the filename has a hyphen. A link written as a full path picks up the new full path. The one exception is a new title that another note already carries, where the link falls back to the full path so it keeps pointing at the note that moved. A move that does not change the note's title therefore leaves bare-title links alone, though a slug-form or punctuation-drifted link still comes back spelled as the title. The note being renamed or moved is one more note holding links to it, so a link in its own body that points at itself follows the same rules: after a rename a bare-title self-link picks up the new title, and after a move a path-qualified self-link picks up the new folder while a bare one keeps its bare form. `rewritten_notes` counts only the other notes that changed, so a rename whose one stale link was the note's own comes back as `0`. The `content_hash` in the same response covers the note's own new text. `delete_note` is the exception, and leaves the trashed copy's link to itself as written. A link inside a table cell writes its alias pipe escaped, as `[[Some Note\|alias]]`, because a bare `|` would end the cell. That escape is part of the form as well: the link is retargeted and the backslash is handed back, so the cell stays a valid table row.

A Markdown link to a note, `[text](../folder/Some%20Note.md)`, is a backlink too and is retargeted in the same write as any wikilink in that file. It keeps its form: a relative path is worked out again from the linking note, a path starting with `/` keeps it, a path written from the Vault root stays so, and a bare filename stays bare while that name still reaches the note, otherwise the relative path is written. Only the path changes, never the link text, heading anchor or title, and a reference-style link changes on its definition line. The moved note's own Markdown links are repointed from its new folder so they reach the same notes. `delete_note` removes each Markdown link to the deleted note and leaves its text as plain words. [[Supported Markdown reference]] has the full rules.

An asset travels with a note only when it already lives inside that note's own folder, or a subfolder of it. An asset the note points at from somewhere else, such as a shared `_system/` or `Attachments/` folder beside the note's folder, stays where it is: `rename_note`, `move_note`, `move_rename_note`, `archive_note` and `delete_note` leave it alone and rewrite the moved note's own link so it still resolves from the note's new home. Other notes pointing at it are left alone too, since nothing about it changed. `moved_assets` in the response counts only the assets that moved.

A note Hatchdoor cannot read as text does not block these tools. One it cannot open at all, such as a `.md` symlink whose target is gone, is skipped, the way the search index skips it. One that is not valid UTF-8, a Latin-1 export from an older tool for example, is checked for links all the same, and left byte for byte as it was when it has none to what is moving. When it does link to the note being renamed, moved, archived or deleted, or to an attachment being moved, renamed or deleted, Hatchdoor cannot fix that link without rewriting the bytes that are not text. The whole call is then refused with `link_rewrite_unsupported`, nothing is written, and the message names every such note at once so they can all be fixed, by re-saving them as UTF-8, before trying again.

A note rewritten only to keep its links pointing at the right place keeps the modification time it had. That covers every note `rewritten_notes` counts, for `rename_note`, `move_note`, `move_rename_note`, `archive_note`, `delete_note` and the attachment tools, and it holds when a failed call puts those notes back. So `recently_modified` and the Stats page do not show a batch of renames as a wave of edits. The note you rename or move keeps its own modification time too. A note you edit directly with a write tool takes the current time as always. If the filesystem refuses the old time, the write still succeeds and the note carries the time of the write.

> [!note]
> A note sitting in the Vault root has the whole Vault as its own folder, so every asset it references counts as living inside it and does travel with the note when it moves to another folder. A rename keeps the note where it is, so nothing travels. Keep notes that share an attachments folder in a folder of their own if you want that folder left alone.

Every write tool accepts an optional `commit_summary`, a one-line string describing what the change was for. On a Vault with versioning enabled it reaches the body of the commit that records that write. One commit usually covers several writes, since Hatchdoor commits a few seconds after writing stops and not once per call: the subject names the first few operations and how many files they touched, and the body lists one `- ` line per summary, in the order the writes happened. A write with no summary still shapes the subject. Pass one on every write and the Vault's history reads as a log of why each note changed.

> [!warning]
> No write tool can create, rename or move a file named `.hatchdoor-layer`, the layer marker. The call is refused, because a marker changes how a whole folder is classified and is meant to be edited by hand in the Vault. A write is also refused when its target path matches the Vault's ignore patterns, because the file would be written to disk and then be invisible to every read.

### Upload links

`create_upload_link` checks the target before it answers and refuses at once, with the same codes `import_attachment` would use, when the path is invalid, the extension is not uploadable, or a file is already there and `overwrite` is `false`. So an agent never sends a file to a link that cannot work. The link is good for one upload to that path under that `overwrite` rule. It is spent by its first use, whether that upload succeeds or not, and it expires five minutes after it was minted. The existence check runs again when the file arrives, so a file that appeared in between is refused with `409 write_conflict` and not overwritten. The upload answers `403` for any other target (`transfer_link_invalid`), a second use (`transfer_link_spent`), expiry (`transfer_link_expired`), a restart or an MCP password change (`transfer_link_invalid`), MCP being off (`mcp_disabled`), and MCP writes being off (`mcp_write_disabled`). A success returns the same shape as `import_attachment`.

A target ending in `.md`, in any case, is a **note upload**. The file becomes a note through the same write `create_note` performs, so its content never passes through the conversation and a long file cannot be cut short on the way in. It gets the same treatment as `create_note`: it must be UTF-8 with no NUL bytes (otherwise `400 invalid_write_input` and nothing is written), CRLF and CR line endings become LF, a missing final newline is added, and `quality_warnings` names each change. The content type the upload declares is ignored. A success returns what `create_note` returns: `slug`, `relative_path`, `content_hash`, `layer` and `quality_warnings`. A path with no extension is not a note upload, even though `create_note` would add `.md` to it. An upper-case extension is written in lower case, so `Report.MD` becomes `Report.md`.

Without `overwrite`, a note upload creates the note and is refused at minting, and again at upload with `409 write_conflict`, when a note is already there. Replacing a note takes `overwrite: true` and `expected_content_hash`, the note's current hash from `get_frontmatter` (which returns it without the body) or `get_note`. Minting refuses a replacing link with no hash (invalid params), a stale hash (`write_conflict`), or a note that does not exist (`note_not_found`). The hash is signed into the link, so it cannot be swapped. The upload writes only if the note still has that hash, and otherwise fails with the same `409 write_conflict` `update_note` gives, leaving the newer edit in place. The answer's `content_hash` is what the next replace needs. `expected_content_hash` anywhere else, on an attachment target or a link that does not replace, is refused as invalid params.

### Renaming a tag across a Vault

`rename_tag` changes a tag everywhere it appears: in frontmatter `tags` lists, and in namespaced `#hashtags` in note bodies. It always takes two calls.

1. Call it with `old_tag` and `new_tag`. Nothing is written. The answer is the plan: `notes` lists every note that would change, with its `slug`, `relative_path`, and whether the change is in its `frontmatter`, its `body`, or both; `frontmatter_notes` and `body_notes` count each; and `plan_hash` is the plan's fingerprint.
2. Call it again with the same tags and `expected_plan_hash` set to that `plan_hash`. Hatchdoor plans again and applies the plan only if it comes out identical. If any note changed in between, the call is refused with `tag_rename_plan_stale` and nothing is written. Plan again and look at the new plan.

The fingerprint is a hash of the edits, and the server keeps no record of it, so a plan never expires by age. It stops working when the Vault no longer produces the same plan.

Renaming a tag renames everything nested under it: `domain` renames `domain/homelab` to `topic/homelab`, even when no note carries `domain` itself. Matching ignores case, as tag search does, and `new_tag` has to be lowercase letters, digits, `-`, `_` and `/`, with no empty segment. A leading `#` is fine on either. A rename that would not settle is refused, because running it again would rename it again: a `new_tag` nested under `old_tag`, like `domain` to `domain/old`, and a rename into an ancestor that turns some note's tag into one still under `old_tag`, like `domain/x` to `domain` when a note carries `domain/x/x`. Any refusal of the names comes back as `invalid_tag_name`.

Renaming into a tag that already exists is a merge, and is allowed. A note that would end up carrying the target twice keeps it once, in the earlier of the two places. `already_tagged_notes` counts the notes that carried `new_tag`, or a tag under it, before the rename. Above zero, the rename is a merge.

Only the renamed characters change. A one-line `tags: [a, b]` list stays on one line, a list written one item per line stays that way, and key order, quoting and every other byte of the note stay as they were. A hashtag inside a fenced code block or an inline code span is not a tag, so it is left alone, the same rule the index follows.

It is all or nothing. When a note carries the tag in a way that cannot be changed without restyling or guessing, the plan is refused with `tag_shape_unsupported`, the message names every such note and why, and no note is written. The cases are a frontmatter list the editor would reformat (quoted items, extra spaces inside the brackets, a comment on the line), frontmatter that is not valid YAML, a body hashtag interrupted by a code span, and a body hashtag that would lose its `/` and so stop being a tag. Fix those notes in the Vault and plan again. If a write fails partway through applying, every note already rewritten is put back. Each note the rename rewrites keeps its modification time, so retagging a Vault does not show up as a wave of edits. Hatchdoor holds the Vault's write lock for the whole call, so a Vault with versioning enabled records the rename as a single write in its next commit. The lock does not bind a person editing the same Vault in Obsidian, so each note is written only if it still matches what the plan read: a note saved by hand while the rename is running stops the call at that note, everything already rewritten is put back, and the hand-written save is left as it was. On a Vault whose filesystem cannot swap two files in one step, see the note under **Write content tools**, a hand save landing inside the moment between that check and the write is overwritten instead of stopping the call.

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

A write conflict is reported as a retryable tool error: a stale `expected_content_hash`, or a registry revision that moved under a Vault-management call. Read the current state again and retry. The operation is safe to repeat.

Any read or write refused this way says so twice: `isError` is true on the result itself, and the structured result carries `ok: false` beside the `code`, `message`, `retryable` and, where the failure names a Vault, `vault_id`. Branch on whichever of the two you already read. The second signal matters if you read the structured result as the tool's typed answer, since the schema each tool advertises describes only the success payload and a refusal arrives in a different shape. The rule is that an `ok` field present and false means the call did not happen: a successful write still returns `ok: true`, and a successful read carries no `ok` field. The exception is the few setup-state refusals, returned before setup finishes or once a search model is chosen, which carry a plain sentence and no structured result to read.

When the manual explains the refusal, the structured result also carries `docs`, naming the page to read: `page` is a name `read_docs` accepts and `heading` is the section on that page, as in `{"page": "guides/how-to-troubleshoot-common-problems", "heading": "git-sync-is-failing"}`. It comes with the codes for a Vault that is unavailable or disabled (`vault_unavailable`, `vault_read_unavailable`, `vault_disabled`), a folder Hatchdoor cannot read (`vault_path_unavailable`, `vault_path_unreadable`), a Vault that cannot do what was asked right now, such as refreshing one that cannot be browsed (`capability_unavailable`), a failed write (`write_failed`, `write_recovery_required`), a Git sync failure (`managed_git_conflict`, `managed_git_authentication_failed`, `managed_git_remote_unreachable`, `managed_git_install_failed`, `managed_git_push_rejected`, `managed_git_dirty_working_copy`, `managed_git_operation_in_progress`, `managed_git_pull_only_local_commits`, `existing_git_local_history_manual_recovery_required`), and a registry that needs recovery (`vault_registry_recovery_required`). Any other error has no `docs`, and neither has an item error inside `batch`. The writes-off refusal (`-32602`) and the setup-state refusals stay as they are.

## Batch

`batch` runs an ordered list of the tools above in a single call. One tool batches them all. Each item names an `op` and carries that tool's own `arguments` as a standalone call would, `vault_id` included, so one batch can span several Vaults.

`operations` is the only thing `batch` itself takes. Unlike every other tool it has no `vault_id` and no `commit_summary` of its own. Each goes inside the `arguments` of the items whose tool takes it. A call that sends a field `batch` does not know is refused before any item runs, and the refusal names every unknown field at once, at both levels, together with the shape it expects.

```json
{
  "operations": [
    {"op": "create_note", "arguments": {"vault_id": "<id>", "relative_path": "Inbox/Draft.md", "content": "# Draft\n"}},
    {"op": "update_frontmatter", "arguments": {"vault_id": "<id>", "slug": "inbox/draft", "frontmatter": {"tags": ["status/draft"]}, "expected_content_hash": "ignored-here"}},
    {"op": "get_note", "arguments": {"vault_id": "<id>", "slug": "inbox/draft"}}
  ]
}
```

**What may go in.** Every read tool except `list_vaults`, `read_docs` and `search_docs`, and every note and attachment write tool, from `create_note` to `delete_attachment`, deletes included. `rename_tag` and `delete_tag` are the exceptions: each touches every note carrying a tag and promises all or nothing, which a best-effort batch cannot keep, so call them on their own. Vault-management tools (`create_vault`, `edit_vault`, `enable_vault`, `disable_vault`, `disconnect_vault`, `sync_vault`, `retry_vault`, `publish_recovery_branch`, `refresh_vault`) and the model-setup tools are not batchable, and neither is `batch` itself. An unknown or disallowed `op`, an empty `operations` array, more than **50** read-shaped items, or more than **20** write-shaped items rejects the whole call before any item runs.

**Best-effort, in order, no rollback.** Items run one after another. An item that fails never stops the ones after it, and nothing already written is undone. An item sees the Vault as it is and knows nothing of the batch around it, apart from the hash chaining below.

**Permission is still per item.** `batch` itself works with write mode off, because a batch may hold only reads. Each write-shaped item is checked as the same standalone call would be: with `HATCHDOOR_MCP_WRITE_ENABLED=false` the read items succeed and the write items fail individually with the code `mcp_writes_disabled`, and a Vault whose `capabilities.mutate` is false refuses its writes the same way.

**`expected_content_hash` inside a batch.** Once a batch has written a note, later items in that same call targeting the same `vault_id` + `slug` have their `expected_content_hash` replaced with the hash that write produced. You cannot know that hash without the round trip a batch exists to avoid, so send any placeholder and Hatchdoor discards it. This applies to `create_note` too: create a note and edit it later in the same call. A note the batch has not written yet has its `expected_content_hash` checked as in a standalone call. The substitution is safe because Hatchdoor takes the write lock on every Vault the batch writes to before the first item runs, and holds them all until the call ends, so no other Hatchdoor writer can get in behind a substituted hash. That lock does not bind Obsidian. An outside editor is stopped by the swap each save makes, described under **Write content tools**, and on a filesystem that cannot do the swap the same narrow race applies here.

**Result shape.** `items` holds one entry per requested operation, with `index`, `op`, `ok`, and then either `result`, which is that tool's normal result, or `error`. `succeeded` and `failed` count them. A batch that ran at all returns success at the tool level. To find out what happened, read `failed` and each item's `ok`, never the call's own status. An item's `error` is the same `{code, message, retryable, vault_id?}` object a standalone refusal carries, and its `code` is always a string to branch on. A refusal that a standalone call reports as a JSON-RPC error, such as malformed arguments or a missing `vault_id`, becomes `invalid_arguments` inside a batch, with the JSON-RPC number kept as `jsonrpc_code`.

**Cost to everyone else.** A batch holds the write lock on every Vault it writes to, from before its first item until the call ends. Other writers to those Vaults wait for it: the Web UI, another agent, the HTTP API. The lock makes the hash chaining safe and keeps the batch's writes in one Git commit, at the cost of a pause other writers notice when a 20-write batch hits a busy Vault. Reads are unaffected. Hatchdoor takes the locks before the first item, always in the same order, so two batches that name the same Vaults in opposite orders cannot block each other forever. If one of those Vaults is busy, with a long Git sync for example, the whole batch waits for it before running anything, read items included.

Changing a Vault's settings while a batch is writing to it does not interrupt the batch: the lock survives the change, and the remaining items keep it. A Vault that is disabled or disconnected part-way through is different. Its remaining items are refused, never written without the lock, and their errors say so. The items already written stand, as in any other partial batch.

Vault changes made by a batch are committed together on the Vault's next Git sync turn, the same as any other burst of writes.

A batch is a single `tools/call`, so it costs one call against the per-token rate limit (`HATCHDOOR_MCP_RATE_LIMITS_ENABLED` in [[Settings and environment variables reference]]) however many reads and writes it carries, which is the main reason to use it. Searches are the exception, because they cost the most: each `search_notes` item costs one call, as it would standalone, and a batch holding any search waits for one of the two search slots as a standalone search does. A batch of 50 searches costs the same as 50 searches. The item limits above are separate from the rate limit.

---

Related: [[Connect your agent]] · [[How to deploy Hatchdoor with an agent]]
