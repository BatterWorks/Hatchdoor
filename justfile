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
dev_dir := ".dev"
target_warn_gb := "20"

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
dev-start: _prepare-cargo _kill-stale
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p {{dev_dir}}

    if [ -d "$CARGO_TARGET_DIR" ]; then
        size_kb=$(du -sk "$CARGO_TARGET_DIR" 2>/dev/null | cut -f1)
        size_gb=$(( size_kb / 1024 / 1024 ))
        if [ "$size_gb" -ge {{target_warn_gb}} ]; then
            echo "warning: $CARGO_TARGET_DIR is ${size_gb}G (>= {{target_warn_gb}}G) - run 'just dev-clean' to reclaim space" >&2
        fi
    fi

    echo "starting backend (cargo run)..."
    setsid cargo run > {{dev_dir}}/backend.log 2>&1 &
    echo $! > {{dev_dir}}/backend.pid

    echo "starting frontend (npm run dev)..."
    cd frontend
    setsid npm run dev -- --host 0.0.0.0 --port {{frontend_port}} --strictPort > ../{{dev_dir}}/frontend.log 2>&1 &
    echo $! > ../{{dev_dir}}/frontend.pid
    cd ..

    sleep 1
    echo
    echo "backend log:  {{dev_dir}}/backend.log   (http://127.0.0.1:{{backend_port}}, compiling takes a bit)"
    echo "frontend log: {{dev_dir}}/frontend.log  (http://0.0.0.0:{{frontend_port}})"
    echo "'just dev-status' to check, 'just dev-stop' to stop"

# Stop the tracked backend/frontend, whole process group (catches vite's
# npm -> sh -> node child chain, not just the top PID).
dev-stop: _kill-stale
    @echo "stopped"

_prepare-cargo:
    @mkdir -p "$CARGO_TARGET_DIR" "$TMPDIR"

# Kill whatever dev-start is tracking, plus anything else bound to our dev
# ports even if it predates this system (e.g. a server started by hand).
_kill-stale:
    #!/usr/bin/env bash
    set -uo pipefail
    for name in backend frontend; do
        pidfile="{{dev_dir}}/${name}.pid"
        if [ -f "$pidfile" ]; then
            pid=$(cat "$pidfile")
            if kill -0 "$pid" 2>/dev/null; then
                echo "stopping tracked ${name} (pid $pid)"
                kill -s TERM -- "-$pid" 2>/dev/null || kill -s TERM "$pid" 2>/dev/null || true
                sleep 1
                kill -0 "$pid" 2>/dev/null && { kill -s KILL -- "-$pid" 2>/dev/null || kill -s KILL "$pid" 2>/dev/null || true; }
            fi
            rm -f "$pidfile"
        fi
    done
    fuser -k -TERM {{backend_port}}/tcp 2>/dev/null || true
    fuser -k -TERM {{frontend_port}}/tcp 2>/dev/null || true
    sleep 1
    true

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
    if [ -d "$CARGO_TARGET_DIR" ]; then
        echo "cargo target dir ($CARGO_TARGET_DIR): $(du -sh "$CARGO_TARGET_DIR" 2>/dev/null | cut -f1)"
    fi

# Reclaim space in the cargo target dir. Next build will be a full rebuild.
dev-clean: _prepare-cargo
    cargo clean

# Build the real frontend bundle and serve it from the backend on one port -
# exactly what production runs. Foreground; Ctrl+C to stop. No hot reload.
prod-check: _prepare-cargo
    #!/usr/bin/env bash
    set -euo pipefail
    echo "building frontend..."
    (cd frontend && npm run build)
    echo "starting backend in foreground (serves frontend/dist) - Ctrl+C to stop"
    cargo run
