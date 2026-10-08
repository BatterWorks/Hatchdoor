---
tags: [type/reference, topic/agent-workflow, topic/layers]
---

# The LLM wiki pattern (external reference)

Hatchdoor's layer system and its [[How to run an LLM wiki in Hatchdoor|LLM wiki guide]] were built for a pattern that Andrej Karpathy described. This page sets out that pattern in his words and links to the primary sources, for anyone who wants the thinking behind the guide as well as its steps.

> [!note]
> "LLM Wiki" is a **workflow pattern Karpathy described**. He shipped no software for it. There is no official repository, package or release, only a tweet and a follow-up write-up. Everything below comes from those two documents.

## Primary sources

| Source | Date | Link |
| --- | --- | --- |
| Announcement tweet, "LLM Knowledge Bases" | 2026-04-02 | [x.com/karpathy/status/2039805659525644595](https://x.com/karpathy/status/2039805659525644595) |
| Follow-up idea document (`llm-wiki.md`) | 2026-04-04 | [gist.github.com/karpathy/442a6bf555914893e9891c11519de94f](https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f) |
| Karpathy's GitHub, for context. No `llmwiki` repository exists there. | | [github.com/karpathy](https://github.com/karpathy) |

## The core idea

The gist puts it this way:

> "Most people's experience with LLMs and documents looks like RAG: you upload a collection of files, the LLM retrieves relevant chunks at query time, and generates an answer. This works, but the LLM is rediscovering knowledge from scratch on every question. There's no accumulation... This is the key difference: the wiki is a persistent, compounding artifact."

So the agent does not work an answer out from raw documents on every question. It writes and maintains a Markdown wiki, a little at a time. The wiki lasts, its pages link to each other, and a person can read it in any Markdown viewer.

## Three layers

| Layer | Role |
| --- | --- |
| **Raw sources** | A directory of source material that never changes: articles, papers, transcripts, images. The agent reads these and never edits them. |
| **The wiki** | Markdown pages the agent writes and owns: summaries, entity pages, concept pages. Two files are special: an index, which catalogs the content and is updated on every ingest, and a chronological log. |
| **The schema** | A configuration document in the style of `AGENTS.md` that describes the wiki's conventions. The human and the agent revise it together over time. |

## Three operations

| Operation | What happens |
| --- | --- |
| **Ingest** | A new source lands in raw sources. The agent reads it, writes a summary page, updates the index, and updates every existing page the new material touches. Karpathy notes "a single source might touch 10-15 wiki pages." |
| **Query** | Someone asks the wiki a question. The agent reads the index, opens the relevant pages and answers from what the wiki already contains. It researches further only when the wiki does not cover the question, and then writes what it learned back in. |
| **Lint** | From time to time the agent checks the wiki's health: contradictions, stale claims, orphan pages, missing cross-references. |

## Tooling named in the source material

- Obsidian, as the viewer for the human, with its Web Clipper extension for turning web articles into Markdown.
- Marp, for slide decks generated from wiki content.
- matplotlib, for charts.
- Dataview, for queries over frontmatter.
- [`qmd`](https://github.com/tobi/qmd), once a wiki outgrows a plain index file. It is a third-party local search engine that combines BM25 and vector search, with a CLI and an MCP server. Karpathy calls it "a good option."

The pattern names no LLM. Karpathy describes it as usable with "your own LLM Agent, e.g. OpenAI Codex, Claude Code, OpenCode / Pi, or etc.," which means any capable agent that works on a local filesystem.

## Scale

In the April 2026 tweet Karpathy put one of his own research wikis at "~100 articles and ~400K words". He gave no other size figure.

## Community implementations

The gist calls the pattern "intentionally abstract... not a specific implementation," so building the tooling is left to whoever adopts it. Several third parties have built implementations since the gist came out. Hatchdoor's layer system and MCP tools are one, built independently of the others.

---

Related: [[How to run an LLM wiki in Hatchdoor]] · [[The layer system]] · [[MCP tools reference]]
