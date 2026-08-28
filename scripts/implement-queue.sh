#!/usr/bin/env bash
#
# Drive /implement across the ready-for-agent queue, one ticket per fresh
# Claude Code process.
#
# /implement already selects its own work: with no argument it takes the oldest
# unassigned, unblocked `ready-for-agent` issue and stops when there is none.
# So this is not a scheduler, it is a driver loop with gates between runs.
# One process per ticket means context resets between tickets, which is the
# `/clear`-between-tickets rule the flow depends on.
#
# Usage:
#   scripts/implement-queue.sh [max_tickets]
#
# Env:
#   BRANCH          branch the work lands on          (default: development)
#   LOG_DIR         per-ticket transcripts            (default: tmp/implement-queue)
#   TICKET_TIMEOUT  wall-clock cap per ticket         (default: 3h)
#   MAX_BUDGET_USD  optional spend cap per ticket     (default: unset)
#   MODEL           model alias for every ticket      (default: opus)
#   EFFORT          effort level                      (default: medium)
#
# Runs with --dangerously-skip-permissions: the agent can run any command
# without prompting. Halts the whole loop on the first failed gate.
#
# Every ticket, finished or failed, is recorded in a TSV ledger under LOG_DIR
# with the gate it died on, its wall time, cost and turn count, so a morning
# review shows what worked and what did not.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

MAX_TICKETS="${1:-20}"
BRANCH="${BRANCH:-development}"
LOG_DIR="${LOG_DIR:-tmp/implement-queue}"
TICKET_TIMEOUT="${TICKET_TIMEOUT:-3h}"
MODEL="${MODEL:-opus}"
EFFORT="${EFFORT:-medium}"
RUN_ID="$(date +%Y%m%d-%H%M%S)"

# The inner agent's Bash tool defaults to a 120s per-command timeout, capped at
# 600s. `cargo test --all` plus the frontend suite can exceed both, and a killed
# validation command reads to the agent as a failure. Raise it for the whole
# unattended run unless the caller has already chosen values.
export BASH_DEFAULT_TIMEOUT_MS="${BASH_DEFAULT_TIMEOUT_MS:-1800000}"
export BASH_MAX_TIMEOUT_MS="${BASH_MAX_TIMEOUT_MS:-1800000}"

mkdir -p "$LOG_DIR"

LEDGER="$LOG_DIR/$RUN_ID-ledger.tsv"
printf 'issue\toutcome\tgate\tseconds\tcost_usd\tturns\tcommit\ttitle\n' > "$LEDGER"

# stderr, so a log line inside eligible_issues can never be captured as a queue entry.
log() { printf '\n[queue %s] %s\n' "$(date +%H:%M:%S)" "$*" >&2; }

# Pull cost/turns off the final stream-json result event. The log interleaves
# stderr, so select the result line rather than parsing the file as a whole.
result_field() {
  local file="$1" field="$2" line
  line="$(grep '"type":"result"' "$file" 2>/dev/null | tail -1)"
  [[ -n "$line" ]] || { echo "-"; return; }
  printf '%s' "$line" | jq -r "(.$field // \"-\") | tostring" 2>/dev/null || echo "-"
}

record() {
  # issue, outcome, gate, seconds, cost, turns, commit, title
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$@" >> "$LEDGER"
}

summary() {
  [[ -s "$LEDGER" ]] || return 0
  log "Ledger: $LEDGER"
  column -t -s $'\t' "$LEDGER" >&2 2>/dev/null || cat "$LEDGER" >&2
}

# Ticket context for the failure path; empty during preflight.
target=""; title=""; ticket_started=0; ticket_log=""

halt() {
  local gate="${2:-preflight}"
  if [[ -n "$target" ]]; then
    record "$target" "FAILED" "$gate" \
      "$(( $(date +%s) - ticket_started ))" \
      "$(result_field "$ticket_log" total_cost_usd)" \
      "$(result_field "$ticket_log" num_turns)" \
      "-" "$title"
  fi
  log "HALTED: $1"
  log "Worktree and issue left as-is for inspection. Logs: $LOG_DIR/$RUN_ID-*.log"
  summary
  exit 1
}

# --- Preflight ---------------------------------------------------------------

command -v claude >/dev/null || halt "claude CLI not on PATH"
command -v gh >/dev/null || halt "gh CLI not on PATH"
gh auth status >/dev/null 2>&1 || halt "gh is not authenticated"

current_branch="$(git rev-parse --abbrev-ref HEAD)"
[[ "$current_branch" == "$BRANCH" ]] || halt "on branch '$current_branch', expected '$BRANCH'"

[[ -z "$(git status --porcelain)" ]] || halt "worktree is dirty; /implement requires a clean tree"

budget_args=()
[[ -n "${MAX_BUDGET_USD:-}" ]] && budget_args=(--max-budget-usd "$MAX_BUDGET_USD")

# A previous halted run leaves its ticket assigned. Assigned issues are excluded
# from the queue, so it would be skipped silently from now on, and anything
# depending on it stays blocked forever. Surface it rather than draining around it.
stranded="$(gh issue list --state open --limit 200 --json number,labels,assignees \
              --jq '.[]
                    | select([.labels[].name] | index("ready-for-agent"))
                    | select((.assignees | length) > 0)
                    | .number' | tr '\n' ' ')"
[[ -z "$stranded" ]] \
  || log "NOTE: ready-for-agent issues already assigned, so not queued: $stranded"

# Eligible = open, ready-for-agent, unassigned, no OPEN blocker.
#
# The label is filtered client-side on purpose: `gh issue list --label
# ready-for-agent` returns [] against this repo even for issues that carry the
# label, so a server-side filter would silently report an empty queue.
eligible_issues() {
  local n
  for n in $(gh issue list --state open --limit 200 \
               --json number,labels,assignees \
               --jq '.[]
                     | select((.assignees | length) == 0)
                     | select([.labels[].name] | index("ready-for-agent"))
                     | .number' | sort -n); do
    # Fail closed: if the dependency API cannot be read (rate limit, network
    # blip, expired auth), treat the issue as blocked. Defaulting to "no
    # blockers" would let a transient failure hand a dependent ticket to
    # /implement ahead of its blocker.
    local open_blockers
    if ! open_blockers="$(gh api "repos/{owner}/{repo}/issues/$n/dependencies/blocked_by" \
                            --jq '[.[] | select(.state == "open")] | length' 2>/dev/null)"; then
      log "WARNING: could not read blockers for #$n; treating it as blocked"
      continue
    fi
    [[ "$open_blockers" == "0" ]] && echo "$n"
  done
}

# --- Loop --------------------------------------------------------------------

completed=0

for (( i = 1; i <= MAX_TICKETS; i++ )); do
  # Re-checked each iteration: a run that ends on a different branch must not launch
  # the next ticket from there.
  now_branch="$(git rev-parse --abbrev-ref HEAD)"
  [[ "$now_branch" == "$BRANCH" ]] || halt "checkout moved to '$now_branch', expected '$BRANCH'"

  mapfile -t queue < <(eligible_issues)

  if (( ${#queue[@]} == 0 )); then
    log "No eligible ready-for-agent tickets left. Done after $completed ticket(s)."
    summary
    exit 0
  fi

  target="${queue[0]}"
  title="$(gh issue view "$target" --json title --jq .title | tr '\t' ' ')"
  head_before="$(git rev-parse HEAD)"
  ticket_started="$(date +%s)"
  log "Ticket $i/$MAX_TICKETS -> #$target: $title"
  log "Remaining eligible: ${queue[*]}"

  ticket_log="$LOG_DIR/$RUN_ID-issue-$target.log"

  # /implement re-selects the same issue itself; passing it explicitly would
  # trip its "already assigned" green-light gate on a retry.
  #
  # `claude -p` has no internal wall-clock cap, so an inner run that wedges
  # (stuck network call, a dev server that never comes healthy) would block the
  # pipeline forever and no gate below would ever run. timeout returns 124.
  timeout --kill-after=60 "$TICKET_TIMEOUT" \
    claude -p "/implement" \
      --model "$MODEL" \
      --effort "$EFFORT" \
      --dangerously-skip-permissions \
      --verbose \
      --output-format stream-json \
      "${budget_args[@]}" \
      2>&1 | tee "$ticket_log"

  status="${PIPESTATUS[0]}"

  # --- Gates --- second argument names the gate for the ledger.
  (( status != 124 && status != 137 )) \
    || halt "#$target: exceeded TICKET_TIMEOUT=$TICKET_TIMEOUT and was killed (see $ticket_log)" timeout
  (( status == 0 )) || halt "#$target: claude exited $status (see $ticket_log)" exit-status

  [[ -z "$(git status --porcelain)" ]] \
    || halt "#$target: worktree left dirty; the run did not finish cleanly" dirty-worktree

  state="$(gh issue view "$target" --json state --jq .state)"
  [[ "$state" == "CLOSED" ]] \
    || halt "#$target: still $state; /implement reported a blocking gate" issue-open

  head_after="$(git rev-parse HEAD)"
  [[ "$head_after" != "$head_before" ]] \
    || halt "#$target: closed but HEAD did not move; no commit was made" no-commit

  git fetch --quiet origin "$BRANCH" || halt "#$target: could not fetch origin/$BRANCH" fetch-failed
  git merge-base --is-ancestor "$head_after" "origin/$BRANCH" \
    || halt "#$target: commit $head_after is not on origin/$BRANCH; push did not land" not-pushed

  record "$target" "OK" "-" \
    "$(( $(date +%s) - ticket_started ))" \
    "$(result_field "$ticket_log" total_cost_usd)" \
    "$(result_field "$ticket_log" num_turns)" \
    "$(git rev-parse --short "$head_after")" "$title"

  completed=$(( completed + 1 ))
  log "#$target done and pushed ($head_after)."
  target=""   # clear, so a later loop-level halt does not re-record this finished ticket
done

log "Reached the $MAX_TICKETS ticket cap. Completed $completed."
summary
