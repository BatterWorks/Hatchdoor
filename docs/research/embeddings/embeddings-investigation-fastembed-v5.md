# Hatchdoor Embedding & Model Investigation

> Research record.
>
> Updated for **FastEmbed v5** and current model options as of **24 July 2026**.

## Executive recommendation

Hatchdoor's embedding architecture is structurally sound, but its main retrieval-quality bottleneck is probably **not Nomic Embed Text v1.5 alone**. The larger opportunities are:

1. embedding note context such as titles, headings, aliases and tags;
2. reducing the default chunk size from 800 tokens;
3. adding production reranking;
4. making model selection and vector dimensions configurable;
5. batching chunks efficiently;
6. measuring changes against a representative multilingual evaluation set.

Now that FastEmbed v5 can be used on every supported platform, the strongest practical upgrade to test first is:

> **EmbeddingGemma 300M Q4**, initially at 768 dimensions and then at 256 dimensions through Matryoshka truncation and re-normalisation.

The most important alternatives are:

- **Nomic Embed Text v2 MoE** for a permissively licensed multilingual successor to Nomic v1.5;
- **Qwen3 Embedding 0.6B** as a higher-compute multilingual quality ceiling;
- **BGE-M3** as a separate dense + sparse + ColBERT retrieval experiment;
- **Granite Embedding English R2** as a lightweight English-specific custom-ONNX candidate;
- **Snowflake Arctic Embed M v2.0** as a CPU-oriented multilingual custom-ONNX candidate.

Do not select a winner from public leaderboard scores alone. Benchmark these models on Hatchdoor's actual note structure, languages and query patterns.

---

## Current Hatchdoor architecture

Hatchdoor currently:

- loads **Nomic Embed Text v1.5** in production;
- stores 768-dimensional vectors in SQLite through `sqlite-vec`;
- correctly applies Nomic's asymmetric prefixes:
  - `search_document:`
  - `search_query:`
- performs Markdown-aware, tokenizer-aware chunking;
- defaults to approximately **800 tokens** with **50-token overlap**;
- reuses embeddings through content hashes;
- records embedder identity and rebuilds the vector cache when that identity changes;
- has an `Embedder` abstraction that already separates:
  - embedding generation,
  - tokenizer access,
  - dimensions,
  - model identity,
  - query/document formatting.

This is a strong foundation for model experimentation.

---

# Retrieval-pipeline findings

## 1. Model selection is too hard-coded

The production model and dimensions should come from a single model specification rather than being repeated in startup and schema code.

Recommended shape:

```rust
pub struct EmbeddingModelSpec {
    pub id: &'static str,
    pub dimensions: usize,
    pub max_length: usize,
    pub query_format: QueryFormat,
    pub document_format: DocumentFormat,
    pub backend: EmbeddingBackend,
}
```

The specification should own:

- model identifier;
- backend type: ONNX, Candle or BGE-M3;
- vector dimensions;
- supported Matryoshka dimensions;
- tokenizer;
- maximum sequence length;
- query and document instructions;
- pooling and normalisation behaviour;
- cache identity.

The vector schema should derive its dimensions from this specification.

---

## 2. The 800-token default is probably too large

Large chunks improve broad recall but often weaken precision because one vector represents several concepts.

Benchmark at least:

| Chunk target | Overlap | Expected role |
|---:|---:|---|
| 300 tokens | 40 | High precision |
| 450 tokens | 50 | Likely best balance |
| 800 tokens | 50 | Current baseline |

A likely production range is **350–500 tokens**, especially after adding document context.

---

## 3. Embed contextual metadata

Current input is effectively:

```text
search_document: <chunk>
```

Recommended conceptual input:

```text
Title: <note title>
Aliases: <aliases>
Tags: <selected tags>
Section: <heading path>

<chunk text>
```

Model-specific wrappers should then be applied around this canonical document representation.

For EmbeddingGemma, for example:

```text
title: <note title> | text: Section: <heading path>

<chunk text>
```

Avoid embedding every metadata field indiscriminately. Prefer stable, retrieval-relevant context:

- note title;
- heading path;
- aliases;
- a small number of tags;
- optionally the note type or collection.

---

## 4. Batch by approximate sequence length

Embedding one chunk per inference call avoids padding but creates unnecessary runtime overhead.

Use length buckets, for example:

- 0–128 tokens;
- 129–256;
- 257–512;
- 513–1024;
- longer only for models and use cases that justify it.

Batching should be configurable because the best batch size differs significantly between an N100 and an M1 Pro.

---

## 5. Improve filtered semantic search

Filtered semantic search should avoid scanning all stored vectors in Rust as the vault grows.

Possible approaches:

- retrieve a larger vector candidate pool, then filter;
- pre-filter IDs through FTS or metadata and join through a temporary table;
- maintain partitions for high-value filters;
- combine FTS and dense candidate sets before reranking.

---

## 6. Use the existing evaluation harness as a release gate

The repository already has the beginnings of a useful evaluation framework. Turn it into a reproducible model-selection suite with committed aggregate results.

Measure:

- Recall@5 and Recall@10;
- MRR;
- nDCG@10;
- correct-note rate;
- correct-heading rate;
- cross-language retrieval;
- index time;
- incremental-index latency;
- single-query latency;
- peak memory;
- model download size;
- vector database size.

Run each model with both:

1. raw chunk text;
2. contextualised chunk text.

Otherwise model weaknesses may be confused with weak document representation.

---

# FastEmbed v5 model shortlist

FastEmbed v5 provides ordinary ONNX text embeddings, dedicated Candle implementations for **Qwen3** and **Nomic v2 MoE**, a dedicated **BGE-M3** interface, sparse embeddings and reranking.

## Summary

| Priority | Model | Languages | Dimensions | Context | FastEmbed v5 route | CPU assessment |
|---:|---|---|---:|---:|---|---|
| 1 | EmbeddingGemma 300M Q4 | 100+ | 768; MRL 512/256/128 | 2,048 | Native ONNX model | Best first practical test |
| 2 | Nomic Embed Text v2 MoE | ~100 | 768; MRL 256 | 512 | `nomic-v2-moe` Candle feature | Strong but benchmark memory and latency |
| 3 | Qwen3 Embedding 0.6B | 100+ | Up to 1,024; MRL | 32K | `qwen3` Candle feature | Quality ceiling; relatively heavy |
| 4 | BGE-M3 | 100+ | 1,024 dense plus sparse/ColBERT | 8,192 | Dedicated `Bgem3Embedding` | Heavy; valuable only as a retrieval-system experiment |
| 5 | Granite English R2 | English | 768 | 8,192 | User-defined/custom ONNX | Excellent English CPU candidate |
| 6 | Arctic Embed M v2.0 | 74 | 768; MRL 256 | 8,192 | User-defined/custom ONNX | Very interesting multilingual CPU candidate |

MRL means Matryoshka Representation Learning: vectors can be truncated to selected lower dimensions and then normalised again.

---

# Recommended multilingual models

## 1. EmbeddingGemma 300M Q4 — first upgrade to implement

### Why it is compelling

EmbeddingGemma combines:

- approximately 300M parameters;
- multilingual training covering more than 100 languages;
- a 2,048-token input window;
- 768-dimensional output;
- Matryoshka dimensions including 512, 256 and 128;
- dedicated query/document formatting;
- a native quantised FastEmbed v5 model;
- a good size-to-quality trade-off for local inference.

FastEmbed v5 exposes variants including:

- `EmbeddingGemma300M`;
- `EmbeddingGemma300MQ`;
- `EmbeddingGemma300MQ4`.

### Recommended Hatchdoor experiment

Benchmark:

1. Q4 at 768 dimensions;
2. Q4 truncated to 256 dimensions and re-normalised;
3. contextual title/section embeddings;
4. chunk targets of 300, 450 and 800 tokens.

A 256-dimensional index would use roughly one third of the raw vector storage and memory bandwidth of the current 768-dimensional index.

### Important caveat

EmbeddingGemma uses the **Gemma terms**, not Apache 2.0. Review the terms before making it Hatchdoor's default distributed model.

### Verdict

**Best first practical FastEmbed v5 candidate.**

---

## 2. Nomic Embed Text v2 MoE — best Nomic-family upgrade

### Why it belongs in the benchmark

Nomic v2 offers:

- multilingual coverage of roughly 100 languages;
- 768-dimensional vectors;
- Matryoshka reduction to 256 dimensions;
- Apache 2.0 licensing;
- familiar asymmetric prefixes:
  - `search_query:`
  - `search_document:`
- a direct conceptual migration from Nomic v1.5;
- a FastEmbed v5 Candle implementation behind `nomic-v2-moe`.

It is a mixture-of-experts model intended to provide stronger multilingual retrieval than similarly sized dense encoders.

### Main constraints

- Its effective input limit is 512 tokens.
- The model download and memory footprint are not especially small.
- Candle performance may differ substantially from the ONNX path.
- Sparse routing does not guarantee that it will be faster on an N100.

### Recommended Hatchdoor experiment

Benchmark:

- 768 dimensions;
- 256 dimensions;
- 300- and 450-token chunks;
- indexing and single-query latency separately;
- N100 and M1 Pro memory peaks.

### Verdict

**Best permissively licensed multilingual successor to Nomic v1.5.**

---

## 3. Qwen3 Embedding 0.6B — multilingual quality ceiling

### Strengths

Qwen3 Embedding 0.6B provides:

- more than 100 languages;
- up to 32K context;
- up to 1,024-dimensional output;
- selectable lower dimensions;
- instruction-aware query embeddings;
- Apache 2.0 licensing;
- a dedicated FastEmbed v5 Candle implementation behind `qwen3`.

A Hatchdoor query instruction could be:

```text
Given a query against a personal Markdown knowledge base, retrieve passages
that directly answer the query, including equivalent passages written in
another language.
```

### Constraints

It is a decoder-derived model with approximately 600M parameters. Expect:

- higher indexing latency;
- higher query latency;
- greater memory use;
- less attractive performance on an N100 than compact encoder models.

The long context is not a reason to embed entire notes. Retrieval still benefits from focused chunks.

### Recommended Hatchdoor experiment

Use it as a ceiling rather than the expected default:

- 512 dimensions;
- optionally 1,024 dimensions;
- 300- and 450-token chunks;
- tailored query instruction;
- M1 Pro first, N100 second.

### Verdict

**Best quality-ceiling experiment among the direct FastEmbed v5 options, but unlikely to be the low-power default.**

---

## 4. BGE-M3 — test the retrieval architecture, not just its dense vector

BGE-M3 produces:

- a 1,024-dimensional dense vector;
- learned sparse token weights;
- ColBERT-style multi-vectors;
- multilingual representations;
- support for long inputs.

FastEmbed v5 provides a dedicated `Bgem3Embedding` interface for these outputs.

### Why it is different

Testing only its dense vector misses the main reason to use it. A meaningful experiment would compare:

```text
Current:
FTS + dense retrieval + reciprocal-rank fusion

Alternative:
BGE-M3 dense + learned sparse + optional ColBERT late interaction
```

### Constraints

- substantially heavier model;
- larger dense vectors;
- additional storage if sparse and multi-vector outputs are persisted;
- more complicated indexing and ranking;
- potentially poor N100 economics.

### Verdict

**Use only as a separate hybrid-retrieval research branch, not as the first model swap.**

---

# English-specific candidates

A multilingual default is preferable for a vault containing English, French, Dutch and Spanish. English-only models are still useful for measuring how much multilingual support costs.

## 1. Granite Embedding English R2 — preferred custom English candidate

Granite English R2 offers:

- 149M parameters;
- 768 dimensions;
- 8,192-token context;
- Apache 2.0 licensing;
- a ModernBERT-derived encoder;
- strong retrieval, long-document and code-retrieval positioning.

The small R2 variant offers:

- 47M parameters;
- 384 dimensions;
- 8,192-token context.

These are not currently first-class FastEmbed model enum entries, so integration would use a compatible ONNX export and the user-defined model interface.

### Verdict

- **Granite English R2:** best balanced English custom candidate.
- **Granite Small English R2:** excellent speed and memory floor.

---

## 2. Existing FastEmbed English baselines

Keep these inexpensive native comparisons:

### GTE Base English v1.5 Quantized

- 768 dimensions;
- native FastEmbed integration;
- realistic English challenger;
- lower integration risk than a custom model.

### BGE Small English v1.5 Quantized

- 384 dimensions;
- very lightweight;
- useful latency and vector-size baseline;
- likely weaker than newer models in raw semantic quality.

### MixedBread Large v1 Quantized

- 1,024 dimensions;
- stronger-quality English baseline;
- relatively expensive on low-power CPUs;
- useful as an ONNX quality ceiling.

---

# Additional multilingual custom candidate

## Snowflake Arctic Embed M v2.0

Arctic Embed M v2.0 is worth testing after the direct v5 candidates because it offers:

- approximately 305M total parameters, with relatively low transformer compute for its class;
- 768-dimensional vectors;
- a useful 256-dimensional Matryoshka option;
- 8,192-token context;
- 74 languages;
- Apache 2.0 licensing;
- published ONNX files.

It is not currently a first-class FastEmbed model enum entry, so it would require a user-defined model or a small upstream contribution.

### Verdict

**Potentially the most interesting custom multilingual CPU model, but integration should follow the native v5 benchmark.**

---

# Models not prioritised

## Jina Embeddings v3

Technically strong, multilingual and long-context, but:

- approximately 600M parameters;
- non-commercial CC BY-NC licensing;
- less attractive for a generally distributed open-source product.

## Very large Qwen3 variants

The 4B and 8B embedding models may raise benchmark scores, but they do not fit Hatchdoor's low-power, self-hosted deployment goal.

## Multilingual E5 Large

Still a useful historical baseline, but newer models offer more compelling quality/efficiency trade-offs.

## ModernBERT Embed Large

Available through FastEmbed, but large and 1,024-dimensional. Granite R2 is a more interesting English efficiency direction, while Qwen3 provides a more useful quality ceiling.

---

# Exact benchmark suite

## Core suite

| Test | Model | Dimensions | Purpose |
|---:|---|---:|---|
| 1 | Nomic Embed Text v1.5 | 768 | Current control |
| 2 | Nomic Embed Text v1.5 Quantized | 768 | Quantisation control |
| 3 | EmbeddingGemma 300M Q4 | 768 | Main practical challenger |
| 4 | EmbeddingGemma 300M Q4 | 256 | Main storage-efficient challenger |
| 5 | Nomic Embed Text v2 MoE | 768 | Nomic-family multilingual upgrade |
| 6 | Nomic Embed Text v2 MoE | 256 | Compact Apache-licensed option |
| 7 | Qwen3 Embedding 0.6B | 512 | Quality ceiling |
| 8 | GTE Base English v1.5 Quantized | 768 | English native baseline |

## Separate architecture experiment

| Test | Model | Outputs |
|---:|---|---|
| 9 | BGE-M3 | Dense only |
| 10 | BGE-M3 | Dense + sparse |
| 11 | BGE-M3 | Dense + sparse + ColBERT |

Do not mix BGE-M3's full multi-representation experiment into the first simple dense-model comparison.

## Optional custom-model suite

| Test | Model | Dimensions |
|---:|---|---:|
| 12 | Granite Small English R2 | 384 |
| 13 | Granite English R2 | 768 |
| 14 | Arctic Embed M v2.0 | 256 |
| 15 | Arctic Embed M v2.0 | 768 |

---

# Evaluation matrix

For every core model, test:

## Document representation

1. raw chunk;
2. title + chunk;
3. title + heading path + aliases + selected tags + chunk.

## Chunking

- 300 / 40;
- 450 / 50;
- 800 / 50.

## Query categories

- exact-name queries;
- conceptual queries;
- questions whose answer is under a specific heading;
- queries using aliases rather than canonical terms;
- English query → English note;
- French/Dutch/Spanish query → same-language note;
- cross-language query → note in another language;
- queries requiring an exact code/configuration fragment;
- broad exploratory questions.

## Performance

Record separately:

- cold model load;
- full initial indexing;
- incremental note update;
- batch throughput;
- p50 and p95 query embedding latency;
- total search latency;
- peak resident memory;
- model cache size;
- vector database size.

---

# Production reranking recommendation

Changing the dense model alone is unlikely to maximise retrieval quality.

Recommended production flow:

1. retrieve FTS candidates;
2. retrieve dense-vector candidates;
3. optionally retrieve learned sparse candidates;
4. combine through reciprocal-rank fusion;
5. rerank the top 20–40 candidates;
6. return the top 5–10 chunks, with per-note deduplication.

FastEmbed v5 includes reranking support, making this a natural next step after the dense-model benchmark.

Measure reranking separately so its gains are not attributed to the embedding model.

---

# Implementation roadmap

## Phase 1 — make models configurable

- introduce `EmbeddingModelSpec`;
- derive dimensions from the selected model;
- include backend, model revision, dimensions, max length and formatting version in cache identity;
- expose the active model through logs and a system endpoint;
- rebuild the index automatically when any embedding-contract field changes.

Suggested cache identity:

```text
fastembed-v5:embeddinggemma-300m-q4:dim=256:max=2048:format=v2
```

## Phase 2 — contextual chunks and chunk-size evaluation

- create a canonical contextual document representation;
- add model-specific wrappers;
- benchmark 300, 450 and 800 tokens;
- commit evaluation results.

## Phase 3 — first v5 model benchmark

Implement in this order:

1. EmbeddingGemma 300M Q4;
2. Nomic v2 MoE;
3. Qwen3 Embedding 0.6B;
4. existing GTE/BGE/MixedBread baselines.

## Phase 4 — reranking

- add a configurable reranker;
- tune the candidate pool;
- add per-note diversity;
- record latency and quality impact.

## Phase 5 — BGE-M3 research path

Only proceed if ordinary hybrid retrieval plus reranking leaves a measurable quality gap.

## Phase 6 — custom ONNX candidates

Evaluate Granite R2 and Arctic Embed M v2.0 after the model registry and evaluation suite are stable.

---

# Final recommendation

Keep **Nomic Embed Text v1.5** as the control, not necessarily as the permanent default.

My recommended order is:

1. **EmbeddingGemma 300M Q4** — best first practical upgrade;
2. **Nomic Embed Text v2 MoE** — best Apache-licensed multilingual successor;
3. **Qwen3 Embedding 0.6B** — quality ceiling;
4. **BGE-M3** — separate multi-representation retrieval experiment;
5. **Granite English R2 / Small R2** — custom English efficiency candidates;
6. **Arctic Embed M v2.0** — custom multilingual CPU candidate.

The likely winning production configuration is not simply “the newest model.” It is more likely to be:

```text
EmbeddingGemma Q4 or Nomic v2
+ 256-dimensional Matryoshka vectors
+ 350–500-token contextual chunks
+ FTS/vector fusion
+ reranking
```

The model should be selected only after measuring that complete pipeline on Hatchdoor's real multilingual vault.

---

# Primary references

- FastEmbed v5 documentation: <https://docs.rs/fastembed/latest/fastembed/>
- EmbeddingGemma 300M: <https://huggingface.co/google/embeddinggemma-300m>
- Nomic Embed Text v2 MoE: <https://huggingface.co/nomic-ai/nomic-embed-text-v2-moe>
- Qwen3 Embedding 0.6B: <https://huggingface.co/Qwen/Qwen3-Embedding-0.6B>
- BGE-M3: <https://huggingface.co/BAAI/bge-m3>
- Granite Embedding English R2: <https://huggingface.co/ibm-granite/granite-embedding-english-r2>
- Snowflake Arctic Embed M v2.0: <https://huggingface.co/Snowflake/snowflake-arctic-embed-m-v2.0>
