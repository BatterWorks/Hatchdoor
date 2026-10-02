---
tags: [type/explanation, topic/search]
---

# How indexing and search work

Markdown is the source of truth. Everything Hatchdoor searches, browses, and links comes from one SQLite file built from that Markdown — the index. This page explains what that file actually is, how it stays current, and how the two search modes differ, since both are easy to get wrong by assumption.

## The index is disposable, on purpose

The SQLite cache is a rebuildable projection of the Vault's Markdown, never a second copy of your data. Delete it and Hatchdoor rebuilds it from the files on disk — nothing is lost, because nothing important lived there in the first place. This is why [[Understand where your data lives]] tells you the cache path doesn't need backing up while the Vault path does: one is derived, one is original.

## What happens when a note changes

A file watcher notices any create, edit, or delete under the Vault — Markdown, attachments, and `.hatchdoor-layer` markers alike, since a marker change reclassifies notes without touching their content. Changes are debounced: a burst of saves within about half a second coalesces into one reindex pass, with a five-second ceiling so a genuinely busy editing session can't defer freshness forever.

A save made through Hatchdoor itself, from the browser or an agent, doesn't wait for the watcher: the save asks for the reindex directly and marks the Vault's search `stale` until it lands, so it's picked up even where the watcher can't see changes. If the operating system reports that it dropped file events, the next pass rescans the whole Vault rather than trusting the gap.

> [!note]
> Reindexing is incremental, not a full rebuild. Each note carries a content hash; unchanged notes and unchanged chunks are reused as-is rather than re-embedded. A full rebuild only happens when something invalidates the whole cache at once — switching embedding models, or flipping `HATCHDOOR_EMBED_LAYERS` — because every vector in the cache has to share one embedding space, and a partial rebuild would leave some vectors comparable and others not.
>
> An index pass that gets cut short, by a restart, a crash or a shutdown, does not start over either. The chunks it embedded are saved to the cache as it goes, and the next pass embeds only what is left, so its progress percentage picks up where the last one stopped. Saved chunks never answer a search: a Vault becomes searchable only once every chunk has its vector, and until then one that was searchable before keeps answering from its previous index, marked `stale`.

## Vaults take turns indexing

Vaults index one at a time, in the order they asked, because the embedding model runs one job at a time anyway. A large Vault's first index can take hours on a small server, and a Vault added after it used to wait for all of it. Now a Vault that has been embedding for about five minutes checks whether another Vault is waiting. If one is, it stops at the next chunk, keeps everything it has embedded (the saving described above), and goes to the back of the line. The waiting Vault runs, and the large one carries on from where it stopped when its turn comes round again. If nothing is waiting it never stops, so an instance with one Vault pays nothing for this.

Nothing jumps the queue: a paused Vault goes behind every Vault already waiting, and one that asks after the pause goes behind it. The cost is that the large Vault finishes later while others are waiting, mostly because it reads its notes again each time it resumes. The five minutes is fixed, not a setting.

While a Vault waits, its `index_turn` is `waiting` and the sidebar shows **waiting** where it would otherwise show it indexing; see [[Vault lifecycle states]]. Its search keeps answering from whatever it already had, and the time left on the **All Vaults** row counts only embedding, so it does not tick down while the Vault waits.

## Two search modes, not one fused "hybrid" search

Hatchdoor offers **Semantic** (the default) and **Keyword** as two separate, user-selectable modes — not a combined ranking. This is a deliberate decision, not a missing feature.

- **Semantic** embeds your query with the same model used to embed notes, then finds the closest vectors — it matches by meaning, so a query can find a note that never uses the query's exact words.
- **Keyword** runs against SQLite's FTS5 full-text index (the same BM25-style engine most databases use for exact-text search) — it matches by the words actually present, which wins for a hostname, filename, tag, ID, or anything else where the precise string is what matters.

> [!warning]
> These two modes are never fused at query time. Hatchdoor evaluated combining them with Reciprocal Rank Fusion and found it added a second query and real latency for an unpredictable, often *worse* result: on the evaluation set, fusion occasionally demoted the correct top result in favor of a lexically-similar distractor. Pure semantic search won outright — see [ADR-05](https://github.com/BattermanZ/Hatchdoor/blob/main/docs/adr/semantic-search-strategy.md) for the full measurement if you want the numbers. If you see the word "hybrid" used loosely elsewhere, it does not mean fused retrieval in Hatchdoor's runtime search path.

Cross-encoder reranking was evaluated too, and dropped for the same reason from the other direction: it improved nothing on the same CPU-only hardware Hatchdoor targets, while costing seconds per query instead of milliseconds.

## Where layers fit in

A note's [[The layer system|layer]] decides whether Semantic search reaches it by default: the default surface is always embedded, but a demoted layer only gets vectors if `HATCHDOOR_EMBED_LAYERS` is on. Keyword search doesn't care — FTS5 indexes every layer regardless, since exact-text matching costs nothing extra to keep available. With it off, a Semantic search that includes a demoted layer doesn't pretend that layer came back empty: that Vault is reported `not_searchable` and the result is marked partial, so an agent or the search dialog knows to try Keyword for those notes.

---

Related: [[Understand where your data lives]] · [[Search and change notes with your agent]] · [[The layer system]]
