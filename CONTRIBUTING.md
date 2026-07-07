# Contributing

Hatchdoor is a Rust backend plus a React/Vite frontend.

Before opening a pull request, run the checks that match the files you changed:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all
```

```bash
cd frontend
npm ci
npm run lint
npm run typecheck
npm test
npm run build
```

Do not commit real vault content, private eval queries, tokens, generated cache
databases, or local model caches.

## The tracked `vault/` fixtures

`vault/Home.md` and `vault/Second Note.md` are intentionally committed: they are
the minimal dev fixtures that let the app boot against a real vault out of the
box (`VAULT_PATH` defaults to `./vault`). Keep this directory tiny and generic —
it is a fixture, not a place for real notes. Everything else vault-shaped is
gitignored: `demo-vault/` (read-only demo content), `data/` (generated cache),
and `.fastembed_cache/` (downloaded model weights).
