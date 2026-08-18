---
tags: [type/how-to, topic/layers]
---

# How to organize a Vault with layers

Use this when a folder's content should stay in the Vault and stay reachable on request, but should stop showing up in default search and stop competing for attention. See [[The layer system]] first if you want the concepts before the steps.

## 1. Add the marker

Create a `.hatchdoor-layer` file directly inside the folder you want to demote — through your usual filesystem/Git access, or the Web UI's file tools if it exposes raw-file creation. **No MCP or HTTP write tool can create, rename, or move this file**: every write path refuses a target named `.hatchdoor-layer` outright, specifically so a marker that reclassifies a whole folder can't be planted or moved by an agent's ordinary write access.

The simplest marker is just the layer name:

```yaml
sources
```

Add a description if it will help an agent decide when to ask for this layer:

```yaml
name: sources
description: Raw source material, kept for reference but not for browsing.
```

Naming rules: letters, digits, and hyphens only, starting with a letter or digit, 32 characters or fewer. `default`, `all`, `noise`, and `none` are reserved.

> [!warning]
> Don't put a named-layer marker at the Vault root — it would demote the entire Vault and leave nothing on the default surface. Hatchdoor refuses to build the index while that's true.

## 2. Let it index

No manual step needed: the marker file is picked up by the same file watcher that notices note edits, the same way as any other Vault change. Give it a moment, then confirm with `get_stats` (HTTP `GET /api/v1/vaults/{scope}/stats` or the MCP `get_stats` tool) or by checking the note's own `layer` field after reading it.

## 3. Confirm the demotion took effect

Search for something that only exists under the marked folder, without any layer selector — it should **not** appear:

```text
search_notes with scope=<vault_id>, query="<something only in that folder>"
```

Now ask for it explicitly, either by naming the layer or by asking for everything:

```json
{ "name": "search_notes", "arguments": { "scope": "<vault_id>", "query": "...", "layers": ["sources"] } }
```

```json
{ "name": "search_notes", "arguments": { "scope": "<vault_id>", "query": "...", "layers": ["all"] } }
```

Browsing is unaffected either way — the explorer tree, the graph, and `get_note` by slug all still show the note on an ordinary (non-demo) deployment. Only default search changed.

> [!note]
> There is no "list every layer" tool. If you need to discover what layer names already exist in a Vault, search with `layers: ["all"]` and read the `layer` field off the hits, or look at the marker files while browsing.

## 4. Re-promote a subfolder if needed

A folder nested inside a demoted one can opt back onto the default surface with its own marker:

```yaml
default
```

This does not create a layer — a note under it always reports `layer: null` — it just overrides the inherited demotion from its parent.

## 5. Decide whether demoted content should be semantically searchable

By default (`HATCHDOOR_EMBED_LAYERS=true`), a note found through an explicit layer request is searchable by meaning, same as the default surface. Turn it off in **Settings → Meaning search in demoted layers** (or `PATCH /api/settings` with `HATCHDOOR_MCP_ENABLED`'s neighbor, `HATCHDOOR_EMBED_LAYERS`) if the Vault has a lot of demoted content and you would rather trade semantic recall there for a smaller, faster index — exact-word search over demoted notes keeps working either way.

> [!warning]
> Flipping `HATCHDOOR_EMBED_LAYERS` triggers a background reindex (its settings class is `reindex`, not `instant`) — expect a delay proportional to Vault size before the change is fully applied.

## 6. Retiring a layer

Delete the `.hatchdoor-layer` file (or edit it to `default`) to stop demoting a folder — its notes rejoin the default surface on the next index turn. If you remove a marker while notes are still tagged with its old layer name for some other reason (e.g. only some of them reindexed yet), they stay on that layer rather than being silently promoted; they remain reachable via `layers: ["all"]` until they're fully reclassified.

---

Related: [[The layer system]] · [[MCP tools reference]] · [[Search and change notes with your agent]]


