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

When changing a callout accent or any token in `frontend/src/styles/base.css`:

```bash
python3 docs/design/palette.py
```

It checks uniform lightness per theme, the chroma ceilings, and 4.5:1 contrast
on the tinted and badge surfaces, and exits non-zero on a violation. No
dependencies.

Changes should come with tests: a bug fix with a regression test that fails
before the fix, and new behavior with tests that cover it.

Do not commit real vault content, private eval queries, tokens, generated cache
databases, or local model caches.

## Documentation freshness

`docs/user-vault/` is the canonical source of the user documentation. Before
merging into `development`, check whether your branch left it stale:

```bash
just docs-freshness
```

It reports which user-facing surfaces the branch changed and which notes claim
to document each one. The surfaces cover MCP tools, the HTTP API, settings,
Git-backed Vaults, vault lifecycle, search and indexing, layers, attachments,
Markdown, note mutations, security, starter content, startup, the Web UI, and
deployment; the authoritative list is the table in the script. It exits
non-zero, because it cannot tell you whether a note still reads true; only
reading it can.

Read every note it names, update whatever drifted, then record the review:

```bash
just docs-freshness-ack
```

A note the script marks "edited on this branch" only means the file moved. That
is not evidence it is correct. Acknowledging without reading defeats the gate.

The script's surface-to-note table lives in
[`scripts/check-docs-freshness.mjs`](scripts/check-docs-freshness.mjs). When you
add a user-facing surface it does not know about, or rename a note it points at,
update the table and check that every entry still resolves:

```bash
node scripts/check-docs-freshness.mjs --validate-table
node --test scripts/check-docs-freshness.test.mjs
```

A rule whose source path no longer exists matches nothing and silently stops
guarding the surface it names, so the table is verified rather than trusted.

## Claiming scoped work

Hatchdoor uses documented module boundaries so a contributor or coding agent can
work without taking implicit ownership of unrelated code.

Before implementation:

1. Find the relevant boundary in
   [`docs/architecture/module-map.md`](docs/architecture/module-map.md).
2. Read the applicable records in
   [`docs/adr/`](docs/adr/README.md), including any linked record containing the
   full decision.
3. Define the task with the
   [`work-packet template`](docs/architecture/work-packet-template.md).
4. List owned paths, any shared coordination paths, stable contracts,
   dependencies, invariants, and exact validation commands.

A work packet narrows the requested outcome; it does not authorize unrelated
cleanup or broader work. An import or dependency does not make another module
writable.

If implementation requires an undeclared path, stop expanding the diff and
classify it as an internal, contract, or coordination change. A path necessary
for the existing outcome may be declared before editing when it does not
materially increase risk or authority. Ask the user before proceeding when it
would materially broaden the outcome, risk, or required authority.

Any supported contract that crosses its producing module boundary or is
externally observable must follow the
[`interface-change checklist`](docs/architecture/interface-change-checklist.md),
even when one work packet owns the producer and every in-repository consumer.
The checklist does not grant authority to edit undeclared consumers.

Composition files such as `src/server.rs`, `src/app_state.rs`, and
`frontend/src/App.tsx` are expected integration points, not feature-owned
shortcuts. A task may change one when its work packet states the precise
integration required.

When adding, moving, deleting, or reclassifying production source files, update
the module map and verify its structural coverage:

```bash
node scripts/check-module-map.mjs
```

Also update the map when supported contracts, invariants, cross-module
dependencies or consumers, coordination paths, or focused validation change.
The checker verifies path coverage, not whether those descriptions remain
semantically accurate.

When changing the checker itself, run its isolated regression tests:

```bash
node --test scripts/check-module-map.test.mjs
```

## Architecture decisions

Before a structural change, read [`docs/adr/`](docs/adr/README.md). Those records
are the binding constraints your PR is expected to respect — each index row notes
what not to break. If your change needs to break one, don't work around it
quietly: propose a new ADR amending it (the file explains how).

## Visual changes

The frontend is built to a documented design system:
[`docs/design/design-system.html`](docs/design/design-system.html) holds the
tokens, component patterns, layouts, and interaction states. Read it before
changing anything visual, and build from the existing tokens rather than new
values. If you ship a component the system does not cover, add its section in
the same pull request — the document is updated by the change that ships the
component, not afterwards.

## Reporting security issues

Do not open a public issue for a vulnerability. Follow the process in
[`SECURITY.md`](SECURITY.md).

## The local `vault/` directory

`vault/` is the default vault path (`VAULT_PATH` defaults to `./vault`) and is
gitignored — it is not committed. You do not need to create it: on first boot
the app runs `seed_empty_vault`, which creates the directory and, if it has no
Markdown yet, populates it with the starter vault from `docs/starter-vault/`.
Everything else vault-shaped is gitignored too: `demo-vaults/` (read-only demo
content — one folder per vault, e.g. `demo-vaults/para/`), `data/` (generated
cache), and `.fastembed_cache/` (downloaded model weights).
