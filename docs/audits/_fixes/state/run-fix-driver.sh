#!/usr/bin/env bash
# ============================================================================
# Audit fix-implementer — cross-window driver (cron 2).
#
# Fixes the confirmed client-edge-case findings, one verified commit at a time,
# across as many 5-hour usage windows as it takes. Each tick:
#   1. no-op if the fixes are already complete (.fixes-complete)
#   2. WAIT (no-op) until the client audit is done (its SUMMARY.md exists)
#   3. ensure a private scratch worktree exists (branch audit-fixes), and
#      sync it up to development's tip so any attended fixes are picked up
#   4. run the resumable fix Workflow via headless `claude -p` — it commits each
#      verified fix to the scratch branch (never in your main working copy)
#   5. fast-forward those commits onto `development` — but only if your main
#      working tree is clean, so your uncommitted work is NEVER touched
#   6. handle rate-limit / error by failing fast and retrying next tick
#
# Concurrency: the flock below serialises every run (this driver run attended in
# the foreground AND the cron), so no two Workflow invocations — and therefore no
# two agents — ever run at once. A killed tick resumes cleanly from the ledger +
# the Audit-Fix-Key git trailers.
# ============================================================================
set -uo pipefail

# cron runs with a bare PATH — put claude, the node toolchain (node/npm/npx/
# codegraph), and cargo/rustup back on it so the workflow's subagents can shell out.
export PATH="/home/battermanz/.local/bin:/home/battermanz/.nvm/versions/node/v24.14.0/bin:/home/battermanz/.cargo/bin:$PATH"

REPO="/home/battermanz/coding/hatchdoor"
DIR="$REPO/docs/audits/_fixes"
STATE="$DIR/state"
WF="$STATE/_fix-workflow.js"
WT="/home/battermanz/coding/hatchdoor-audit-fixes"
BRANCH="audit-fixes"
BASE="development"
LOG="$STATE/fix-driver.log"
LOCK="$STATE/fix-driver.lock"
DONE="$STATE/.fixes-complete"

CLIENT_SUMMARY="$REPO/docs/audits/client-edge-cases/SUMMARY.md"

log() { printf '%s %s\n' "$(date -Is)" "$*" >>"$LOG"; }

exec 9>"$LOCK"
if ! flock -n 9; then
  log "another run in progress (attended or cron) — skipping this tick"
  exit 0
fi

if [[ -f "$DONE" ]]; then
  log "fixes already complete — nothing to do (disarm cron; see automation.md)"
  exit 0
fi

# --- wait gate: the client audit must be finished ---
if [[ ! -s "$CLIENT_SUMMARY" ]]; then
  log "waiting — client audit not complete yet (need client SUMMARY.md)"
  exit 0
fi

# --- ensure the private scratch worktree exists on the audit-fixes branch ---
if ! git -C "$REPO" worktree list --porcelain 2>/dev/null | grep -q "^worktree $WT$"; then
  log "creating scratch worktree $WT"
  if git -C "$REPO" show-ref --verify --quiet "refs/heads/$BRANCH"; then
    git -C "$REPO" worktree add "$WT" "$BRANCH" >>"$LOG" 2>&1
  else
    git -C "$REPO" worktree add "$WT" -b "$BRANCH" "$BASE" >>"$LOG" 2>&1
  fi || { log "FATAL: worktree add failed"; exit 1; }
fi

# --- deps: symlink node_modules (identical deps -> safe, instant; no reinstall) ---
if [[ ! -e "$WT/frontend/node_modules" && -d "$REPO/frontend/node_modules" ]]; then
  ln -s "$REPO/frontend/node_modules" "$WT/frontend/node_modules" && log "linked node_modules into worktree"
fi

# --- sync the scratch branch up to development's tip (pick up attended fixes) ---
# ff-only: if development moved ahead it fast-forwards; if the scratch branch has
# un-forwarded commits (a prior ff was skipped) this is a no-op and keeps them.
git -C "$WT" merge --ff-only "$BASE" >>"$LOG" 2>&1 || log "scratch not fast-forwardable to $BASE (has un-forwarded commits) — proceeding"

cd "$REPO" || { log "FATAL: cd $REPO failed"; exit 1; }
log "run start (scratch $WT, branch $BRANCH)"

PROMPT="Run the audit fix-implementer. Call the Workflow tool exactly once with {\"scriptPath\": \"$WF\"}. Do not read files, plan, or do anything else before or after — invoke that single tool and report the JSON it returns. It is resumable and self-checkpointing; it edits and commits ONLY inside the scratch worktree $WT."

OUT="$(claude -p "$PROMPT" --dangerously-skip-permissions 2>&1)"
CODE=$?
printf '%s\n' "----- tick $(date -Is) (exit $CODE) -----" >>"$LOG"
printf '%s\n' "$OUT" | tail -n 40 >>"$LOG"

if printf '%s' "$OUT" | grep -qiE 'usage limit|rate limit|resets at|limit reached|too many requests'; then
  log "rate-limited (exit $CODE) — retry next tick"
  exit 0
fi
if [[ $CODE -ne 0 ]]; then
  log "claude exited $CODE — retry next tick"
  exit 0
fi

# --- forward the verified fix commits onto development ---
# Only when your main working tree is clean AND on development, so uncommitted
# work is never clobbered. If it's busy, the commits wait safely on the scratch
# branch and forward on a later tick. The scratch branch is always ahead of
# development by construction, so this is always a genuine fast-forward.
CUR_BRANCH="$(git -C "$REPO" rev-parse --abbrev-ref HEAD 2>/dev/null)"
if [[ -n "$(git -C "$REPO" status --porcelain 2>/dev/null)" ]]; then
  log "main working tree is dirty — leaving fixes on '$BRANCH', not forwarding to $BASE this tick"
elif [[ "$CUR_BRANCH" != "$BASE" ]]; then
  log "main repo is on '$CUR_BRANCH', not '$BASE' — leaving fixes on '$BRANCH', not forwarding"
else
  if git -C "$REPO" merge --ff-only "$BRANCH" >>"$LOG" 2>&1; then
    log "fast-forwarded $BASE to the fix commits on '$BRANCH'"
  else
    log "ff-only merge of '$BRANCH' into $BASE refused — left on '$BRANCH' for manual review"
  fi
fi

if [[ -f "$DONE" ]]; then
  log "run complete — ALL FIXES PROCESSED (on $BASE; disarm cron — see automation.md)"
else
  log "run complete — progress made, findings still pending"
fi
