# Andrej Karpathy's "LLM Wiki" pattern

> Research record.
>
> Researched: 2026-08-18
> Scope: What Karpathy's "LLM Wiki" is, how it works, when it was published,
> its current status, and its relationship to his other public projects
> Status: Findings from primary sources only (his own tweet and gist);
> secondary/SEO write-ups were used only to locate those sources, never cited
> as evidence

## Executive answer

"LLM Wiki" is not a piece of software Karpathy shipped — it is a **workflow
pattern** he described in a tweet and then wrote up as a standalone idea
document (a GitHub Gist). The pattern: instead of doing RAG over a pile of
raw documents, you have an LLM agent **incrementally compile a persistent,
interlinked Markdown wiki** from your sources, keep it updated as you add
more sources, and use it (with Obsidian as a viewer/"IDE") as the thing you
actually query. Karpathy calls this "manipulating knowledge" the way he
otherwise manipulates code, and pitches it as an alternative to classic
RAG/NotebookLM-style retrieval, where "nothing accumulates."

Primary sources confirm:

- **Announcement tweet**: [@karpathy, April 2, 2026](https://x.com/karpathy/status/2039805659525644595)
  (verified via the `fxtwitter.com` mirror after `x.com` itself returned an
  HTTP 402 to automated fetch — mirror content matches the tweet id, author,
  and timestamp fields exactly, so treated as primary).
- **Follow-up idea doc / Gist**: [gist.github.com/karpathy/442a6bf555914893e9891c11519de94f](https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f),
  titled `llm-wiki.md`, created **April 4, 2026** (confirmed via the raw gist
  HTML's `datetime="2026-04-04T16:25:13Z"` timestamp and by fetching the raw
  gist content directly with `curl`, bypassing any AI-summarization step).

There is **no GitHub repository** named `llmwiki`, `llm-wiki`, or similar
under [github.com/karpathy](https://github.com/karpathy) — his repositories
are things like `nanochat`, `nanoGPT`, `llm.c`, `LLM101n`, `minbpe`,
`micrograd`, etc. (confirmed by fetching his repositories page directly).
"LLM Wiki" is a written pattern/idea file, explicitly described in the gist
itself as "intentionally abstract... not a specific implementation" — the
actual code (wiki directory structure, ingest/query/lint tooling) is meant to
be built by whoever adopts the pattern, in collaboration with their own LLM
agent. Numerous third parties have since built concrete implementations
(`karpathy-llm-wiki`, `llmwiki`, `llm-wiki-compiler`, `llm_wiki`, Obsidian
plugins, etc.), but **none of those are Karpathy's own code** — they are
community projects inspired by his gist.

## What it actually is (item 1 of the brief)

It is a **workflow/pattern description**, not:
- not a wiki generated once by an LLM and left static,
- not a wiki *about* LLMs as a subject,
- not a dataset,
- not a course/curriculum artifact (it is unrelated to `LLM101n` beyond both
  being Karpathy publications).

It is closest to "a documented methodology for using an LLM agent as a
continuously-operating knowledge-base maintainer," with Obsidian as the
human-facing viewer.

## Stated purpose/motivation, in Karpathy's own words

From the [announcement tweet](https://x.com/karpathy/status/2039805659525644595)
(April 2, 2026):

> "Something I'm finding very useful recently: using LLMs to build personal
> knowledge bases for various topics of research interest. In this way, a
> large fraction of my recent token throughput is going less into
> manipulating code, and more into manipulating knowledge (stored as
> markdown and images)."

And, describing scale he had personally reached:

> "...once your wiki is big enough (e.g. mine on some recent research is
> ~100 articles and ~400K words), you can ask your LLM agent all kinds of
> complex questions against the wiki, and it will go off, research the
> answers, etc. I thought I had to reach for fancy RAG, but the LLM has been
> pretty good about auto-maintaining index files..."

The gist restates and extends the motivation with a more explicit critique
of retrieval-only tooling:

> "Most people's experience with LLMs and documents looks like RAG: you
> upload a collection of files, the LLM retrieves relevant chunks at query
> time, and generates an answer. This works, but the LLM is rediscovering
> knowledge from scratch on every question. There's no accumulation... This
> is the key difference: the wiki is a persistent, compounding artifact."

and closes on the maintenance-burden argument for why this wasn't already
common practice:

> "The tedious part of maintaining a knowledge base is not the reading or
> the thinking — it's the bookkeeping... Humans abandon wikis because the
> maintenance burden grows faster than the value. LLMs don't get bored,
> don't forget to update a cross-reference, and can touch 15 files in one
> pass."

He also frames it historically, tying it to Vannevar Bush's Memex (the gist,
["Why this works" section](https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f)):

> "The idea is related in spirit to Vannevar Bush's Memex (1945)... The part
> he couldn't solve was who does the maintenance. The LLM handles that."

## How it technically works (item 3)

Per both the tweet and the gist, the pipeline has three layers and three
operations.

**Layers** (per the gist):
1. **Raw sources** — an immutable `raw/` directory of source documents
   (articles, papers, repos, datasets, images). The LLM reads but never
   modifies these.
2. **The wiki** — a directory of Markdown files (summaries, entity pages,
   concept pages, an index) that the LLM generates and owns entirely. Two
   files play a special role: `index.md` (a content catalog, updated on
   every ingest) and `log.md` (an append-only chronological record, with a
   suggested consistent line prefix like `## [2026-04-02] ingest | Article
   Title` so it's `grep`-able).
3. **The schema** — a `CLAUDE.md`/`AGENTS.md`-style configuration document
   telling the agent the wiki's conventions and workflows; co-evolved by the
   human and the LLM over time.

**Operations**:
- **Ingest** — drop a new source in `raw/`, have the agent read it, write a
  summary page, update the index, update affected entity/concept pages
  (Karpathy notes "a single source might touch 10-15 wiki pages"), and log
  the event.
- **Query** — ask questions against the wiki; the agent reads the index,
  drills into relevant pages, and synthesizes a cited answer, optionally
  filing the answer itself back into the wiki as a new page so explorations
  compound too.
- **Lint** — periodic LLM-driven health checks for contradictions, stale
  claims, orphan pages, missing cross-references, and gaps (which can be
  backfilled via web search).

**Tooling mentioned by name** (from the tweet and gist): Obsidian as the
viewing "IDE" (plus its Web Clipper extension for converting web articles to
Markdown, and a hotkey workflow for pulling images down locally), Marp for
generating slide decks from wiki content, matplotlib for chart output,
Dataview for frontmatter-driven queries, and — once a wiki outgrows a plain
index file — [`qmd`](https://github.com/tobi/qmd), a third-party local
hybrid BM25/vector search engine with both a CLI and an MCP server, which
Karpathy names as "a good option." He also mentions personally having "vibe
coded a small and naive search engine over the wiki."

**Model(s) used**: Karpathy does not name a specific model in either the
tweet or the gist — the pattern is described model-agnostically ("your own
LLM Agent, e.g. OpenAI Codex, Claude Code, OpenCode / Pi, or etc."), i.e. it
is a way of using any sufficiently capable coding/agentic LLM against a local
filesystem, not a wrapper around one particular model.

**Automated vs. human-curated**: The human is responsible for sourcing
(deciding what goes into `raw/`), directing analysis, and asking questions.
The LLM is responsible for essentially all of the wiki's *content* — writing
and maintaining every page, cross-references, and the index/log. Karpathy is
explicit about this division: "You rarely ever write or edit the wiki
manually, it's the domain of the LLM."

## When announced/released (item 4)

- **April 2, 2026** — initial tweet, [@karpathy status
  2039805659525644595](https://x.com/karpathy/status/2039805659525644595),
  titled "LLM Knowledge Bases" (confirmed `created_at: "Thu Apr 02 20:42:21
  +0000 2026"` in the tweet's own metadata).
- **April 4, 2026** — the standalone `llm-wiki.md` gist, a more polished,
  reusable write-up of the same idea (confirmed by the gist's own
  `datetime` attribute).

## Current status (item 5)

- **Karpathy's own artifacts**: the tweet and the gist are the only two
  primary-source documents; there is no dedicated Karpathy repository, no
  released code, and no dataset. The gist itself says the pattern is
  "intentionally abstract... not a specific implementation," so by design
  there is nothing further to "release" from Karpathy directly.
- **Engagement**: the gist page and tweet do show large engagement counts,
  but this report does not cite specific star/fork/like numbers as settled
  fact, because independent checks were inconsistent — two separate WebFetch
  passes over the same gist page returned conflicting figures ("5,000+
  stars/forks" in one pass vs. "46,082 stars, 9,486 forks, 1,061 comments"
  in another), and a direct `curl` of the rendered gist HTML could not
  extract a reliable numeric star/fork count (GitHub renders those counters
  client-side). Third-party tweets (not treated as authoritative, only as
  color) mention figures like "5,000 stars in 48 hours," but that number is
  unverified against a primary source and should not be repeated as fact.
  What is independently verifiable: the tweet's own engagement fields at
  fetch time, via the `fxtwitter.com` mirror of the live tweet, showed
  21,787,028 views, 60,793 likes, 7,383 retweets, 2,924 replies, and 2,192
  quotes — these numbers came from the tweet object itself, not a
  secondary summary, but will have grown further since this fetch.
- **Third-party implementations** (community projects, not Karpathy's own,
  found via GitHub search and explicitly excluded from being treated as
  primary evidence about Karpathy's project itself): `Astro-Han/karpathy-llm-wiki`,
  `lucasastorian/llmwiki`, `nashsu/llm_wiki`, `atomicstrata/llm-wiki-compiler`,
  `Ss1024sS/LLM-wiki`, `balukosuri/llm-wiki-karpathy`, and an Obsidian
  community plugin ("Karpathy LLM Wiki"). These confirm the pattern was
  picked up and re-implemented widely, but say nothing authoritative about
  Karpathy's own usage or plans.
- **Scale Karpathy personally reported**: "~100 articles and ~400K words"
  for one of his own research wikis, as of the April 2, 2026 tweet — this is
  the only concrete size figure attributable to Karpathy himself; no later
  update from him on this figure was found.

## Relationship to Karpathy's other recent work (item 6)

- **nanochat / llm.c**: no direct technical connection found. Both are
  training/inference codebases; "LLM Wiki" is purely a knowledge-management
  workflow. The only link is thematic — his tweet frames LLM Wiki as a shift
  in where his own "token throughput" goes ("less into manipulating code,
  and more into manipulating knowledge"), implicitly contrasting it with the
  code-heavy nature of projects like nanochat/llm.c, but he does not say the
  wiki was used to build either of those repos.
- **Eureka Labs / LLM101n**: no explicit link in either primary source. Both
  are separate Karpathy initiatives (Eureka Labs announced July 2024 per
  search results, not verified first-hand in this pass since it was outside
  this task's date window); nothing in the tweet or gist mentions Eureka
  Labs or LLM101n.
- **"Software 2.0"/"Software 3.0"**: no explicit link found in either
  primary source. Secondary sources loosely associate the LLM Wiki pattern
  with his "Software 3.0" framing (LLMs as a new way of directing computers,
  in English rather than code), but Karpathy does not use that terminology
  or cite that essay in the tweet or the gist. Any such connection is
  interpretive, not something he stated.

## Note on secondary-source noise

A first-pass web search turned up a large number of SEO/content-mill
articles (`datasciencedojo.com`, `medium.com`, `aibuilderclub.com`,
`kunalganglani.com`, `starmorph.com`, `theaioperator.io`, `hjarni.com`,
`agentpedia.codes`, etc.) that describe "Karpathy's LLM Wiki" in ways that
sometimes disagree with each other on hard numbers (e.g. wildly different
star counts, some describing it as a shipped "tool" rather than a pattern).
None of these were used as a cited source for any claim above; they were
used only to locate the real primary URLs (the tweet ID and the gist ID),
which were then independently verified via direct `curl` fetches of the raw
gist content and the `fxtwitter.com` mirror of the live tweet object.
