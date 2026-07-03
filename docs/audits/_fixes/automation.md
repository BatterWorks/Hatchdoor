# Audit fix-implementer (cron 2) — how to arm / disarm

Automatically implements the **confirmed high + medium + low** findings from the
**client-edge-cases** audit, one verified commit at a time, across as many 5-hour
usage windows as it takes. (The backend-robustness audit is already fixed on
`development`, so it is out of scope here.)

**Nothing is armed yet.** This file is the on/off switch.

## What it does each tick

1. If all findings are processed (`state/.fixes-complete`) → no-op.
2. If the client audit isn't done yet (needs `client-edge-cases/SUMMARY.md`) →
   waits, no-op.
3. Ensures a private **scratch worktree** at `../hatchdoor-audit-fixes` on branch
   `audit-fixes` (branched from `development`), `node_modules` symlinked, and
   fast-forwards it up to `development`'s tip so any fixes you did attended are
   picked up (never redone).
4. Runs the resumable fix Workflow: for each pending finding it first tries
   **test-first** — write a failing test that reproduces the finding (red),
   confirm it fails, then implement the fix and confirm green. If no
   deterministic test is feasible (perf/visual/race findings), it falls back to
   a **regression gate**. Either way it runs the **full frontend gate**
   (`prettier`/`eslint --fix` → `typecheck` → `test` → `lint` → `build`), then
   **commits** the fix in the scratch worktree (one commit per fix, with an
   `Audit-Fix-Key:` trailer, plus the new test when there is one) or **reverts +
   flags** it.
5. **Fast-forwards the passing commits onto `development`** — but only when your
   main working tree is **clean and on `development`**, so your uncommitted work
   is never touched. If the tree is busy, the commits wait safely on the scratch
   branch and forward on a later tick.

## Safety model

- **Your WIP is never touched.** All editing / `reset --hard` cleanup happens in
  the *scratch worktree*, a separate directory the engine fully owns. The only
  thing that reaches your repo is a **fast-forward** of `development`, and only
  when your tree is clean. Dirty tree → it skips forwarding and logs it.
- **One writer at a time.** `run-fix-driver.sh` holds an `flock`, so a foreground
  (attended) run and the cron can never run the Workflow — or any agent —
  simultaneously. The Workflow itself is strictly sequential (one agent at a time).
- **Strict gate** — a fix is committed only if the real build + test suite stays
  green. A fix that can't be made green (after one repair attempt) is fully
  reverted and listed under "Flagged for human" in `FIXES.md`.
- **Test-first where possible; know which is which.** Testable findings get a new
  failing→passing test (`tdd` in `FIXES.md`). Findings that can't be
  deterministically tested (perf, layout, races) are only `regr` — "still compiles
  + nothing else broke", not proof the fix behaves. Scrutinise the `regr` rows by
  hand.
- **Idempotent resume** — commits carry the finding key; the loader rebuilds
  "done" from git log *and* the ledger, and the ledger is checkpointed to disk
  after **every** finding. No double-commits across a killed tick.
- `--dangerously-skip-permissions` (unattended) — blast radius is the scratch
  worktree plus `docs/audits/_fixes/` runtime state (gitignored).

## Scope: 25 findings

5 high · 12 medium · 8 low across categories 01–07 (`docs/audits/client-edge-cases/`).
The 2 MB-upload-cap finding is already fixed backend-side and the duplicate
copyNoteLink low will **self-abstain** (their red tests can't reproduce), so those
land as "flagged: abstained", not edits.

## Arm (start it)

Fires every 30 min; ticks before the audit is done, or while you're mid-session,
are cheap no-ops.

```bash
( crontab -l 2>/dev/null; \
  echo '*/30 * * * * /home/battermanz/coding/hatchdoor/docs/audits/_fixes/state/run-fix-driver.sh' \
) | crontab -
crontab -l | grep run-fix-driver.sh   # verify
```

## Watch progress

```bash
tail -f docs/audits/_fixes/state/fix-driver.log     # driver ticks
cat    docs/audits/_fixes/FIXES.md                  # committed vs flagged
git    log --oneline development                     # the fix commits (on development)
```

## First-run check (recommended)

Run one tick by hand and watch it before arming the cron:

```bash
docs/audits/_fixes/state/run-fix-driver.sh
tail -n 80 docs/audits/_fixes/state/fix-driver.log
```

## Disarm

```bash
crontab -l | grep -v run-fix-driver.sh | crontab -
# optional cleanup once you're done:
git worktree remove ../hatchdoor-audit-fixes
```

## Tuning

Edit the `CONFIG` block at the top of `state/_fix-workflow.js`:
- `severities` — currently `['high','medium','low']`.
- `auditStateGlobs` — currently client-edge-cases only.
- `repairAttempts` — retries after a failing gate before reverting (default 1).
- `gates` — the exact format/check commands per language.
