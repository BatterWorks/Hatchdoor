---
tags: [type/explanation, topic/layers]
---

# The layer system

A Vault accumulates content of different relevance: primary notes next to meeting transcripts, reference dumps, archived research, or material an agent generated on request. All of it deserves to live in the same portable Markdown tree — but not all of it deserves equal weight in a search result or a first glance at the explorer.

Layers let a folder opt into a **secondary surface**: still fully part of the Vault, still linkable, still versioned, but excluded from the default view someone gets by browsing or searching without asking for it specifically.

> [!note]
> Layers are not access control. There is no permission boundary here — anyone who can read the Vault at all can reach a demoted note by asking for it explicitly. What layers change is *default visibility*, not *who* can see something.

## Marking a folder

A folder opts into a layer by containing a `.hatchdoor-layer` file. The file is plain YAML, in one of two forms:

```yaml
sources
```

```yaml
name: sources
description: Raw source material, kept for reference but not for browsing.
```

The name is normalized before it becomes a layer: Unicode-folded, lowercased, spaces become hyphens, and only letters, digits, and hyphens survive. `default`, `all`, `noise`, and `none` are reserved and cannot be claimed — they mean something else to the system (see below).

## Inheritance and re-promotion

A layer applies to its folder and everything beneath it, by longest matching path — a note three folders under a marker still belongs to that marker's layer unless a closer marker overrides it. A subfolder can opt back out with its own marker whose name is literally `default`:

```yaml
default
```

That is not a layer — it never appears as one, and a note under it always reports `layer: null` — it exists purely to re-promote a subtree that would otherwise inherit a demotion from a parent folder.

The Vault root itself can never carry a named layer marker: that would demote every note in the Vault and leave nothing on the default surface, so Hatchdoor refuses to build an index while one is present.

## What being on a layer actually changes

This is the detail worth getting precisely right, because the two changes it makes are easy to conflate:

1. **Default search excludes it.** Search without an explicit layer selection only ever considers the default surface (`layer IS NULL`). A demoted note simply never appears in an ordinary search result.
2. **Browsing does not.** On an ordinary, authenticated instance, the explorer tree, the graph, the recent-notes list, and reading a note directly by its slug all show **everything**, demoted or not. Layers narrow default *search*, not the operator's or the agent's ability to look around or fetch something they already know the slug of.

> [!warning]
> A public, read-only demo deployment (`HATCHDOOR_DEMO_MODE=true`) is the one exception: with no operator and no layer toggle to speak of, it narrows every surface — tree, graph, recent, exact fetch, search — to the default layer only. An ordinary deployment never does this.

An agent (or a person, via the Web UI's **Keyword mode** or an explicit layer filter) that deliberately asks for a layer by name, or for `all`, gets those notes back like any other.

## Semantic search and `HATCHDOOR_EMBED_LAYERS`

Being on the default surface always earns a note a vector embedding, so semantic search reaches it. A demoted note's embedding is optional and instance-wide: `HATCHDOOR_EMBED_LAYERS` (default on) decides whether demoted notes get embedded too.

- **On** — demoted notes are found by meaning, the same as default-surface notes, once something explicitly asks for that layer.
- **Off** — demoted notes are still fully indexed structurally (title, links, keyword search all work), just not embedded — saving indexing time and vector-storage space at the cost of semantic search over that layer. Exact-word search still finds them either way.

## Where a note's layer shows up

Every note and link read (`get_note`, search hits, `get_note_links`) and every write outcome (`create_note`, `update_note`, and friends) reports the resolved `layer` — `null` for the default surface, the layer name otherwise. There is currently no dedicated "list every layer in this Vault" call: an agent discovers layer names empirically, by searching with `layers: ["all"]` and reading the `layer` field off the results, or by looking at the marker files directly while browsing the Vault.

## What layers are not

- **Not the archive folder.** `archive_note`/`archive_folder` moves a note into a designated folder and is a completely separate mechanism from layers; an archived note's `layer` depends only on whether *that* folder happens to carry a `.hatchdoor-layer` marker, same as any other folder.
- **Not something an agent's write tools can set directly.** No write tool can create, rename, or move a file named `.hatchdoor-layer` — see [[How to organize a Vault with layers]] for what that means in practice.

---

Related: [[How to organize a Vault with layers]] · [[How to run an LLM wiki in Hatchdoor]] · [[The LLM wiki pattern (external reference)]] · [[MCP tools reference]] · [[HTTP API reference]]
