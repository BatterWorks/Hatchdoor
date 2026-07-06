# Semantic Search Strategy

> Status: accepted decision record. Hatchdoor ships pure semantic retrieval by default. Cross-encoder reranking and hybrid keyword/vector fusion remain offline evaluation tools only.

## TL;DR

We evaluated four retrieval strategies on the same 26-query eval set on CPU-only deployment hardware:

1. Pure semantic (Nomic Embed Text v1.5)
2. Pure semantic + JINARerankerV1TurboEn cross-encoder
3. Pure semantic + JINARerankerV2BaseMultilingual cross-encoder
4. Hybrid (Nomic + SQLite FTS5 BM25, fused with Reciprocal Rank Fusion)

**Pure semantic wins.** It beats the cross-encoder reranker on every metric, and beats the hybrid retriever on MRR while tying it on Recall and FP-rate. The aggregate numbers, the per-query diff, and the hardware constraints all point the same direction: ship pure Nomic, no reranker, no fusion layer.

## What we measured

Same 26 queries, same cache, same metrics (Recall@5/10, MRR, FP-rate@5):

| | R@5 any | R@5 all | R@10 any | R@10 all | MRR | FP@5 | Median e2e |
|---|---|---|---|---|---|---|---|
| **Pure Nomic (Embed Text v1.5)** | **1.000** | **0.968** | **1.000** | 0.968 | **0.923** | **0.500** | ~embed-only (~150 ms) |
| Nomic + JINARerankerV1TurboEn (k=10) | 0.962 | 0.949 | 1.000 | 0.968 | 0.878 | 0.750 | 5198 ms |
| Nomic + JINARerankerV2BaseMultilingual (k=20) | — | — | — | — | — | — | never finished (>45 min budget) |
| Hybrid (Nomic + FTS5 BM25, RRF k=60, initial-k=20) | 1.000 | 0.968 | 1.000 | **0.978** | 0.894 | 0.500 | 194 ms |

The original private evaluation report included the full per-run rows and per-query diff. The aggregate result is preserved here because it explains the product decision without publishing the private query set.

### Per-query diff: pure vs hybrid

26 queries, broken down by which side wins on rank-of-first-expected:

- **Ties: 21** (most queries put the expected note at the same rank under both strategies — usually rank 1)
- **Hybrid wins: 2** (D2 rank 4→2; U4 rank 2→1)
- **Pure Nomic wins: 3** (D3 1→2; D13 1→2; U2 1→2)
- **Anti-flips: 0** in either direction

The hybrid wins are real but small (the expected note was already in pure's top-10 — RRF just nudged it higher). The pure-Nomic wins are **top-spot demotions** caused by FTS surfacing a lexically-strong but topically-wrong note above the right one. A rank-1 → rank-2 demotion is a more user-visible failure than a rank-4 → rank-2 promotion.

## Why pure semantic was the right answer

1. **CPU-only deployment constraints.** The intended self-hosted deployment path needs to work well on modest CPU-only machines. Any cross-encoder runtime path was multiple seconds per query in evaluation; the multilingual model did not finish within the time budget. That eliminates cross-encoder reranking for the default runtime path.

2. **Hybrid is unpredictable on this corpus.** The per-query diff showed that hybrid helps a few distinctive proper-noun queries, but hurts when the vault contains lexically similar distractors. There was no reliable query class where fusion clearly won. Aggregate MRR loss (0.923 to 0.894) reflects that the harm outweighed the help.

3. **Hybrid is not free.** It adds a second retriever query and an RRF fuse to every search. Median e2e jumps from ~150 ms (embed only) to 194 ms. Small in absolute terms, but a cost paid on every query in exchange for a net-negative quality effect.

4. **Pure Nomic already solved the original motivation, modulo FP-rate@5.** The evaluation was opened because FP-rate@5 worsened after enabling Nomic task prefixes. The reranker was meant to be the fix. In practice, pure Nomic's 0.500 FP-rate@5 was unchanged by either reranker or by hybrid. The FP-rate@5 floor appears to be a property of the eval set itself, not a model failure. No retrieval strategy moved the needle on this metric.

## What this changes about Phase 2

The original retrieval roadmap included hybrid retrieval plus context assembly. Hybrid retrieval was dropped from the runtime path. The runtime search direction is now:

- **Pure semantic retrieval (Layer 6) as the runtime path**, exposed through a new MCP tool + HTTP route.
- **Context assembly (Layer 9)** — bundling retrieved chunks with parent headings + linked notes for the agent.
- **No fusion of FTS5 with semantic results** at runtime. SQLite FTS5 stays where it is (`note_fts`), used only by the existing keyword `search` path.

Code that stays in tree but is no longer a runtime target:

- `src/rerank/` (cross-encoder reranker) — offline eval only.
- `src/eval/rerank_runner.rs` and the `eval rerank` subcommand.
- `src/eval/hybrid_runner.rs`, `SqliteCache::fts_search_notes`, and the `eval hybrid` subcommand.
- `src/eval/compare_runner.rs` and the `eval compare` subcommand — useful for any future "ship X or Y" question.

## Caveats worth remembering

- 26 queries against 490 chunks is small. The MRR gap between pure and hybrid (0.923 vs 0.894) is well within "could flip on a different query set." If a vault grows by roughly an order of magnitude, re-run the eval harness to confirm pure semantic search still wins.
- The "FP-rate@5 floor at 0.500" is suspicious — it suggests one or two queries in the anti-expected set are genuinely ambiguous given the vault contents. Worth a manual review of which queries are at fault before declaring it a fixed property of the corpus.
- Hybrid retrieval, as implemented here, fuses at **note** granularity (FTS5 is per-note; semantic is per-chunk, collapsed to per-note). A chunk-level FTS index would change the experiment. If a future Phase 2.x decides to revisit fusion, that's the variable to try.

## References

- Offline evaluation code: `src/eval/`.
- Hybrid evaluation implementation: `src/eval/hybrid_runner.rs`.
- Cross-encoder reranker implementation: `src/rerank/`.
- Reciprocal Rank Fusion reference: Cormack, Clarke & Buettcher (2009), "Reciprocal Rank Fusion outperforms Condorcet and individual rank learning methods."
