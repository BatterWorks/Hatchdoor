# Audit fix-implementer (cron 2) — how to arm / disarm

Automatically implements the **confirmed high + medium** findings from *both*
audits (backend-robustness + client-edge-cases), one verified commit at a time,
across as many 5-hour usage windows as it takes. Companion to the audit cron;
it stays idle until that one finishes.

**Nothing is armed yet.** This file is the on/off switch.

## What it does each tick

1. If all findings are processed (`state/.fixes-complete`) → no-op.
2. If **both** audits aren't done yet (needs `backend-robustness/SUMMARY.md`
   **and** `client-edge-cases/SUMMARY.md`) → waits, no-op. This is the
   "run only after the audit cron completes" gate.
3. Ensures an isolated worktree at `../hatchdoor-audit-fixes` on branch
   `audit-fixes` (branched from `development`), with `node_modules` symlinked.
4. Runs the resumable fix Workflow: for each pending finding it first tries
   **test-first** — write a failing test that reproduces the finding (red),
   confirm it fails, then implement the fix and confirm green. If no
   deterministic test is feasible (perf/visual/race findings), it falls back to
   a **regression gate**. Either way it runs the **full gate** (`cargo fmt`→
   `build`→`test`→`clippy` for Rust; `prettier`/`eslint --fix`→`typecheck`→
   `test`→`lint`→`build` for frontend), then **commits** the fix (one commit per
   fix, with an `Audit-Fix-Key:` trailer, and the new test when there is one) or
   **reverts + flags** it for you.
5. Never merges. You review branch `audit-fixes` and integrate what you want.

## Safety model

- **Isolated worktree** — never touches your main working directory or
  `development`/`main`.
- **Strict gate** — a fix is committed only if the real build + test suite
  stays green. A fix that can't be made green (after one repair attempt) is
  fully reverted and listed under "Flagged for human" in `FIXES.md`.
- **Test-first where possible; know which is which.** Testable findings get a
  new failing→passing test, so the fix is self-proving (`tdd` in `FIXES.md`).
  Findings that can't be deterministically tested (perf, layout, races) are only
  `regr` — "still compiles + nothing else broke", not proof the fix behaves.
  Treat `FIXES.md` as a review queue, and scrutinise the `regr` rows by hand.
- **Idempotent resume** — commits carry the finding key; the loader rebuilds
  "done" from git log *and* the ledger, and every run hard-cleans any
  uncommitted remains of a killed tick. No double-commits.
- `--dangerously-skip-permissions` (unattended) — blast radius is the worktree
  plus `docs/audits/_fixes/`.

## Arm (start it)

Fires every 30 min; ticks before the audits finish are cheap no-ops.

```bash
( crontab -l 2>/dev/null; \
  echo '*/30 * * * * /home/battermanz/coding/hatchdoor/docs/audits/_fixes/state/run-fix-driver.sh' \
) | crontab -
crontab -l | grep run-fix-driver.sh   # verify
```

You can arm this **at the same time as the audit cron** — it will simply wait
until both `SUMMARY.md` files exist before doing any work.

## Watch progress

```bash
tail -f docs/audits/_fixes/state/fix-driver.log     # driver ticks
cat    docs/audits/_fixes/FIXES.md                  # committed vs flagged
git -C ../hatchdoor-audit-fixes log --oneline       # the fix commits
```

## Review the result

```bash
cd ../hatchdoor-audit-fixes
git log --oneline development..HEAD                 # every auto-fix
git diff development..HEAD                           # the whole change set
# integrate selectively, e.g.:  git -C ../hatchdoor cherry-pick <hash>
```

## Disarm

```bash
crontab -l | grep -v run-fix-driver.sh | crontab -
# optional cleanup once you've integrated what you want:
git -C /home/battermanz/coding/hatchdoor worktree remove ../hatchdoor-audit-fixes
```

## First-run check (recommended)

Only meaningful once both audits are done. Run one tick by hand and watch it:

```bash
docs/audits/_fixes/state/run-fix-driver.sh
tail -n 60 docs/audits/_fixes/state/fix-driver.log
```

## Tuning

Edit the `CONFIG` block at the top of `state/_fix-workflow.js`:
- `severities` — add `'critical'`/`'low'` to widen scope.
- `repairAttempts` — retries after a failing gate before reverting (default 1).
- `gates` — the exact format/check commands per language.
