# Agent Instructions

## Running dev servers

Use `just dev-start` / `just dev-stop` (see `justfile`) instead of backgrounding
raw `cargo run` / `npm run dev` calls. Those get orphaned across sessions and
duplicate build-cache directories accumulate when `CARGO_TARGET_DIR` isn't set
in the invoking shell - `just dev-start` handles both. `just dev-status` shows
what's currently running before you start another copy.

## CodeGraph

When a `.codegraph/` directory exists at the repository root, reach for
CodeGraph before grep, find, or reading source files when locating or
understanding code:

```bash
codegraph explore "<symbols or question>"
```

When the CodeGraph MCP tool is available, prefer `codegraph_explore`. It returns
relevant source, call paths, and blast-radius information in one query.

If `.codegraph/` is absent, use normal search/read tools and do not initialize an
index automatically. If the directory exists but neither the MCP tool nor CLI is
available, fall back to normal search/read tools rather than blocking the task.

## Collaboration boundaries

Before changing code, tests, or project tooling/configuration:

1. Read [`docs/architecture/module-map.md`](docs/architecture/module-map.md).
2. Read the relevant ADR records in
   [`docs/adr/README.md`](docs/adr/README.md), including any linked record that
   contains the full decision.
3. Identify or create a work packet using
   [`docs/architecture/work-packet-template.md`](docs/architecture/work-packet-template.md).
4. State the owned paths, public contract, coordination paths, consumed
   dependencies, forbidden paths/invariants, and validation commands.

An existing issue or task description counts as the work packet when it includes
those fields. Otherwise, record them in the agent's working plan or task notes
before coding. Committing a separate packet document is optional unless the
task explicitly requests one.

A work packet narrows the user-requested outcome; it never grants authority to
broaden that outcome or perform opportunistic cleanup. Owned paths are writable
only within that outcome. Coordination paths are writable only when the packet
explicitly lists the required integration change. Consuming a module does not
grant permission to edit it.

If implementation requires an undeclared path:

- stop expanding the diff;
- determine whether it is an internal, contract, or coordination change;
- when the path is necessary for the existing requested outcome and does not
  materially increase risk or authority, declare it in the work packet before
  editing it;
- when it would materially broaden the outcome, risk, or required authority,
  stop and ask the user before proceeding;
- follow the
  [`interface-change checklist`](docs/architecture/interface-change-checklist.md)
  whenever a supported contract crosses its producing module boundary or is
  externally observable, even when one work packet owns the producer and every
  in-repository consumer.

Do not treat a directory layout as proof of isolation. Composition files such
as `src/server.rs`, `src/app_state.rs`, and `frontend/src/App.tsx` are declared
integration points and have no default feature owner.

## Architecture guardrails

- Keep one binary and one shared core; do not split domains into services.
- Keep HTTP and MCP mutations routed through `src/vault/write/`.
- Keep Markdown authoritative and SQLite disposable.
- Keep runtime search behavior consistent with ADR-05 unless a superseding ADR
  is part of the task.
- Prefer Rust privacy, narrow re-exports, existing ESLint rules, and focused
  tests over new frameworks or speculative abstractions.
- Do not add traits, packages, state libraries, or code generation solely to
  make a boundary appear more formal.

## Validation

Run the focused checks named in the work packet while developing. Before
hand-off, run the full checks appropriate to all changed paths as documented in
`CONTRIBUTING.md`.

When production source files are added, moved, deleted, or reclassified, update
the module map and, from the repository root, run:

```bash
node scripts/check-module-map.mjs
```

Also update the map when supported contracts, invariants, cross-module
dependencies/consumers, coordination paths, or focused validation change. The
checker verifies structural coverage only; semantic accuracy still requires
review.
