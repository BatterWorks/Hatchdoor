# Hatchdoor eval

Developer harness for comparing embedding models against a labelled query set
from the real vault. See `docs/superpowers/specs/2026-05-18-phase-1.5-eval-design.md`
for the full design.

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

## Adding queries

Edit `eval/queries.jsonl`. One JSON object per line:

```
{
  "id": "U7",
  "query": "...",
  "expected_notes": ["note slug A"],
  "expected_heading_path": "optional heading",
  "anti_expected": ["near-duplicate that must NOT appear in top-5"]
}
```

`expected_heading_path` and `anti_expected` are optional. Add queries when real
use surfaces something the eval currently misses.
