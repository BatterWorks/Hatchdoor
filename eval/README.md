# Hatchdoor eval

Developer harness for comparing embedding models against a labelled query set.
The checked-in `queries.jsonl` file is a tiny sample that targets the placeholder
vault in this repository. For real measurements, create your own private query
set against your own vault.

## Vault path

The eval binary indexes the vault at `VAULT_PATH`, falling back to `./vault`
(the 2-note placeholder in the repo). It loads `.env` from the repo root on
startup, so if your `.env` sets `VAULT_PATH=/path/to/notes` you're done. To
override for a single run, prefix the command:

```
VAULT_PATH=/path/to/notes cargo run --release --bin eval -- build ...
```

Sanity-check the indexing log: `notes=2` means it's still pointing at the
placeholder vault.

## Build the three caches (one-time, ~2 h total)

```
cargo run --release --bin eval -- build --model BGESmallENV15      --cache data/cache/hatchdoor-cache-bge-small.sqlite3
cargo run --release --bin eval -- build --model NomicEmbedTextV15  --cache data/cache/hatchdoor-cache-nomic-v1-5.sqlite3
cargo run --release --bin eval -- build --model MxbaiEmbedLargeV1  --cache data/cache/hatchdoor-cache-mxbai-large.sqlite3
```

`MxbaiEmbedLargeV1` is the slow one (~1 h on CPU). Run it overnight or in
`tmux` / `nohup` so a closed terminal doesn't kill it. The build streams
per-note progress so silence means it has hung; non-silence means it is alive.

## Run the eval against any cache

```
cargo run --release --bin eval -- run \
  --model BGESmallENV15 \
  --cache data/cache/hatchdoor-cache-bge-small.sqlite3 \
  --queries eval/queries.jsonl
```

Metrics print to stdout. A section is appended to `eval/results.md`.

The cache records the exact embedding representation, not just its vector
dimension. `run`, `rerank`, `hybrid`, and `compare` refuse a missing or
mismatched stamp; rebuild the disposable cache with the same model, dimension,
and document representation before querying it.

## Rerank an existing cache

The `rerank` subcommand applies a cross-encoder reranker on top of an existing
embedding cache. It does **not** rebuild anything — the cache stays untouched.

```
cargo run --release --bin eval -- rerank \
  --model NomicEmbedTextV15 \
  --cache data/cache/hatchdoor-cache-nomic-v1-5.sqlite3 \
  --reranker JINARerankerV2BaseMultilingual \
  --queries eval/queries.jsonl
```

Available rerankers:

- `JINARerankerV1TurboEn` — 37.8M params, English only, fastest CPU option (~50–100 ms / 20 candidates on i5-class CPU).
- `JINARerankerV2BaseMultilingual` — 278M params, 26 languages, the quality target.

The first invocation per reranker downloads its ONNX weights (~150 MB and ~570 MB respectively) into the fastembed cache. Subsequent runs are fast.

`--initial-k` controls how many embedding candidates are passed to the reranker; default is 20. Metrics are still scored at k=5 and k=10 against the post-rerank order. A section is appended to `eval/results.md` with rank-pre / rank-post / Δ columns, correct-heading and category/tier/language slices, and median / p90 / max latency stats.

## Adding private queries

Copy or replace `eval/queries.jsonl`. One JSON object per line:

```
{
  "id": "U7",
  "query": "...",
  "expected_notes": ["note slug A"],
  "expected_heading_path": "optional heading",
  "anti_expected": ["near-duplicate that must NOT appear in top-5"]
}
```

`expected_heading_path` and `anti_expected` are optional. Keep personal or
sensitive query sets out of public commits.
