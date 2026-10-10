---
tags: [type/explanation, topic/layers]
---

# The layer system

A Vault collects content of uneven value: your main notes next to meeting transcripts, reference dumps, old research, and material an agent wrote on request. All of it belongs in the same folder of Markdown, but not all of it should carry the same weight in a search result.

A layer lets a folder step back onto a **secondary surface**. Its notes stay in the Vault, can still be linked and are still versioned, and they stay out of search unless someone asks for them.

> [!note]
> Layers are not access control. Anyone who can read the Vault can reach a demoted note by asking for it. A layer changes what shows up by default and never who can see it.

## Marking a folder

A folder joins a layer when it contains a `.hatchdoor-layer` file. The file is plain YAML, in one of two forms:

```yaml
sources
```

```yaml
name: sources
description: Raw source material, kept for reference but not for browsing.
```

Hatchdoor normalizes the name before it becomes a layer: it folds Unicode, lowercases, turns spaces into hyphens, and keeps only letters, digits and hyphens. `default`, `all`, `noise` and `none` are reserved, because Hatchdoor uses them for something else (see below).

## Inheritance and re-promotion

A layer applies to its folder and everything beneath it, and the closest marker wins. A note three folders under a marker belongs to that marker's layer unless a closer marker says otherwise. A subfolder can opt back out with a marker whose name is `default`:

```yaml
default
```

That is not a layer. It never appears as one, and a note under it always reports `layer: null`. Its only job is to put a subfolder back on the default surface when a parent folder demoted it.

The Vault root can never carry a named layer marker. It would demote every note in the Vault and leave nothing on the default surface, so Hatchdoor refuses to build an index while one is present.

## What being on a layer actually changes

A layer changes one thing and leaves another alone, and the two are easy to mix up:

1. **Default search excludes it.** A search that selects no layer looks only at the default surface (`layer IS NULL`). A demoted note never appears in an ordinary search result.
2. **Browsing does not.** On an ordinary instance where you sign in, the explorer tree, the graph, the recent notes list and reading a note by its slug all show every note, demoted or not. You and your agent can still look around and open any note whose slug you know.

> [!warning]
> A public, read-only demo (`HATCHDOOR_DEMO_MODE=true`) is the one exception. It has no operator and no way to choose a layer, so it narrows the tree, the graph, the recent list, exact reads and search to the default layer. An ordinary deployment never does this. The same goes for images and attachments: a demo refuses any file stored under a demoted folder, even when a note on the default surface embeds it, so that embed shows as broken on the demo.

An agent that asks for a layer by name, or for `all`, gets those notes back like any other. So does a call to the HTTP search route with its `layers` parameter. Search in the browser has no layer choice: in both modes it looks at the default surface only, so open a demoted note there from the sidebar, a link or the graph.

## Semantic search and `HATCHDOOR_EMBED_LAYERS`

A note on the default surface always gets an embedding, so semantic search reaches it. For demoted notes the embedding is optional, and one setting decides for the whole instance: `HATCHDOOR_EMBED_LAYERS`, on by default.

- **On**: demoted notes are found by meaning, like notes on the default surface, once something asks for their layer.
- **Off**: demoted notes are still indexed for titles, links and keyword search, and get no embedding. That saves indexing time and disk space, and semantic search no longer covers that layer. Exact-word search finds the notes either way.

## Where a note's layer shows up

Every note and link read (`get_note`, search hits, `get_note_links`) and every write result (`create_note`, `update_note` and the other write tools) reports the note's `layer`: `null` for the default surface, the layer name otherwise. No call lists every layer in a Vault. An agent finds the layer names by searching with `layers: ["all"]` and reading the `layer` field of the results, or by looking at the marker files while browsing the Vault.

## What layers are not

- **Not the archive folder.** `archive_note` moves a note into the Vault's archive folder, which has nothing to do with layers. An archived note's `layer` depends only on whether that folder carries a `.hatchdoor-layer` marker, as with any other folder.
- **Not something an agent's write tools can set directly.** No write tool can create, rename or move a file named `.hatchdoor-layer`. [[How to organize a Vault with layers]] says what that means in practice.

---

Related: [[How to organize a Vault with layers]] · [[How to run an LLM wiki in Hatchdoor]] · [[The LLM wiki pattern (external reference)]] · [[MCP tools reference]] · [[HTTP API reference]]
