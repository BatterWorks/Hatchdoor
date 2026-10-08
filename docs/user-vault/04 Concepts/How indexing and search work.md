---
tags: [type/explanation, topic/search]
---

# How indexing and search work

Your Markdown files are the real copy of your notes. To search them quickly, Hatchdoor reads them into one SQLite file, the index, and everything it searches, lists and links comes from there. This page explains what the index is, how it keeps up with your notes, and how the two search modes differ.

## The index is disposable, on purpose

The index is built from the Vault's Markdown and holds nothing that is not in your files. Delete it and Hatchdoor rebuilds it from the files on disk, with nothing lost. That is why [[Understand where your data lives]] says to back up the Vault folder and not the cache folder.

## What happens when a note changes

A file watcher notices every file created, edited or deleted under the Vault: Markdown, attachments and `.hatchdoor-layer` markers. A marker counts because changing one moves notes between layers without touching their content. Hatchdoor waits about half a second after a change, so a burst of saves becomes one reindex pass. It never waits more than five seconds, so a long editing session cannot hold the index back.

A change made outside Hatchdoor, in Obsidian or any other program, normally shows within seconds. Some folders send Hatchdoor no change events at all, and a Windows folder used through Docker Desktop is the common case. So once a minute Hatchdoor also compares each Vault's folder with what it saw the time before, by file names, sizes and modification times, and reindexes when something was added, changed or removed. On those folders an outside change shows within about a minute. There is nothing to turn on and no setting for it.

A save made through Hatchdoor, from the browser or an agent, does not wait for the watcher. The save asks for the reindex itself and marks the Vault's search `stale` until it lands, so the change is picked up even where the watcher sees nothing. If the operating system reports that it dropped file events, the next pass rescans the whole Vault.

> [!note]
> Reindexing covers only what changed. Each note carries a content hash, and Hatchdoor reuses unchanged notes and unchanged chunks without embedding them again. A full rebuild happens only when something makes the whole cache invalid at once, such as switching embedding models or changing `HATCHDOOR_EMBED_LAYERS`. Every vector in the cache has to come from the same model, and a partial rebuild would leave vectors that cannot be compared with each other.
>
> An index pass cut short by a restart, a crash or a shutdown does not start over either. Hatchdoor saves the chunks it has embedded as it goes, and the next pass embeds only what is left, so the progress percentage picks up where the last one stopped. Saved chunks never answer a search. A Vault becomes searchable only once every chunk has its vector, and until then a Vault that was searchable before keeps answering from its previous index, marked `stale`.

## Vaults take turns indexing

Vaults index one at a time, in the order they asked, because the embedding model runs one job at a time. A large Vault's first index can take hours on a small server, and a Vault added after it should not have to wait for all of it. So a Vault that has been embedding for about five minutes checks whether another Vault is waiting. If one is, it stops at the next chunk, keeps everything it has embedded, and goes to the back of the line. The waiting Vault runs, and the large one carries on from where it stopped when its turn comes round again. If nothing is waiting it never stops, so an instance with one Vault pays nothing for this.

Nothing jumps the queue. A paused Vault goes behind every Vault already waiting, and one that asks after the pause goes behind it. The cost is that the large Vault finishes later while others are waiting, mostly because it reads its notes again each time it resumes. The five minutes is fixed, and no setting changes it.

While a Vault waits, its `index_turn` is `waiting` and the sidebar shows **waiting** where it would otherwise show it indexing; see [[Vault lifecycle states]]. Its search keeps answering from whatever it already had. The time left on the **All Vaults** row counts only embedding, so it does not tick down while the Vault waits.

## Two search modes, not one fused "hybrid" search

Hatchdoor has two search modes, **Semantic** (the default) and **Keyword**, and you choose between them. It never blends the two into one ranking.

- **Semantic** embeds your query with the model that embedded your notes, then finds the closest vectors. It matches by meaning, so a query can find a note that never uses the query's words.
- **Keyword** runs against SQLite's FTS5 full-text index, the BM25-style engine most databases use for exact-text search. It matches the words that are present, which is what you want for a hostname, filename, tag, ID, or any other exact string.

> [!warning]
> Hatchdoor never fuses the two modes at query time. It was tested: combining them with Reciprocal Rank Fusion added a second query and measurable latency, and the results were unpredictable and often worse. On the evaluation set, fusion sometimes pushed the correct top result below a distractor that shared its words. Semantic search alone scored best. If you see "hybrid" used loosely elsewhere, it does not mean fused retrieval in Hatchdoor's search.

Cross-encoder reranking was tested too and dropped. On the CPU-only hardware Hatchdoor targets it improved nothing, and it cost seconds per query where a search takes milliseconds.

## Where layers fit in

A note's [[The layer system|layer]] decides whether Semantic search reaches it by default. The default surface is always embedded, and a demoted layer gets vectors only when `HATCHDOOR_EMBED_LAYERS` is on. Keyword search covers every layer either way, because the full-text index includes them all at no extra cost. With the setting off, a Semantic search that includes a demoted layer does not report that layer as empty. That Vault is reported `not_searchable` and the result is marked partial, so an agent or the search dialog knows to try Keyword for those notes.

---

Related: [[Understand where your data lives]] · [[Search and change notes with your agent]] · [[The layer system]]
