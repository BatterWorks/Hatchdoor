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
