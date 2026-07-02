# Cross-window auto-resume — how to arm / disarm

The backend-robustness audit is a resumable Workflow. `run-driver.sh` lets it
run **unattended across many 5-hour usage windows**: cron wakes it on an
interval, it spends whatever budget the current window has, and picks up where
it left off after the limit replenishes — until all 7 categories + `SUMMARY.md`
exist, then it stops doing work.

**Nothing is armed yet.** This file is the on/off switch.

## Arm (start the auto-resume loop)

Adds a cron entry firing every 30 minutes. A rate-limited tick fails fast and
costs nothing; a tick with budget makes progress.

```bash
( crontab -l 2>/dev/null; \
  echo '*/30 * * * * /home/battermanz/coding/hatchdoor/docs/audits/backend-robustness/state/run-driver.sh' \
) | crontab -
```

Verify it registered:

```bash
crontab -l | grep run-driver.sh
```

## Watch progress

```bash
tail -f docs/audits/backend-robustness/state/driver.log   # driver ticks
ls    docs/audits/backend-robustness/*.md                 # reports as they land
cat   docs/audits/backend-robustness/SUMMARY.md           # final rollup
```

The driver writes `.audit-complete` when every category report and
`SUMMARY.md` are present; from then on each tick is a no-op.

## Disarm (stop waking)

Do this once the audit is done (or any time you want to stop). Cron does **not**
self-remove.

```bash
crontab -l | grep -v run-driver.sh | crontab -
```

## First-run sanity check (recommended before trusting it overnight)

Run one tick by hand and watch it, so an auth/permission problem surfaces while
you're looking:

```bash
docs/audits/backend-robustness/state/run-driver.sh
tail -n 60 docs/audits/backend-robustness/state/driver.log
```

## Notes / caveats

- **Must run locally.** The finders read `src/` and use the `codegraph` shell
  command — a cloud routine has no filesystem, so this is OS cron on this box,
  not a `/schedule` cloud agent.
- **`--dangerously-skip-permissions`** is used because no human is present to
  approve the subagents' Read/Write/Bash. Blast radius is contained: the job
  reads the repo and writes only under `docs/audits/backend-robustness/`.
- **Budget can't be read live.** There's no API telling the run how much of the
  5hr allowance is left, so it can't target "spend exactly all of it" — it
  simply uses whatever a window gives before the limit trips, then resumes.
- **Interrupt safety** comes from the workflow's per-category disk checkpoints,
  not from cron. Killing a tick mid-run at worst discards the one in-flight
  category, which is re-found next tick (finder output is cached per category).
- A run can outlast a 30-min tick; `flock` makes the next tick skip rather than
  double-run.
