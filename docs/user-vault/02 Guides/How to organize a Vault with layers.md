---
tags: [type/how-to, topic/layers]
---

# How to organize a Vault with layers

A layer keeps a folder out of ordinary search without moving or hiding it. Use one for material you want to keep but not see in every search: raw transcripts, clipped articles, old research. The notes stay in the Vault, in the sidebar and in links, and an agent can still search them when it asks for that layer. To read about the idea before the steps, see [[The layer system]].

## 1. Add the marker

Create a file named `.hatchdoor-layer` directly inside the folder you want to demote, with your file manager, a text editor or Git. The name starts with a dot, so some file managers hide the file once it exists. **Neither the browser nor an agent can create, rename, or move this file**: every write refuses a target named `.hatchdoor-layer`, so an agent with ordinary write access cannot plant or move a marker that reclassifies a whole folder.

The simplest marker is the layer name alone:

```yaml
sources
```

Add a description if it will help an agent decide when to ask for this layer:

```yaml
name: sources
description: Raw source material, kept for reference but not for browsing.
```

A name holds letters, digits and hyphens only, starts with a letter or digit, and is 32 characters or fewer. `default`, `all`, `noise` and `none` are reserved.

> [!warning]
> Don't put a named-layer marker at the Vault root. It would demote the entire Vault and leave nothing on the default surface, and Hatchdoor refuses to build the index while it is there.

## 2. Let it index

There is nothing to run. The file watcher that notices note edits picks up the marker file too. Give it a moment, then check with `get_stats` (the MCP tool, or `GET /api/v1/vaults/{scope}/stats` over HTTP), or read a note in the folder and look at its `layer` field.

## 3. Confirm the demotion took effect

Search for something that exists only under the marked folder, without selecting a layer. It should **not** appear:

```text
search_notes with scope=<vault_id>, query="<something only in that folder>"
```

Now ask for it, by naming the layer or by asking for everything:

```json
{ "name": "search_notes", "arguments": { "scope": "<vault_id>", "query": "...", "layers": ["sources"] } }
```

```json
{ "name": "search_notes", "arguments": { "scope": "<vault_id>", "query": "...", "layers": ["all"] } }
```

Browsing does not change. On an ordinary deployment, one that is not a public demo, the explorer tree, the graph and `get_note` by slug all still show the note. Only default search changed.

> [!note]
> No tool lists every layer. To find the layer names a Vault already has, search with `layers: ["all"]` and read the `layer` field of the hits, or look at the marker files while browsing.

## 4. Re-promote a subfolder if needed

A folder inside a demoted one can go back to the default surface with a marker of its own:

```yaml
default
```

This does not create a layer, and a note under it always reports `layer: null`. It only cancels the demotion the folder inherited from its parent.

## 5. Decide whether demoted content should be semantically searchable

By default (`HATCHDOOR_EMBED_LAYERS=true`), a note found through a layer request can be found by meaning, like a note on the default surface. If the Vault holds a lot of demoted content and you would trade search by meaning there for a smaller index that builds faster, turn it off in **Settings → Meaning search in demoted layers**, or set `HATCHDOOR_EMBED_LAYERS` through `PATCH /api/settings`. Exact-word search over demoted notes works either way.

> [!warning]
> Changing `HATCHDOOR_EMBED_LAYERS` starts a background reindex, which takes longer the larger the Vault is. The change is complete when the reindex finishes.

## 6. Retiring a layer

Delete the `.hatchdoor-layer` file, or edit it to `default`, to stop demoting a folder. Its notes go back to the default surface on the next index turn. Until a note has been reindexed it keeps its old layer and stays reachable with `layers: ["all"]`.

---

Related: [[The layer system]] · [[MCP tools reference]] · [[Search and change notes with your agent]]
