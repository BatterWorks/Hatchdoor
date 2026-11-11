# Dev server lifecycle. See AGENTS.md for why this exists instead of raw
# `cargo run` / `npm run dev`.

# Share build artifacts across linked worktrees through the primary checkout.
# Explicit environment values still win for hosts with a dedicated build disk.
git_common_parent := `dirname "$(git rev-parse --path-format=absolute --git-common-dir)"`
cargo_target_dir := env_var_or_default("CARGO_TARGET_DIR", git_common_parent + "/target")
cargo_home := env_var_or_default("CARGO_HOME", env_var("HOME") + "/.cargo")
cargo_tmp_dir := env_var_or_default("HATCHDOOR_TMPDIR", cargo_target_dir + "/tmp")
backend_port := "42824"
frontend_port := "5173"
browser_token_port := "42825"
dev_dir := ".dev"
target_warn_gb := "20"

# Dev-only Vault collection registry. The deployed default (/data/state) does
# not exist outside the container, so local dev keeps its own alongside the
# pid/log files. See scripts/dev-vaults.sh for the fixture profiles.
export HATCHDOOR_VAULT_REGISTRY_PATH := justfile_directory() + "/" + dev_dir + "/state/vaults.json"

# Export the resolved values so recipes behave consistently even when the
# invoking shell does not set Cargo paths. TMPDIR stays off small tmpfs mounts.
export CARGO_TARGET_DIR := cargo_target_dir
export CARGO_HOME := cargo_home
export TMPDIR := cargo_tmp_dir

default:
    @just --list

# Start the backend (cargo run) and frontend (vite, hot reload) in the
# background. Always safe to re-run: kills any previous instance first, so
# you never end up with two copies fighting over the same port.
dev-start profile="": _prepare-cargo _kill-stale
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p {{dev_dir}}

    # An explicit profile reprovisions; otherwise reuse whatever is already
    # there, and provision 'clean' on a first run so there is always a registry.
    if [ -n "{{profile}}" ]; then
        scripts/dev-vaults.sh "{{profile}}"
    elif [ ! -f "$HATCHDOOR_VAULT_REGISTRY_PATH" ]; then
        scripts/dev-vaults.sh clean
    else
        echo "vault profile: $(cat {{dev_dir}}/vaults-profile 2>/dev/null || echo unknown) (just dev-vaults <profile> to switch)"
    fi

    if [ -d "$CARGO_TARGET_DIR" ]; then
        size_kb=$(du -sk "$CARGO_TARGET_DIR" 2>/dev/null | cut -f1)
        size_gb=$(( size_kb / 1024 / 1024 ))
        if [ "$size_gb" -ge {{target_warn_gb}} ]; then
            echo "warning: $CARGO_TARGET_DIR is ${size_gb}G (>= {{target_warn_gb}}G) - run 'just dev-clean' to reclaim space" >&2
        fi
    fi

    {{just_executable()}} _start-backend

    echo "starting frontend (npm run dev)..."
    cd frontend
    setsid npm run dev -- --host 0.0.0.0 --port {{frontend_port}} --strictPort > ../{{dev_dir}}/frontend.log 2>&1 &
    echo $! > ../{{dev_dir}}/frontend.pid
    cd ..

    sleep 1
    echo
    echo "backend log:  {{dev_dir}}/backend.log   (http://127.0.0.1:{{backend_port}}, compiling takes a bit)"
    echo "frontend log: {{dev_dir}}/frontend.log  (http://0.0.0.0:{{frontend_port}})"
    echo "'just dev-wait' returns once both answer, 'just dev-status' to check, 'just dev-stop' to stop"

# Rebuild the dev Vault fixtures under .dev/vaults and rewrite the registry.
# Profiles: clean (one healthy Vault), messy (healthy + pathological content +
# every degraded state), broken (degraded states only), demo (the four public
# demo-vaults/* side by side). Destroys and recreates the fixture tree, so
# never point this at a Vault you care about.
dev-vaults profile="clean":
    @scripts/dev-vaults.sh "{{profile}}"

# Reprovision the profile currently in use, discarding any local edits made to
# the fixtures while poking at them.
dev-vaults-reset:
    @scripts/dev-vaults.sh "$(cat {{dev_dir}}/vaults-profile 2>/dev/null || echo clean)"

# Stop the tracked backend/frontend, whole process group (catches vite's
# npm -> sh -> node child chain, not just the top PID).
dev-stop: _kill-stale _stop-browser-token
    @echo "stopped"

# Fails naming the half that did not answer, at once when that half is not
# running at all. The time limit is in seconds (scripts/dev-wait.mjs).
#
# Wait until the backend's health endpoint and the frontend both answer.
dev-wait seconds="600":
    @node scripts/dev-wait.mjs --backend-port {{backend_port}} --frontend-port {{frontend_port}} --dev-dir "{{dev_dir}}" "{{seconds}}"

# For a check of what the app shows while the backend is away. The frontend
# keeps running, and `just dev-status` reports the backend as not running.
#
# Stop the backend alone; `just dev-restart-backend` brings it back.
dev-stop-backend: (_stop-tracked "backend" backend_port)
    @echo "backend stopped"

# Through the same process-group handling as `dev-stop`, never a bare
# `cargo run`. Returns once the backend's health endpoint answers.
#
# Stop the backend, start it again, and wait for it. The frontend stays up.
dev-restart-backend: _prepare-cargo (_stop-tracked "backend" backend_port)
    #!/usr/bin/env bash
    set -euo pipefail
    if [ ! -f "$HATCHDOOR_VAULT_REGISTRY_PATH" ]; then
        echo "error: there are no dev Vault fixtures yet; 'just dev-start' makes them and starts both halves" >&2
        exit 1
    fi
    {{just_executable()}} _start-backend
    node scripts/dev-wait.mjs --only backend --backend-port {{backend_port}} --dev-dir "{{dev_dir}}"

# Returns once the note is indexed, printing its slug and its address on the
# dev frontend. <vault> is a fixture's folder (healthy, readonly), name or id.
# Refuses a path that is already there; read-only Vaults included. Add
# `--timeout <seconds>` for a Vault still building its first index
# (scripts/dev-note.mjs).
#
# Place a test note in a dev fixture Vault: <vault> <file or -> [path in the Vault].
[positional-arguments]
dev-note *args:
    @node scripts/dev-note.mjs --backend-port {{backend_port}} --frontend-port {{frontend_port}} --dev-dir "{{justfile_directory()}}/{{dev_dir}}" "$@"

# Does the whole handshake and never prints the token. `just mcp-call -`
# reads one `<tool> <json arguments>` a line and makes them all in one
# session (scripts/mcp-call.mjs). Talks to 127.0.0.1 only.
#
# Call one MCP tool on the local dev server and print its result as JSON.
mcp-call $tool $arguments="{}":
    @node scripts/mcp-call.mjs --port {{backend_port}} "${tool}" "${arguments}"

# Serves the web token on 127.0.0.1, to the dev frontend's pages only, so it
# never passes through a tool call or a transcript. Stops by itself after 15
# minutes. The key in the printed line is TOKEN_KEY in frontend/src/api/api.ts.
#
# Sign a test browser in to the dev app; run the line it prints in the page.
dev-browser-token: _stop-browser-token
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p {{dev_dir}}
    setsid python3 scripts/dev-browser-token.py --port {{browser_token_port}} \
        --origin http://127.0.0.1:{{frontend_port}} --origin http://localhost:{{frontend_port}} \
        > {{dev_dir}}/browser-token.log 2>&1 &
    for _ in 1 2 3 4 5 6 7 8 9 10; do
        fuser {{browser_token_port}}/tcp >/dev/null 2>&1 && break
        sleep 0.3
    done
    if ! fuser {{browser_token_port}}/tcp >/dev/null 2>&1; then
        cat {{dev_dir}}/browser-token.log >&2
        exit 1
    fi
    echo "open http://127.0.0.1:{{frontend_port}} in the test browser, then run this in the page:"
    echo "  localStorage.setItem('hatchdoor_web_token', await (await fetch('http://127.0.0.1:{{browser_token_port}}/')).text()); location.reload()"
    echo "'just dev-browser-token-stop' when the check is done"

# Stop the helper `dev-browser-token` started.
dev-browser-token-stop: _stop-browser-token
    @echo "stopped"

# Start a prototype for a `needs-a-choice` ticket (#602): .dev/prototypes/<name>/
# holding each path as the dev instance draws it today, at phone, tablet and
# desktop size in light and dark, as static pages with no scripts, beside the
# stylesheet the app ships and an index. <name> is <issue>-<slug>, as in
# `just proto-new 583-vaults-unavailable /`. `just dev-start` comes first. A
# path with `?` in it is quoted. Run it again with more paths to add to it.
proto-new $name +$paths:
    #!/usr/bin/env bash
    set -euo pipefail
    # The paths are split on spaces and nothing more: a `?` or `&` in one is
    # part of the address, not a pattern.
    set -f
    node scripts/proto-new.mjs --origin http://127.0.0.1:{{frontend_port}} "${name}" ${paths}

# Serve a prototype to the LAN with caching off, and print its address. Stays
# in the foreground until stopped. <name> is a folder under .dev/prototypes/
# and nothing else, since the whole LAN can read what it serves. The port must
# be one this host lets through its firewall.
proto-serve $name $port:
    @node scripts/proto-serve.mjs "${name}" "${port}"

# By port, like the dev ports below: nothing to go stale once the helper has
# stopped by itself.
_stop-browser-token:
    #!/usr/bin/env bash
    set -uo pipefail
    fuser -k -TERM {{browser_token_port}}/tcp >/dev/null 2>&1 || true
    for _ in 1 2 3 4 5 6 7 8 9 10; do
        fuser {{browser_token_port}}/tcp >/dev/null 2>&1 || break
        sleep 0.2
    done

_prepare-cargo:
    @mkdir -p "$CARGO_TARGET_DIR" "$TMPDIR"

# Kill whatever dev-start is tracking, plus anything else bound to our dev
# ports even if it predates this system (e.g. a server started by hand).
_kill-stale: (_stop-tracked "backend" backend_port) (_stop-tracked "frontend" frontend_port)

# One half: the whole process group of the tracked pid (catches vite's
# npm -> sh -> node child chain, and the binary under `cargo run`), then
# whatever else holds the port. TERM first; KILL only for what is still
# there after five seconds. The pid file goes, so `dev-status` tells the truth.
_stop-tracked name port:
    #!/usr/bin/env bash
    set -uo pipefail
    pidfile="{{dev_dir}}/{{name}}.pid"
    if [ -f "$pidfile" ]; then
        pid=$(cat "$pidfile")
        alive() { kill -0 -- "-$pid" 2>/dev/null || kill -0 "$pid" 2>/dev/null; }
        if alive; then
            echo "stopping tracked {{name}} (pid $pid)"
            kill -s TERM -- "-$pid" 2>/dev/null || kill -s TERM "$pid" 2>/dev/null || true
            for _ in $(seq 1 25); do
                alive || break
                sleep 0.2
            done
            if alive; then
                echo "{{name}} ignored TERM for five seconds, killing it"
                kill -s KILL -- "-$pid" 2>/dev/null || kill -s KILL "$pid" 2>/dev/null || true
            fi
        fi
        rm -f "$pidfile"
    fi
    fuser -k -TERM {{port}}/tcp >/dev/null 2>&1 || true
    for _ in $(seq 1 25); do
        fuser {{port}}/tcp >/dev/null 2>&1 || break
        sleep 0.2
    done
    fuser -k -KILL {{port}}/tcp >/dev/null 2>&1 || true

# The backend half of `dev-start`, in its own process group and tracked by
# pid, so `_stop-tracked` can stop all of it.
_start-backend: _prepare-cargo
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p {{dev_dir}}
    echo "starting backend (cargo run)..."
    setsid cargo run > {{dev_dir}}/backend.log 2>&1 &
    echo $! > {{dev_dir}}/backend.pid

# Check what's running and how big the build cache has grown.
dev-status:
    #!/usr/bin/env bash
    set -uo pipefail
    for name in backend frontend; do
        pidfile="{{dev_dir}}/${name}.pid"
        if [ -f "$pidfile" ] && kill -0 "$(cat "$pidfile")" 2>/dev/null; then
            pid=$(cat "$pidfile")
            started=$(ps -o lstart= -p "$pid" 2>/dev/null | xargs)
            echo "${name}: running (pid $pid, started $started)"
        else
            echo "${name}: not running"
        fi
    done
    if fuser {{browser_token_port}}/tcp >/dev/null 2>&1; then
        echo "browser-token: running (just dev-browser-token-stop to stop)"
    fi
    if [ -d "$CARGO_TARGET_DIR" ]; then
        echo "cargo target dir ($CARGO_TARGET_DIR): $(du -sh "$CARGO_TARGET_DIR" 2>/dev/null | cut -f1)"
    fi

# Reclaim space in the cargo target dir. Next build will be a full rebuild.
dev-clean: _prepare-cargo
    cargo clean

# The checks to run before a pull request: the repository's own scripts,
# formatting, lints for both the shipped and the all-features build, the
# backend tests once, and the frontend. Skips the tests that load real model
# weights. See CONTRIBUTING.md.
#
# A branch that changed only documentation runs the script checks alone; one
# that also edited manual pages adds the tests that read the manual. A tree
# that already passed is not checked again. The first line names the mode
# (scripts/check-mode.mjs).
#
# The test lines drop HATCHDOOR_VAULT_REGISTRY_PATH, which this file exports
# for the dev server: the tests must see the deployed default, not .dev/.
check: _prepare-cargo
    #!/usr/bin/env bash
    set -euo pipefail
    status=0
    plan=$(node scripts/check-mode.mjs start) || status=$?
    if [ "$status" -eq 10 ]; then exit 0; fi
    if [ "$status" -ne 0 ]; then exit "$status"; fi
    read -r mode tree <<<"$plan"
    {{just_executable()}} _check-scripts
    if [ "$mode" = full ]; then
        {{just_executable()}} _check-static
        env -u HATCHDOOR_VAULT_REGISTRY_PATH cargo test --all --features eval
        {{just_executable()}} _check-frontend
    elif [ "$mode" = manual ]; then
        mapfile -t rust < <(node scripts/check-mode.mjs manual-tests rust)
        mapfile -t front < <(node scripts/check-mode.mjs manual-tests frontend)
        env -u HATCHDOOR_VAULT_REGISTRY_PATH cargo test --features eval --lib -- "${rust[@]}"
        (cd frontend && npx vitest run "${front[@]}")
    fi
    node scripts/check-mode.mjs record "$mode" "$tree"

# Everything `check` covers, plus the backend tests in the exact configuration
# a deployment ships and the tests that load real model weights (the first run
# downloads them from Hugging Face).
check-full: _check-scripts _check-static && _check-frontend
    env -u HATCHDOOR_VAULT_REGISTRY_PATH cargo test --all
    env -u HATCHDOOR_VAULT_REGISTRY_PATH cargo test --all --all-features

# Seconds, so they run first: the module map's structural coverage, the
# docs-freshness table, and the tests of everything under scripts/. Those
# tests build fixtures under TMPDIR, which _prepare-cargo creates.
_check-scripts: _prepare-cargo
    node scripts/check-module-map.mjs
    node scripts/check-docs-freshness.mjs --validate-table
    node --test scripts/*.test.mjs

_check-static: _prepare-cargo
    cargo fmt --all -- --check
    cargo clippy --all-targets -- -D warnings
    cargo clippy --all-targets --all-features -- -D warnings

_check-frontend:
    cd frontend && npm run format:check
    cd frontend && npm run lint
    cd frontend && npm run typecheck
    cd frontend && npm test
    cd frontend && npm run build

# Exits non-zero so the review cannot be skipped silently. Pass a different
# base with `just docs-freshness main`. Also fails when shipped code changed
# with no CHANGELOG.md edit and no `Changelog: none, <reason>` trailer.
#
# Before merging into development: are the user-vault notes and changelog fresh?
docs-freshness base="development":
    node scripts/check-docs-freshness.mjs --base '{{base}}'

# Only run this after actually reading the notes it named. It does not waive
# a missing changelog entry.
#
# Record that the documentation freshness review happened.
docs-freshness-ack base="development":
    node scripts/check-docs-freshness.mjs --base '{{base}}' --acknowledge

# Opens pull requests and a draft release; never merges. Re-run it after the
# version-bump pull request merges to open the release pull request. See
# ADR-36.
#
# Prepare a release: bump the version, then open the release pull request.
release-prepare version:
    node scripts/release-prepare.mjs '{{version}}'

# Run only after the maintainer approved the release title and notes in this
# session. Needs HATCHDOOR_RELEASE_HOOK; re-run it to resume after a failure.
# See ADR-36.
#
# Publish a release: merge, tag, build images, publish, deploy.
release-publish version:
    node scripts/release-publish.mjs '{{version}}'

# Takes minutes, so it runs at release and on demand, never in `check`.
# Without an image it builds localhost/hatchdoor-smoke:latest from this
# checkout. See scripts/smoke-install.sh and CONTRIBUTING.md.
#
# Build with Docker, then follow the README Quick Start in rootless Podman.
smoke-install image="": _prepare-cargo
    scripts/smoke-install.sh {{ if image == "" { "" } else { quote(image) } }}

# Build the real frontend bundle and serve it from the backend on one port -
# exactly what production runs. Foreground; Ctrl+C to stop. No hot reload.
prod-check: _prepare-cargo
    #!/usr/bin/env bash
    set -euo pipefail
    echo "building frontend..."
    (cd frontend && npm run build)
    echo "starting backend in foreground (serves frontend/dist) - Ctrl+C to stop"
    cargo run
