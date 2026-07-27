# Embedding sweep decisions — 2026-07-26

> Research record.

This records the decisions taken while the embedding evaluation was in progress.
The raw, per-query measurements are in
[`eval/results.md`](../../../eval/results.md).

## What was completed

The initial exploratory grid covered four model/dimension configurations, three
chunk settings, and contextual documents on and off: 24 completed cells. The
application control is Nomic Embed Text v1.5 at native 768 dimensions; its
current build defaults are 800-token chunks, 50-token overlap, and contextual
document headers.

The original 42-cell sweep was narrowed before the remaining models were run.
This is intentional: the completed cells gave sufficiently consistent evidence
to lock the document representation, so the remaining budget is better spent
on model comparison.

## Locked representation for the continuation

Use **800-token chunks, 50-token overlap, and contextual documents enabled**.

Why:

- 800/50 was the best or tied-best general retrieval setting in the completed
  configurations. It consistently produced the strongest Recall@10 and usually
  the strongest Recall@5 and MRR.
- Turning context off improved note coverage: it won Recall@5 in 10 of 12
  like-for-like comparisons (two ties), and Recall@10 in 11 of 12.
- Turning context on improved result quality: it won MRR in 11 of 12 comparisons
  and correct-heading accuracy in all 12. It also usually reduced false positives.
- Hatchdoor normally presents ranked results and routes people into a specific
  note section, so first-result quality and heading accuracy are more valuable
  than the extra broad-recall gain from context-off indexing.

450/50 with context on remains the specialist choice for heading navigation:
it had the strongest correct-heading score in every completed model/dimension
configuration. It is not the general default because 800/50 had the better
overall retrieval profile.

## Model findings so far

### Current control: Nomic Embed Text v1.5

At the locked representation, the control scored Recall@5 0.822, Recall@10
0.907, MRR 0.738, FP-rate@5 0.277, and correct-heading 0.750.

### GTE Base English v1.5

Do not promote this model. At the same representation it scored lower than the
control on Recall@5 (0.814), Recall@10 (0.881), and MRR (0.700), with a higher
FP-rate@5 (0.301). Correct-heading tied at 0.750. It remains a useful
English-only floor/control rather than an upgrade.

### Nomic Embed Text v2 MoE

Native v2 MoE is the strongest completed candidate for raw recall. Its best
cell, 800/50 with context off, reached Recall@5 0.915 and Recall@10 0.941.
The balanced, contextual 800/50 cell scored Recall@5 0.864, Recall@10 0.924,
MRR 0.771, and correct-heading 0.792: better than the current control on all
of those quality measures, but with higher FP-rate@5 (0.325 versus 0.277).

It is not an automatic deployment choice. Its build peak RSS was about 3.64 GB
versus about 0.96 GB for the current model, and the model has a 512-token input
limit, so an 800-token chunk can be truncated. The 256-dimensional variant did
not provide a compelling quality/storage trade-off in the completed cells.

## Continuation scope

Run exactly these five model/dimension candidates at 800/50 with context on:

1. Qwen3 Embedding 0.6B at 512 dimensions.
2. EmbeddingGemma 300M Q4 at native 768 dimensions.
3. EmbeddingGemma 300M Q4 at 256 dimensions.
4. Snowflake Arctic Embed M v2 at native 768 dimensions.
5. Snowflake Arctic Embed M v2 at 256 dimensions.

EmbeddingGemma is included for benchmark comparison only. It uses Gemma terms,
so a licensing review is required before considering it as Hatchdoor's default
distributed model.

## Completed continuation findings

All five continuation cells completed. The native 768-dimensional
**EmbeddingGemma 300M Q4** configuration is the best overall result in this
evaluation.

| Model | Recall@5 | Recall@10 | MRR | FP-rate@5 | Correct-heading | Build | Peak RSS |
|---|---:|---:|---:|---:|---:|---:|---:|
| Current Nomic v1.5 | 0.822 | 0.907 | 0.738 | 0.277 | 0.750 | 895 s | 1.26 GB |
| EmbeddingGemma Q4, native | **0.915** | **0.958** | 0.801 | 0.349 | 0.792 | **887 s** | **0.55 GB** |
| Snowflake Arctic M v2, native | 0.907 | 0.932 | **0.810** | 0.313 | 0.792 | 1,071 s | 3.12 GB |
| Nomic v2 MoE, native | 0.864 | 0.924 | 0.771 | 0.325 | 0.792 | 2,113 s | 3.64 GB |
| Qwen3 0.6B, 512 dimensions | 0.737 | 0.847 | 0.654 | 0.277 | 0.792 | 5,617 s | 3.43 GB |

### Interpretation

- **Gemma Q4 native is the quality-and-efficiency winner.** Against the current
  control, it adds 0.093 Recall@5, 0.051 Recall@10, and 0.063 MRR, improves
  heading accuracy, uses less than half the peak build memory, and takes the
  same time to build.
- **Gemma's drawback is precision.** Its FP-rate@5 is 0.349, versus 0.277 for
  the control. It retrieves more expected material, but it also places more
  explicitly undesirable notes in the top five. This needs a qualitative check
  of those false positives before rollout.
- **Arctic native has the best first-hit ranking** (MRR 0.810) and a lower
  FP-rate@5 than Gemma (0.313), but its recall is slightly lower and it needs
  about 5.7 times Gemma's build RAM. That small MRR advantage does not justify
  the operational cost as a general default.
- **Qwen3 at 512 dimensions is rejected.** It is much slower and heavier than
  the control while scoring worse. This finding applies to the tested
  512-dimensional configuration, not an untested native 1,024-dimensional one.
- **Do not use Gemma at 256 dimensions.** MRR falls from 0.801 to 0.689 and
  Recall@5 from 0.915 to 0.831, while the retained SQLite cache only shrinks
  from about 11.1 MB to 9.0 MB.
- **Arctic at 256 dimensions is viable but not compelling.** It retains strong
  quality (Recall@5 0.898, MRR 0.780), but has the same roughly 3.12 GB model
  build-memory requirement as native Arctic.

## Operational decisions

- Preserve all 24 completed result sections; the continuation script must not
  archive `eval/results.md` on startup.
- Retain successful SQLite caches under `data/cache/sweep/` so candidates can
  be inspected, rerun against another query set, or used for follow-up tests.
  The script still removes a stale same-name cache before rebuilding and removes
  a cache whose build fails.
- Do not restart the sweep automatically after changing it. The user starts the
  five-cell continuation manually with `bash eval/run-sweep.local.sh`.

## Production recommendation and remaining gate

Subject to accepting the Gemma terms, make **EmbeddingGemma300MQ4 at native
768 dimensions with the Gemma retrieval-format v1 template, 800-token chunks,
zero overlap, and contextual documents** the production candidate. It is the
strongest quality-and-efficiency result in the completed sweep.

Before changing the production embedder, review the Gemma terms and inspect the
additional top-five false positives. If either blocks adoption, Arctic native
is the strongest technical fallback, with the stated 3.12 GB build-memory cost.

## Prompted Gemma chunk and overlap follow-up

The first Gemma result used Hatchdoor's generic contextual text and no Gemma
retrieval prompt. The follow-up corrected that representation to Gemma's
retrieval contract:

- query: `task: search result | query: <query>`;
- document: `title: <note title> | text: Section: <heading path> <body>`.

The corrected representation was tested at native 768 dimensions with context
on. Results:

| Chunk / overlap | Recall@5 | Recall@10 | MRR | FP-rate@5 | Correct-heading |
|---|---:|---:|---:|---:|---:|
| 800 / 0 | **0.958** | 0.958 | **0.846** | 0.361 | 0.833 |
| 800 / 50 | **0.958** | 0.958 | **0.846** | 0.361 | 0.833 |
| 800 / 100 | 0.958 | 0.958 | 0.838 | 0.361 | 0.833 |
| 450 / 50 | 0.941 | 0.958 | 0.830 | **0.337** | **0.958** |
| 1200 / 75 | 0.932 | **0.966** | 0.835 | 0.361 | 0.500 |
| 1600 / 100 | 0.924 | 0.958 | 0.801 | 0.422 | 0.083 |

### Decision

Use **800 tokens with zero overlap** as the general configuration. It ties
800/50 on every primary quality metric, builds faster (918 s versus 1,033 s),
and avoids redundant content. The two caches are almost identical in size,
which indicates that the Markdown-aware splitter rarely needed overlap to join
an otherwise artificial boundary in this vault.

This does not establish that zero overlap is universally best. It is the right
choice for this structured Markdown vault, where title and heading context is
embedded with every chunk. Re-evaluate it if the corpus gains substantial
unstructured PDFs, transcripts, or long prose.

450/50 remains the specialist setting for maximum heading precision. It is not
the default because 800/0 has stronger general note retrieval and ranking.

### Remaining validation before a production change

- Review the Gemma terms.
- Qualitatively inspect the extra top-five false positives: prompted Gemma's
  FP-rate@5 is 0.361, above the Nomic v1.5 control's 0.277.
- Run a small held-out query set or real-user search canary so the configuration
  is not chosen solely on the set used for tuning.
- Measure live query latency and a full re-index on the intended production VM.

## Gemma indexing batch-size and CPU follow-up

The question was whether Gemma's indexing CPU use could be raised to make a
full build faster. This was a throughput test, not a retrieval-quality sweep:
each cell used Gemma native with retrieval-format v1, contextual documents,
800-token chunks, zero overlap, the same 378-note vault, and a fresh cache.
No query evaluation was run because batching does not change the configured
model or document representation.

| Batch size | Build time | Process CPU | Peak RSS | Embed calls | Padding |
|---:|---:|---:|---:|---:|---:|
| 1 | **895.7 s** | 282% | **538 MB** | 775 | **0.0%** |
| 2 | 1,068.7 s (+19%) | 299% | 577 MB | 538 | 14.3% |
| 4 | 1,078.4 s (+20%) | 314% | 622 MB | 429 | 20.2% |

All three cells processed the same 775 chunks and 381,165 real embedding
tokens. The CPU figures are whole-process percentages: on the four-vCPU VM,
batch 1 used about 70.5% of available aggregate CPU capacity, batch 2 about
74.8%, and batch 4 about 78.6%.

### Decision

Keep **batch size 1**. It is both the fastest and the lowest-memory option.
Batch 2 and batch 4 do use slightly more CPU, but this is counterproductive:
the ONNX backend pads every batch to its longest input. Batch 2 performed
63,780 padded tokens of extra work; batch 4 performed 96,657. The vault also
has many one-chunk notes, so even at batch 2 and batch 4 the median effective
call still contained only one input. Fewer calls therefore did not make up for
the added padded computation.

The evaluator retains `--batch-size` as a benchmark-only control, but the
production `BuildOptions` default remains one input per call.

### Why Nomic can show higher CPU use

Hatchdoor does not give Nomic a special thread setting: both Nomic v1.5 and
EmbeddingGemma Q4 are constructed through the same FastEmbed/ONNX Runtime
path, with no model-specific session thread count in this code. The observed
difference is therefore most plausibly in their exported ONNX graphs and
kernels: Gemma Q4's quantized operations have less parallel work or are more
memory-bound for parts of inference, whereas Nomic's graph keeps more cores
busy. This is an inference, not an operator-level profile.

Higher CPU use is not itself a performance goal. Gemma batch 1 completed in
895.7 seconds—essentially the same full-build time as the prior Nomic control
measurement—while the attempts to raise Gemma CPU use made it about 20% slower.
An ONNX Runtime operator profile would be required to attribute the remaining
idle CPU to specific kernels; there is no current evidence that pursuing it
will improve indexing time.

## Reranking: rejected for the local CPU deployment

The proposed multilingual reranker follow-up was stopped during its first
cell: BGE Reranker v2 M3, using the locked Gemma retrieval cache, an initial
candidate set of 20, and a 512-token query-document limit. After about 16
minutes of wall time it was still evaluating the 125-query set, using roughly
three CPU cores and 3.5 GB of resident memory. It had accumulated about 49
minutes of CPU time.

This is a performance-gate result, not a retrieval-quality result: the cell
was intentionally stopped before it produced metrics. It is sufficient to
reject cross-encoder reranking from Hatchdoor's local CPU search path. The
added BGE/GTE test wiring and local Phase-1 harness were removed rather than
continuing a model sweep whose resource profile cannot meet the interactive
deployment target.
