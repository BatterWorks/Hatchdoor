---
tags: [type/reference, topic/agent-workflow, topic/layers]
---

# The LLM wiki pattern (external reference)

Hatchdoor's layer system and its [[How to run an LLM wiki in Hatchdoor|LLM-wiki workflow guide]] were built for a specific external pattern, not invented independently. This page is a dictionary of that pattern itself — what it is, in Andrej Karpathy's own words, with links to the primary sources — for anyone who wants the model behind the guide, not just the steps.

> [!note]
> "LLM Wiki" is a **workflow pattern Karpathy described**, not software he shipped. There is no official repository, package, or release — only a tweet and a follow-up write-up. Everything below is sourced from those two documents.

## Primary sources

| Source | Date | Link |
| --- | --- | --- |
| Announcement tweet, "LLM Knowledge Bases" | 2026-04-02 | [x.com/karpathy/status/2039805659525644595](https://x.com/karpathy/status/2039805659525644595) |
| Follow-up idea document (`llm-wiki.md`) | 2026-04-04 | [gist.github.com/karpathy/442a6bf555914893e9891c11519de94f](https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f) |
| Karpathy's GitHub (for context — no `llmwiki` repo exists there) | — | [github.com/karpathy](https://github.com/karpathy) |

## The core idea

> "Most people's experience with LLMs and documents looks like RAG: you upload a collection of files, the LLM retrieves relevant chunks at query time, and generates an answer. This works, but the LLM is rediscovering knowledge from scratch on every question. There's no accumulation... This is the key difference: the wiki is a persistent, compounding artifact." — the gist

Rather than re-deriving an answer from raw documents on every question, an LLM agent incrementally writes and maintains a Markdown wiki: durable, interlinked, and readable by a human in a normal Markdown viewer.

## Three layers

| Layer | Role |
| --- | --- |
| **Raw sources** | An immutable directory of source material — articles, papers, transcripts, images. The agent reads these but never edits them. |
| **The wiki** | Markdown pages (summaries, entity pages, concept pages) the agent writes and owns entirely, plus two special files: an index (a content catalog, updated on every ingest) and a chronological log. |
| **The schema** | An `AGENTS.md`-style configuration document describing the wiki's own conventions, co-evolved by the human and the agent over time. |

## Three operations

| Operation | What happens |
| --- | --- |
| **Ingest** | A new source lands in raw sources. The agent reads it, writes a summary page, updates the index, and updates every existing page the new material touches — Karpathy notes "a single source might touch 10-15 wiki pages." |
| **Query** | A question is asked against the wiki. The agent reads the index, drills into relevant pages, and answers from what the wiki already contains — researching further only when the wiki doesn't cover it, then feeding what it learns back in. |
| **Lint** | Periodic agent-driven health checks: contradictions, stale claims, orphan pages, missing cross-references. |

## Tooling named in the source material

Obsidian, as the human-facing viewer (plus its Web Clipper extension for turning web articles into Markdown); Marp for slide decks generated from wiki content; matplotlib for charts; Dataview for frontmatter-driven queries; and, once a wiki outgrows a plain index file, [`qmd`](https://github.com/tobi/qmd) — a third-party local hybrid BM25/vector search engine with a CLI and an MCP server, which Karpathy calls "a good option."

The pattern names no specific LLM — it's described as usable with "your own LLM Agent, e.g. OpenAI Codex, Claude Code, OpenCode / Pi, or etc.," i.e. any sufficiently capable agentic model working against a local filesystem.

## Scale

Karpathy reported one of his own research wikis at "~100 articles and ~400K words" as of the April 2026 tweet — the only concrete size figure he gave.

## What's already community, not official

Several third parties have built concrete implementations of the pattern since the gist was published — the gist itself calls the pattern "intentionally abstract... not a specific implementation," so building the actual tooling was always left to whoever adopts it. Hatchdoor's layer system and MCP tools are one such implementation, built independently of any of those other projects.

---

Related: [[How to run an LLM wiki in Hatchdoor]] · [[The layer system]] · [[MCP tools reference]]
