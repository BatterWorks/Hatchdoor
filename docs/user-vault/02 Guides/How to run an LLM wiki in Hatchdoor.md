---
tags: [type/how-to, topic/agent-workflow, topic/layers]
---

# How to run an LLM wiki in Hatchdoor

An **LLM wiki** is a pattern popularized by Andrej Karpathy: instead of an agent re-researching the same ground on every question (per-query RAG), it incrementally builds and maintains a persistent, interlinked Markdown wiki — a compounding artifact that gets more useful over time, not a cache that gets thrown away after each answer. Hatchdoor's Vault model, wikilinks, and layer system fit this almost exactly. This guide sets one up.

The pattern has three operations: **ingest** raw material, **query** the wiki, and **lint** it for drift. The first two map cleanly onto Hatchdoor's existing tools; the third is partly manual today — see the note at the end.

## 1. Shape the Vault into two zones

Keep raw material and the curated wiki in the same Vault, separated by a [[The layer system|layer]]: a `raw` (or similarly named) layer for source dumps — transcripts, pasted articles, scraped pages — and the Vault's **default surface** for the curated wiki pages themselves.

Follow [[How to organize a Vault with layers]] to create the marker:

```yaml
name: raw
description: Unprocessed source material an agent ingests from, not the curated wiki.
```

This keeps raw material fully in the Vault, versioned and linkable, without it cluttering default search or the first thing a person sees browsing.

## 2. Connect an agent with write access

Follow [[Connect your agent]] and [[Search and change notes with your agent]] to get a read-then-write connection working. An LLM wiki needs write access — turn on **Let assistants change notes** once you trust the read path.

## 3. Ingest

Give the agent raw material and a clear instruction to file it under the `raw` layer first, before writing anything to the curated wiki:

```text
Use Hatchdoor MCP. Create a note under the "raw" layer folder containing this material verbatim: [paste source text]. Use a short, dated filename. Do not touch any other note yet.
```

Then have it turn that into (or fold it into) a curated wiki page — this is where the "compounding artifact" idea matters: prefer extending an existing page over creating a new one for the same topic.

```text
Search the wiki (default surface) for an existing page about [topic]. If one exists, read it and use append_to_note or replace_section to add what's new from [[raw source note]], with a wikilink back to the source. If none exists, create a new wiki page, link it from any obviously related pages you already found, and link it back to the raw source.
```

A single source often produces several pages, or one new page plus edits to the pages that should link to it. That whole set can go in one `batch` call rather than a round trip each — including creating a page and then editing it again later in the same call, which needs no intermediate read. Batches are best-effort and have no rollback, so tell the agent to report each item's own outcome rather than assuming the set landed whole; see [[MCP tools reference#Batch]].

> [!tip]
> Small, well-linked pages beat one giant page. The wiki's value is in the link graph as much as the content — an agent that queries it later benefits from being able to follow `[[wikilinks]]` between related pages, not just semantic search hits.

## 4. Query

Before answering a question, an agent should search the wiki, not just its own memory or a fresh web search:

```text
Before answering, search the Hatchdoor wiki for [topic]. Read the most relevant page(s) and any pages they link to. Answer using what the wiki already says; only research further if the wiki doesn't cover it — and if you do, ingest what you learn back into the wiki afterward.
```

This closes the loop: query first, ingest what's missing, so the next query is faster and doesn't repeat the same research.

## 5. Lint

Hatchdoor has no dedicated wiki-integrity tool yet — it's a known gap, not yet implemented. Until then, approximate it manually:

- Periodically `search_notes` with `layers: ["all"]` to check nothing landed on the wrong surface.
- Use `recently_modified` and [[Browse and review through the Web UI]] to spot-check what the agent has been writing.
- Pull metadata without the bodies: `get_frontmatter` returns one page's tags, aliases, and other properties on its own, and a `batch` of them across a set of pages is cheap enough to check a whole zone for tag drift in one call. `update_frontmatter` fixes what it finds, one key at a time, without rewriting the page.
- Ask the agent directly: *"Search the wiki for pages with no incoming or outgoing wikilinks — those are candidates for linking in or archiving."* This is a manual substitute for the automated check that doesn't exist yet.

---

Related: [[The LLM wiki pattern (external reference)]] · [[The layer system]] · [[How to organize a Vault with layers]] · [[Search and change notes with your agent]] · [[MCP tools reference]]
