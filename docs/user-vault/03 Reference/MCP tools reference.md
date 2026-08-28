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
> The full tool catalogue is always advertised, even before the model-setup completes and even at zero Vaults, so a client that caches tools at connection time never needs to reconnect. Before setup finishes, only `get_model_setup_status`, `accept_gemma_terms`, `decline_gemma_terms`, and the Vault collection discovery/management tools (`list_vaults` and friends) actually run; every other tool returns "Hatchdoor is still being set up." until a model is selected.

There is no selected, sole, or default Vault. Every tool below that touches content takes an explicit `vault_id`; every collection-level tool takes an explicit `scope` (a Vault ID or the literal `all`).

## Model setup

Always available, regardless of `HATCHDOOR_MCP_ENABLED`'s write posture — these are the only tools that run before first-run setup completes.

| Tool | Purpose |
| --- | --- |
| `get_model_setup_status` | Report setup state, the Gemma terms/policy links, and the Nomic fallback notice. No parameters. |
| `accept_gemma_terms` | Accept Gemma's terms, then download it and begin indexing. No parameters. |
| `decline_gemma_terms` | Decline Gemma, remove any partial Gemma download, download Nomic Embed Text v1.5 instead, and begin indexing. No parameters. |

> [!warning]
> Once a model is selected, calling either `accept_gemma_terms` or `decline_gemma_terms` again returns an error — changing models after setup is not supported.

## Vault collection: discovery and management

`list_vaults` is always available. The other six require `HATCHDOOR_MCP_WRITE_ENABLED`; without it they return the same "MCP write tools are disabled" error as content write tools.

| Tool | Gating | Purpose |
| --- | --- | --- |
| `list_vaults` | Always | Every Vault's ID, name, status, redacted source, and capabilities, plus the registry's `registry_revision`. Call this first — every write below needs a fresh `expected_registry_revision`. |
| `create_vault` | Write mode | Create a Vault definition. The registry assigns the Vault ID; read it back from `list_vaults`. |
| `edit_vault` | Write mode | Replace one Vault definition wholesale (not a patch — send back every field you want to keep). |
| `enable_vault` | Write mode | Enable a disabled Vault definition. |
| `disable_vault` | Write mode | Disable a Vault without deleting its files. |
| `disconnect_vault` | Write mode | Remove a Vault from the registry without deleting local files, checkouts, Git history, or credentials outside the registry record. |
| `sync_vault` | Write mode | Request immediate managed-Git synchronization for one eligible Vault. |
| `retry_vault` | Write mode | Retry an admitted managed-Git operation for one eligible Vault. |

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

Both take just `vault_id`. `sync_vault` requests an immediate poll for a managed-Git Vault instead of waiting for `poll_interval_secs`. `retry_vault` retries an operation the scheduler admitted but that failed (e.g. a transient network error), rather than waiting for its own backoff.

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
| `search_notes` | `scope`, `query` | Search one Vault or all enabled Vaults. Optional: `mode` (`semantic` default or `keyword`), `limit` (1–50, default 10), `per_note_cap` (1–10, default 2), `layers` (array of layer names to include). |
| `get_note` | `vault_id`, `slug` | Read one exact note's authoritative Markdown. |
| `get_note_links` | `vault_id`, `slug` | Outgoing links and backlinks for one exact note. |
| `resolve_wikilink` | `vault_id`, `target` | Resolve a wikilink target within one Vault. |
| `get_tree` | `scope` | Grouped explorer tree for one Vault or all enabled Vaults. |
| `get_stats` | `scope` | Grouped statistics for one Vault or all enabled Vaults. |
| `get_graph` | `scope` | Grouped link graph for one Vault or all enabled Vaults. |
| `get_frontmatter` | `vault_id`, `slug` | Read one exact note's frontmatter metadata — `tags`, `aliases`, and every remaining top-level key under `properties` — without returning the Markdown body. A note with no frontmatter block answers `has_frontmatter: false` with empty collections rather than an error. It returns no `content_hash`: to write the metadata back, take the hash from `get_note`. |
| `recently_modified` | `scope` | Recently modified notes. Optional `limit` (1–25, default 5). |
| `list_note_attachments` | `vault_id`, `slug` | List the attachments one note references, without the note's full content. |
| `get_attachment` | `vault_id`, `relative_path` | Fetch one attachment's bytes, addressed by the same `relative_path` `list_note_attachments` reports. Optional `encoding`: `url` (the default) returns a `download_url`, `base64` returns the bytes inline. |
| `get_attachment_import_config` | `vault_id` | Report whether uploads are currently possible for this Vault, the available methods, their byte limits, and the allowed file extensions. Call this before uploading. |

Collection-scoped results (`search_notes`, `get_tree`, `get_stats`, `get_graph`, `recently_modified` with `scope: "all"`) carry `scope`, `collection_revision`, `partial`, and `participants` — an agent should branch on the structured error `code`, never on message text, and should treat `partial: true` as "some enabled Vaults did not answer in time," not as an error.

> [!note]
> `get_attachment_import_config`'s `enabled` field is the AND of two independent gates: `HATCHDOOR_MCP_WRITE_ENABLED` (instance-wide) and the target Vault's own `capabilities.mutate` (source mode and lifecycle phase). The response explains which one is currently false when `enabled` is `false`.

`get_attachment` mirrors the inbound upload flow in the opposite direction, and its two encodings carry the same tradeoff. `encoding: "url"` returns `content.download_url` — a **relative** path to resolve against the same scheme, host, and port as the MCP endpoint itself — plus `path_note` and `auth` fields restating that. Send your MCP bearer token as an `Authorization: Bearer` header and it works: the asset route accepts that credential for as long as MCP is enabled, exactly as the upload endpoint accepts it on the way in. The web bearer token works too, as a header or an `access_token` query parameter, and with neither token configured (or in demo mode) the URL needs no credential at all. A client that cannot make an out-of-band HTTP request calls `get_attachment` again with `encoding: "base64"` and gets `content.content` inline instead — bounded by the same `HATCHDOOR_MCP_MAX_BASE64_BYTES` cap `import_attachment` uses on the way in, and rejected with the measured size when the file is over it. Either way the result carries `vault_id`, `relative_path`, `size_bytes`, and `content_type`.

> [!note]
> Fetching that URL with the MCP token is held to the same limits as fetching the bytes over `/mcp`: the same `HATCHDOOR_MCP_MAX_BASE64_BYTES` ceiling (a larger attachment returns `413`, so use the web token or raise the setting) and the same per-token rate quota, drawn from the same counter, answering `429` with `Retry-After`. The URL is a cheaper transport, not a larger allowance. Turning MCP off revokes it on the very next request, with no restart. See [[The security model]].

## Write content tools

Every tool below requires `HATCHDOOR_MCP_WRITE_ENABLED=true` and takes `vault_id` in addition to the parameters listed. Every mutating tool that targets an existing note also requires `expected_content_hash` — the hash most recently read from `get_note` — for optimistic concurrency: a stale hash means someone else changed the note since you read it, and the write is rejected rather than silently overwriting.

| Tool | Required parameters (beyond `vault_id`) | Purpose |
| --- | --- | --- |
| `create_note` | `relative_path`, `content` | Create a Markdown note. Parent folders are created automatically. Fails if the note exists unless `overwrite: true`. |
| `update_note` | `slug`, `content`, `expected_content_hash` | Replace a note's full content. |
| `append_to_note` | `slug`, `content`, `expected_content_hash` | Append content to a note. |
| `edit_note` | `slug`, `old_string`, `new_string`, `expected_content_hash` | Surgical string replacement. `old_string` must match exactly and be unique unless `replace_all: true`; otherwise the edit is rejected without writing. Prefer this over `update_note` for small changes. |
| `replace_section` | `slug`, `heading`, `mode`, `content`, `expected_content_hash` | Replace or insert around a Markdown section identified by its heading. `mode` is `replace` (overwrite the section — `content` should include the heading), `before`, or `after`. The section spans the heading through the next same-or-higher heading; headings inside fenced code blocks are ignored, and the heading must match exactly and be unique. |
| `update_frontmatter` | `slug`, `frontmatter`, `expected_content_hash` | Shallow top-level merge into the note's YAML frontmatter, leaving the body untouched. Keys you don't mention survive; an explicit `null` deletes a key; a nested mapping is replaced wholesale rather than merged into. A note with no frontmatter block gets one created, and deleting its last key removes the block rather than leaving an empty one. Rejects an empty `frontmatter` object, and a creation whose values are all `null`. |
| `rename_note` | `slug`, `new_title`, `expected_content_hash` | Rename within the current folder; rewrites wikilink backlinks and moves/rewrites referenced assets. |
| `move_note` | `slug`, `target_folder`, `expected_content_hash` | Move to a target folder; same backlink/asset handling as rename. |
| `move_rename_note` | `slug`, `target_relative_path`, `expected_content_hash` | Move and rename in one operation. |
| `archive_note` | `slug`, `expected_content_hash` | Move to the configured archive folder (the Vault's own `archive_folder`, set via `create_vault`/`edit_vault` above, or the instance default). |
| `delete_note` | `slug`, `expected_content_hash` | Trash a note under `.hatchdoor-trash`; removes backlinks to it and moves/rewrites its assets. |
| `import_attachment` | `content` (base64), `target_relative_path` | Upload an attachment by sending its bytes base64-encoded. This is the **fallback** for clients that cannot make an out-of-band HTTP request — size-limited (`HATCHDOOR_MCP_MAX_BASE64_BYTES`, default 5 MiB decoded). Prefer `POST /api/v1/vaults/{vault_id}/attachments` when possible; call `get_attachment_import_config` first to see current limits. |
| `move_attachment` | `source_relative_path`, `target_relative_path` | Move an attachment and rewrite every note reference to it. |
| `rename_attachment` | `source_relative_path`, `new_filename` | Rename an attachment in place and rewrite every note reference to it. |
| `delete_attachment` | `source_relative_path` | Trash an attachment under `.hatchdoor-trash` and rewrite every note reference to it. |

Every write tool accepts an optional `commit_summary` (a one-line string) used in the Git commit body for Vaults with versioning enabled.

> [!warning]
> No write tool can create, rename, or move a file named `.hatchdoor-layer` (the layer marker) — that call is rejected outright, since a marker silently changes how a whole folder is classified and is meant to be edited directly in the Vault. Writes are also rejected if the target path matches the Vault's own noise-exclusion patterns, since such a file would be written to disk but stay invisible to every read surface.

### Response shape

A successful note write returns `vault_id`, `slug`, `relative_path`, `content_hash` (use this for the next write), `layer`, `quality_warnings`, `rewritten_notes` (backlinks updated), `moved_assets`, and `trashed_path` (set only by `delete_note`). A successful attachment write returns `vault_id`, `attachment`, `rewritten_notes`, `trashed_path`, and `cleanup_warning`.

A write conflict (stale `expected_content_hash`, or a registry revision that moved under a Vault-management call) is reported as a retryable tool error — re-read the current state and retry rather than assuming the operation is unsafe to repeat.

## Batch

`batch` runs an ordered list of the tools above in a single call. There is one such tool, not a batching variant per tool: each item names an `op` and carries that tool's own `arguments` exactly as a standalone call would, `vault_id` included, so one batch can span several Vaults.

```json
{
  "operations": [
    {"op": "create_note", "arguments": {"vault_id": "<id>", "relative_path": "Inbox/Draft.md", "content": "# Draft\n"}},
    {"op": "update_frontmatter", "arguments": {"vault_id": "<id>", "slug": "inbox/draft", "frontmatter": {"tags": ["status/draft"]}, "expected_content_hash": "ignored-here"}},
    {"op": "get_note", "arguments": {"vault_id": "<id>", "slug": "inbox/draft"}}
  ]
}
```

**What may go in.** Every read tool except `list_vaults`, and every note and attachment write tool — `create_note` through `delete_attachment`, deletes included. Vault-management tools (`create_vault`, `edit_vault`, `enable_vault`, `disable_vault`, `disconnect_vault`, `sync_vault`, `retry_vault`) and the model-setup tools are not batchable, and neither is `batch` itself. An unknown or disallowed `op`, an empty `operations` array, more than **50** read-shaped items, or more than **20** write-shaped items rejects the whole call up front, before any item executes.

**Best-effort, in order, no rollback.** Items run one after another; an item that fails never stops the ones after it, and nothing already written is undone. There is no mid-batch visibility either — an item sees the Vault, not the batch's own bookkeeping, apart from the hash chaining below.

**Permission is still per item.** `batch` itself is not gated on write mode, because a batch may be entirely read-only. Each write-shaped item is gated exactly as the same call would be standalone: with `HATCHDOOR_MCP_WRITE_ENABLED=false` the read items succeed and the write items fail individually, and a Vault whose `capabilities.mutate` is false refuses its writes the same way.

**`expected_content_hash` inside a batch.** Once a batch has written a note, later items in that same call targeting the same `vault_id` + `slug` have their `expected_content_hash` replaced with the hash that write produced — you cannot know the intermediate hash without the round trip a batch exists to avoid, so supply any placeholder for it and it is discarded. This applies to `create_note` too: create a note and edit it later in the same call. A note the batch has *not* already written validates its `expected_content_hash` normally, exactly like a standalone call. The relaxation never leaks outside the call — Hatchdoor holds each touched Vault's mutation lock from that Vault's first write through the end of the batch, so no outside writer can slip in behind a substituted hash.

**Result shape.** `items` (one entry per requested operation, carrying `index`, `op`, `ok`, and then either `result` — that tool's own normal result — or `error`), plus `succeeded` and `failed` counts. A batch that ran at all returns success at the tool level; read `failed` and the per-item `ok` flags, never the call's own status, to find out what happened.

**Cost to everyone else.** From its first write to a Vault until the call ends, a batch holds that Vault's write lock. Other writers to the same Vault — the Web UI, another agent, the HTTP API — wait for it. That is what makes the hash chaining safe and what keeps the batch's writes in one Git commit, but it means a 20-write batch against a busy Vault is a pause other writers feel. Reads are unaffected.

Vault changes made by a batch are committed together on the Vault's next Git sync turn, the same as any other burst of writes. (On the legacy single-Vault sync path, a batch's writes may land in more than one commit; nothing is lost or reordered, only the commit boundary differs.)

A batch is a single `tools/call`, so it costs one call against the per-token rate limit (`HATCHDOOR_MCP_RATE_LIMITS_ENABLED` in [[Settings and environment variables reference]]) no matter how many items it carries — which is most of the reason to reach for it. The item caps above are the tool's own, enforced separately.

---

Related: [[Connect your agent]] · [[How to deploy Hatchdoor with an agent]]
