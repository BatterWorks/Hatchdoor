#!/usr/bin/env bash
# ============================================================================
# Backend-robustness audit — cross-window auto-resume driver.
#
# Fired by cron on an interval (see automation.md). Each tick it invokes the
# resumable Workflow once via headless `claude -p`. The workflow self-
# checkpoints to state/, so:
#   * budget available -> it advances categories and banks them
#   * 5hr limit hit mid-run -> in-flight category retried next tick, done ones kept
#   * rate-limited outright -> fail-fast, log, exit; retried next tick
#   * all reports + SUMMARY.md present -> no-op, mark complete
# Converges to a finished audit across as many usage windows as it takes.
# ============================================================================
set -uo pipefail

# cron runs with a bare PATH — put claude, the node toolchain (node/npm/npx/
# codegraph), and cargo/rustup back on it so the workflow's subagents can shell out.
export PATH="/home/battermanz/.local/bin:/home/battermanz/.nvm/versions/node/v24.14.0/bin:/home/battermanz/.cargo/bin:$PATH"

DIR="/home/battermanz/coding/hatchdoor/docs/audits/backend-robustness"
STATE="$DIR/state"
REPO="/home/battermanz/coding/hatchdoor"
WF="$STATE/_audit-workflow.js"
LOG="$STATE/driver.log"
LOCK="$STATE/driver.lock"
DONE="$STATE/.audit-complete"

CATS=(
  01-concurrency-shared-state
  02-sqlite-cache-atomicity
  03-git-sync-failure-modes
  04-vault-write-path-safety
  05-mcp-protocol-surface
  06-auth-http-handlers
  07-api-error-shape-seam
)

log() { printf '%s %s\n' "$(date -Is)" "$*" >>"$LOG"; }

is_done() {
  [[ -f "$DIR/SUMMARY.md" ]] || return 1
  local c
  for c in "${CATS[@]}"; do
    [[ -s "$DIR/$c.md" ]] || return 1
    [[ -s "$STATE/$c.verdicts.json" ]] || return 1
  done
  return 0
}

# --- single instance: a long run must not overlap the next tick ---
exec 9>"$LOCK"
if ! flock -n 9; then
  log "another run in progress — skipping this tick"
  exit 0
fi

if [[ -f "$DONE" ]] || is_done; then
  touch "$DONE"
  log "audit already complete — nothing to do (disarm cron; see automation.md)"
  exit 0
fi

cd "$REPO" || { log "FATAL: cd $REPO failed"; exit 1; }
log "run start"

PROMPT="Run the backend robustness audit. Call the Workflow tool exactly once with {\"scriptPath\": \"$WF\"}. Do not read files, plan, explore, or do anything else before or after — invoke that single tool and then report the JSON it returns. The workflow is resumable and self-checkpointing; it will skip categories already on disk."

# Headless, unattended. --dangerously-skip-permissions is required because no
# human is present to approve the Read/Write/Bash the subagents use. Scope is
# contained: reads src/, writes only under docs/audits/backend-robustness/.
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

if is_done; then
  touch "$DONE"
  log "run complete — AUDIT DONE (disarm cron; see automation.md)"
else
  log "run complete — progress made, categories still remain"
fi
