# Audit fix-implementer

Cross-window, verification-gated implementation of the **confirmed high +
medium** findings from both audits:

- `../backend-robustness/` (Workflow 2)
- `../client-edge-cases/`

It is the second stage after the audits: a resumable Workflow, driven by cron,
that turns verified findings into reviewed commits without a human present —
safely, because every change must survive the real build + test suite before it
is committed, and nothing is ever merged.

## Pieces

| File | Role |
|---|---|
| `state/_fix-workflow.js` | The resumable Workflow: collect confirmed findings → per-finding test-first (red) where testable → implement → gate → commit or revert+flag → deterministic `FIXES.md`. |
| `state/run-fix-driver.sh` | Cron tick: wait for both audits, provision the worktree, run the Workflow via headless `claude -p`, handle rate-limits. |
| `state/ledger.json` | Durable progress (done / flagged) — the resume source of truth, alongside git-log key trailers. |
| `FIXES.md` | Human-facing rollup: what was committed (with hashes) vs. flagged. |
| `automation.md` | Arm / disarm / review instructions. **Start here.** |

## How it stays safe unattended

- All edits happen in an isolated worktree (`../hatchdoor-audit-fixes`, branch
  `audit-fixes` off `development`) — the main tree is never touched, nothing is
  merged.
- Hybrid TDD: a deterministically testable finding gets a failing test written
  first (red→green proves the fix); non-testable ones are regression-gated. Each
  committed fix is tagged `tdd` or `regr` in `FIXES.md`.
- A fix is committed **only** if `cargo build/test/clippy` (Rust) and/or
  `typecheck/test/lint/build` (frontend) pass. Otherwise it is reverted and
  flagged for a human.
- Resume is idempotent: each commit carries an `Audit-Fix-Key:` trailer, so a
  tick killed between commit and ledger-write never double-applies.

## Status

Not armed. Waits for both audits' `SUMMARY.md`. See `automation.md`.
