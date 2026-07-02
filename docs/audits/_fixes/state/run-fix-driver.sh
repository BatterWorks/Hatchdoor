#!/usr/bin/env bash
# ============================================================================
# Audit fix-implementer — cross-window driver (cron 2).
#
# Sibling to the audit driver. Each tick:
#   1. no-op if the fixes are already complete (.fixes-complete)
#   2. WAIT (no-op) until BOTH audits are done (both SUMMARY.md exist) — this is
#      what makes it "start only after the audit cron finishes"
#   3. ensure the isolated worktree exists, with node_modules linked so the
#      frontend gate can run
#   4. run the resumable fix Workflow via headless `claude -p`
#   5. handle rate-limit / error by failing fast and retrying next tick
# The Workflow commits each verified fix to the `audit-fixes` branch and NEVER
# merges. Progress is ledgered to state/, so a killed tick resumes cleanly.
# ============================================================================
set -uo pipefail

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

BACKEND_SUMMARY="$REPO/docs/audits/backend-robustness/SUMMARY.md"
CLIENT_SUMMARY="$REPO/docs/audits/client-edge-cases/SUMMARY.md"

log() { printf '%s %s\n' "$(date -Is)" "$*" >>"$LOG"; }

exec 9>"$LOCK"
if ! flock -n 9; then
  log "another run in progress — skipping this tick"
  exit 0
fi

if [[ -f "$DONE" ]]; then
  log "fixes already complete — nothing to do (disarm cron; see automation.md)"
  exit 0
fi

# --- wait gate: both audits must be finished ---
if [[ ! -s "$BACKEND_SUMMARY" || ! -s "$CLIENT_SUMMARY" ]]; then
  log "waiting — audits not both complete yet (need backend + client SUMMARY.md)"
  exit 0
fi

# --- ensure isolated worktree exists on the audit-fixes branch ---
if ! git -C "$REPO" worktree list --porcelain 2>/dev/null | grep -q "^worktree $WT$"; then
  log "creating worktree $WT"
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

cd "$REPO" || { log "FATAL: cd $REPO failed"; exit 1; }
log "run start (worktree $WT, branch $BRANCH)"

PROMPT="Run the audit fix-implementer. Call the Workflow tool exactly once with {\"scriptPath\": \"$WF\"}. Do not read files, plan, or do anything else before or after — invoke that single tool and report the JSON it returns. It is resumable and self-checkpointing; it edits and commits ONLY inside the worktree $WT and never merges."

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

if [[ -f "$DONE" ]]; then
  log "run complete — ALL FIXES PROCESSED (review branch $BRANCH; disarm cron)"
else
  log "run complete — progress made, findings still pending"
fi
