# Contributing

Hatchdoor is a Rust backend plus a React/Vite frontend. Contributions are
accepted under the project's AGPL-3.0 license.

## Getting started

To run Hatchdoor locally for development, follow the
[Running Without Docker](README.md#running-without-docker) section of the README.

Branch off `development` and open your pull request against `development` — not
`main`, which is the release branch.

## Checks before a pull request

Run the checks before opening a pull request:

```bash
just check
```

It runs these, stopping at the first failure:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all --features eval
cd frontend
npm run format:check
npm run lint
npm run typecheck
npm test
npm run build
```

Run `npm ci` in `frontend/` first on a fresh checkout or after the lockfile
changes. While you work, run the individual commands for the side you are
touching, or a focused `cargo test <module>`.

The offline benchmark harness (the `eval` and `index_microbench` binaries and
the candle-backed embedders they sweep) sits behind the non-default `eval`
feature, so a default build compiles none of it. Clippy therefore runs twice:
the default run lints exactly what a deployment ships, and the all-features run
is the only one that compiles the gated lines. The tests run once, with
`--features eval`. That run covers every test a default build has, plus the
harness's own test targets, which a default `cargo test` skips entirely.
Without it, gated code rots with nothing to say so.

The full suite adds two more backend runs:

```bash
just check-full
```

It runs the tests in the exact default configuration a deployment ships, and
with `--all-features`, which turns on `embedder-tests`: tests that load real
model weights. The first such run downloads the weights from Hugging Face, so
allow for that once, and for the disk it costs; afterwards they come from the
local cache. Run `just check-full` when you change `src/embed/`, `src/rerank/`,
model identities, or the version of `fastembed` or another inference
dependency.

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
Markdown, note mutations, security, startup, the Web UI, and
deployment; the authoritative list is the table in the script. It exits
non-zero, because it cannot tell you whether a note still reads true; only
reading it can.

Read every note it names, update whatever drifted, then record the review:

```bash
just docs-freshness-ack
```

A note the script marks "edited on this branch" only means the file moved. That
is not evidence it is correct. Acknowledging without reading defeats the gate.

The same script refuses a branch that changes code Hatchdoor ships without editing `CHANGELOG.md`, and acknowledging does not waive that. Add an entry under `## Unreleased` for anything a user, operator or agent can notice. When nothing noticeable changed, such as a test-only change or an internal refactor, put a trailer on one of the branch's commits instead:

```text
Changelog: none, <reason>
```

The list of shipped paths is `SHIPPED_PATHS` in the script; test files under them do not count.

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
gitignored — it is not committed. Hatchdoor neither creates nor fills it: create the folder yourself before pointing a Vault at it, and it stays as empty as you leave it.
Everything else vault-shaped is gitignored too: `demo-vaults/` (read-only demo
content — one folder per vault, e.g. `demo-vaults/para/`), `data/` (generated
cache), and `.fastembed_cache/` (downloaded model weights).
