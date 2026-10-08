---
tags: [type/how-to, topic/agent-workflow, topic/layers]
---

# How to run an LLM wiki in Hatchdoor

An **LLM wiki** is a pattern Andrej Karpathy described. With per-query RAG, an agent researches the same ground again on every question. With an LLM wiki, it builds and maintains a lasting Markdown wiki whose pages link to each other, and the wiki gets more useful with every source it takes in. Hatchdoor's Vaults, wikilinks and layers fit the pattern closely. This guide sets one up.

The pattern has three operations: **ingest** raw material, **query** the wiki, and **lint** it for drift. Hatchdoor's tools cover the first two. The third is partly manual today (see the end of this page).

## 1. Shape the Vault into two zones

Keep raw material and the curated wiki in the same Vault, separated by a [[The layer system|layer]]. A `raw` layer, or one with a similar name, holds source dumps such as transcripts, pasted articles and scraped pages. The Vault's **default surface** holds the curated wiki pages.

Follow [[How to organize a Vault with layers]] to create the marker:

```yaml
name: raw
description: Unprocessed source material an agent ingests from, not the curated wiki.
```

Raw material then stays in the Vault, where it is versioned and can be linked, and stays out of default search.

## 2. Connect an agent with write access

Follow [[Connect your agent]] and [[Search and change notes with your agent]] to get a connection that reads first and then writes. An LLM wiki needs write access, so turn on **Let assistants change notes** once you trust the read path.

## 3. Ingest

Give the agent raw material and tell it to file it under the `raw` layer first, before it writes anything to the curated wiki:

```text
Use Hatchdoor MCP. Create a note under the "raw" layer folder containing this material verbatim: [paste source text]. Use a short, dated filename. Do not touch any other note yet.
```

Then have it turn that into a curated wiki page, or fold it into one. Tell it to extend an existing page before it creates a new one for the same topic.

```text
Search the wiki (default surface) for an existing page about [topic]. If one exists, read it and use append_to_note or replace_section to add what's new from [[raw source note]], with a wikilink back to the source. If none exists, create a new wiki page, link it from any obviously related pages you already found, and link it back to the raw source.
```

One source often produces several pages, or one new page plus edits to the pages that should link to it. The whole set can go in one `batch` call, with no round trip per page. A batch can also create a page and edit it again later in the same call, with no read in between. A batch does not roll back, and one item can fail while the others succeed, so tell the agent to report each item's outcome. See [[MCP tools reference#Batch]].

> [!tip]
> Small, well-linked pages beat one giant page. Much of a wiki's value is in its links. An agent that queries it later can follow `[[wikilinks]]` between related pages as well as read search hits.

## 4. Query

Before it answers a question, an agent should search the wiki. Its own memory and a new web search come second:

```text
Before answering, search the Hatchdoor wiki for [topic]. Read the most relevant page(s) and any pages they link to. Answer using what the wiki already says; only research further if the wiki doesn't cover it, and if you do, ingest what you learn back into the wiki afterward.
```

Query first, then ingest what was missing. The next query is then faster and does not repeat the research.

## 5. Lint

Hatchdoor has no dedicated wiki-checking tool yet. Until it does, check by hand:

- Run `search_notes` with `layers: ["all"]` now and then, to check nothing landed on the wrong surface.
- Use `recently_modified` and [[Browse and review through the Web UI]] to spot-check what the agent has been writing.
- Read metadata without the bodies. `get_frontmatter` returns one page's tags, aliases and other properties, and a `batch` of them across a set of pages checks a whole zone for tag drift in one call. Each answer carries that page's content hash too, so the fix needs no second read: `update_frontmatter` takes the hash and corrects one key at a time without rewriting the page.
- Use `find_text` after a rename or a correction to check that the old wording is gone. It returns every note that still contains the exact string, in every layer, with a count.
- Ask the agent directly: *"Search the wiki for pages with no incoming or outgoing wikilinks. Those are candidates for linking in or archiving."*

---

Related: [[The LLM wiki pattern (external reference)]] · [[The layer system]] · [[How to organize a Vault with layers]] · [[Search and change notes with your agent]] · [[MCP tools reference]]
