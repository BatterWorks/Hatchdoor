# Contributing

Hatchdoor is a Rust backend plus a React/Vite frontend. Contributions are
accepted under the project's AGPL-3.0 license.

## Getting started

To run Hatchdoor locally for development, follow the
[Running Without Docker](README.md#running-without-docker) section of the README.

Branch off `development` and open your pull request against `development` — not
`main`, which is the release branch.

## Checks before a pull request

Run the checks that match the files you changed.

Backend (Rust):

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all
```

Frontend (`frontend/`):

```bash
cd frontend
npm ci
npm run lint
npm run typecheck
npm test
npm run build
```

Changes should come with tests: a bug fix with a regression test that fails
before the fix, and new behavior with tests that cover it.

Do not commit real vault content, private eval queries, tokens, generated cache
databases, or local model caches.

## Architecture decisions

Before a structural change, read [`docs/adr/`](docs/adr/README.md). Those records
are the binding constraints your PR is expected to respect — each index row notes
what not to break. If your change needs to break one, don't work around it
quietly: propose a new ADR amending it (the file explains how).

## Reporting security issues

Do not open a public issue for a vulnerability. Follow the process in
[`SECURITY.md`](SECURITY.md).

## The local `vault/` directory

`vault/` is the default vault path (`VAULT_PATH` defaults to `./vault`) and is
gitignored — it is not committed. You do not need to create it: on first boot
the app runs `seed_empty_vault`, which creates the directory and, if it has no
Markdown yet, populates it with the starter vault from `docs/starter-vault/`.
Everything else vault-shaped is gitignored too: `demo-vault/` (read-only demo
content), `data/` (generated cache), and `.fastembed_cache/` (downloaded model
weights).
