#!/usr/bin/env bash
# Follow the README Quick Start against a locally built image, in a throwaway
# folder, with rootless Podman (#569). `just smoke-install` runs it.
#
# Usage: scripts/smoke-install.sh [image]
#
# With no image it builds one from this checkout with Docker, as the release
# does, tags it localhost/hatchdoor-smoke:latest and loads it into Podman,
# which runs it. It keeps that tag in both stores so the next build is warm.
# An image given by name must already be in Podman's store.
# Each step prints its name. On failure the command names the failed step,
# keeps the folder with the container's log in it, and exits non-zero. On
# success it removes the container, its network and the folder.
#
# The docs-freshness table pairs this script with the README Quick Start, the
# Docker Compose install guide and the agent deploy guide: a change to one of
# them names the others. Keep the steps below in the order those pages give.
#
# The search model comes from a copy already on this machine, never from a
# download: the folder holding nomic-embed-text-v1.5 is ./models in this
# checkout (where `just dev-start` keeps it), or HATCHDOOR_SMOKE_MODELS.
#
# The run uses its own container name, Compose project and host port, so it
# can run beside `just dev-start` or a real install. Its Compose override
# carries no `labels:`, so no tool that reads container labels (a reverse
# proxy's discovery, for one) mistakes the run for a real service.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
image=${1:-}
models_source=${HATCHDOOR_SMOKE_MODELS:-$repo/models}
note_title="Smoke install lighthouse"

step="checking what the run needs"
work=""
name="hatchdoor-smoke-$$"
started=0

say() { printf '%s\n' "$*"; }
fail() {
    say "smoke-install: $*" >&2
    exit 1
}
begin() {
    step=$1
    say "==> $step"
}

compose() {
    podman compose -p "$name" -f docker-compose.yml -f smoke-override.yml "$@" \
        >>"$work/compose.log" 2>&1
}

finish() {
    local status=$?
    trap - EXIT INT TERM
    if [ "$status" -eq 0 ]; then
        return
    fi
    say "" >&2
    say "FAILED at step: $step" >&2
    if [ -n "$work" ] && [ -d "$work" ]; then
        if [ "$started" -eq 1 ]; then
            podman logs "$name" >"$work/container.log" 2>&1 || true
            (cd "$work" && compose down --volumes --remove-orphans) || true
        fi
        say "Kept for inspection: $work" >&2
        say "  container log: $work/container.log" >&2
        say "  compose log:   $work/compose.log" >&2
        say "Remove it with: podman unshare rm -rf '$work'" >&2
    fi
    exit "$status"
}
trap finish EXIT
trap 'exit 130' INT TERM

# --- What the run needs ------------------------------------------------------

begin "$step"
command -v podman >/dev/null || fail "podman is not installed."
command -v curl >/dev/null || fail "curl is not installed."
command -v jq >/dev/null || fail "jq is not installed."
command -v openssl >/dev/null || fail "openssl is not installed."
command -v python3 >/dev/null || fail "python3 is not installed."
podman compose version >/dev/null 2>&1 ||
    fail "'podman compose' does not work here: it needs a compose provider."
if [ ! -d "$models_source/nomic-embed-text-v1.5" ]; then
    fail "no copy of the search model in $models_source/nomic-embed-text-v1.5.
This run never downloads it. Fill that folder once (start 'just dev-start' and
pick the English-only model), or point HATCHDOOR_SMOKE_MODELS at a folder that
holds nomic-embed-text-v1.5."
fi

port=$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
base="http://127.0.0.1:$port"

if [ -z "$image" ]; then
    # Docker builds it with the release's build arguments; Podman only runs
    # it (#705). Each run replaces the tag in both stores and removes the image
    # the tag held before, so old builds do not pile up as untagged images.
    begin "building the image with Docker and handing it to Podman"
    command -v docker >/dev/null || fail "docker is not installed."
    image=localhost/hatchdoor-smoke:latest
    old_docker=$(docker image inspect -f '{{.Id}}' "$image" 2>/dev/null || true)
    docker build \
        --build-arg HATCHDOOR_IMAGE=published \
        --build-arg VERSION="$(sed -n 's/^version = "\(.*\)"$/\1/p' "$repo/Cargo.toml" | head -n 1)" \
        --build-arg GIT_SHA="$(git -C "$repo" rev-parse HEAD)" \
        -t "$image" "$repo"
    new_docker=$(docker image inspect -f '{{.Id}}' "$image")
    if [ -n "$old_docker" ] && [ "$old_docker" != "$new_docker" ]; then
        docker image rm "$old_docker" >/dev/null 2>&1 || true
    fi
    if podman image exists "$image"; then
        podman image rm "$image" >/dev/null
    fi
    docker save "$image" | podman load
    podman image exists "$image" || fail "Podman did not receive $image."
else
    podman image exists "$image" || fail "image $image is not in Podman's store."
    # Compose asks Podman through its Docker interface, which reads a short
    # name as a Docker Hub one, so pass the full name Podman stores.
    image=$(podman image inspect -f '{{index .RepoTags 0}}' "$image")
fi

# --- 1. Create your config ---------------------------------------------------

begin "1. creating the config in a new folder"
# Not inside the Cargo target dir, where the justfile points TMPDIR: a failed
# run keeps its folder, whose files belong to Podman's user 65532, and
# `just dev-start` then cannot measure the target dir and stops.
parent=${HATCHDOOR_SMOKE_TMPDIR:-${TMPDIR:-/tmp}}
if [ -z "${HATCHDOOR_SMOKE_TMPDIR:-}" ] && [ -n "${CARGO_TARGET_DIR:-}" ] &&
    [[ "$parent/" == "$CARGO_TARGET_DIR/"* ]]; then
    parent=/tmp
fi
work=$(mktemp -d "$parent/hatchdoor-smoke.XXXXXX")
: >"$work/compose.log"
cd "$work"
cp "$repo/docker-compose.yml" "$repo/.env.example" .
cp .env.example .env
# The model copy goes in before the README hands the folders to user 65532.
mkdir -p data/cache data/state models
cp -a "$models_source/nomic-embed-text-v1.5" models/
chmod 700 data/cache data/state models
podman unshare chown -R 65532:65532 data models

# Not a README step: this run's own name, port and image, and no restart, so a
# refused start stays stopped. `!override` replaces the shipped port rather
# than adding to it.
cat >smoke-override.yml <<EOF
services:
  hatchdoor:
    image: $image
    container_name: $name
    ports: !override
      - "127.0.0.1:$port:42824"
    restart: "no"
EOF

# One known note in the default vault folder, readable by user 65532
# whatever the caller's umask.
mkdir -p vault
printf '# %s\n\nThis note proves an agent can search a fresh install.\n' \
    "$note_title" >"vault/$note_title.md"
chmod 755 vault
chmod 644 "vault/$note_title.md"

# --- 2. Start Hatchdoor: the safe refusal -----------------------------------

begin "2. first start stops safely and saves a web token"
started=1
compose up -d
for _ in $(seq 1 60); do
    state=$(podman inspect -f '{{.State.Status}}' "$name" 2>/dev/null || echo missing)
    [ "$state" = "exited" ] && break
    sleep 1
done
[ "$state" = "exited" ] || fail "the first start did not stop (state: $state)."
podman unshare cat data/state/new-web-token.env 2>/dev/null |
    grep -q '^HATCHDOOR_WEB_BEARER_TOKEN=.' ||
    fail "data/state/new-web-token.env holds no HATCHDOOR_WEB_BEARER_TOKEN= line."

# --- 3. Add the token and start again ---------------------------------------

begin "3. second start with the token in .env"
podman unshare cat data/state/new-web-token.env >>.env
compose up -d
for _ in $(seq 1 120); do
    curl -sf "$base/health" >/dev/null && break
    state=$(podman inspect -f '{{.State.Status}}' "$name" 2>/dev/null || echo missing)
    [ "$state" = "running" ] || [ "$state" = "created" ] ||
        fail "the second start stopped (state: $state) before /health answered."
    sleep 1
done
curl -sf "$base/health" >/dev/null || fail "/health did not answer within 120 seconds."

web_token=$(sed -n 's/^HATCHDOOR_WEB_BEARER_TOKEN=//p' .env | tail -n 1)
api() {
    curl -sf "$@" -H "Authorization: Bearer $web_token"
}

# --- 4. Model, Vault, agent access, one search ------------------------------

begin "4a. choosing the English-only model"
api -X POST "$base/api/model/decline-gemma" >/dev/null

begin "4b. turning on agent access"
mcp_token=$(openssl rand -hex 32)
api -X PATCH "$base/api/settings" -H "Content-Type: application/json" \
    -d '{"updates": {"HATCHDOOR_MCP_ENABLED": "true", "HATCHDOOR_MCP_WRITE_ENABLED": "false", "HATCHDOOR_MCP_BEARER_TOKEN": "'"$mcp_token"'"}}' \
    >/dev/null

begin "4c. adding the Vault"
rev=$(api "$base/api/v1/vaults" | jq -r '.registry_revision')
api -X POST "$base/api/v1/vaults" -H "Content-Type: application/json" \
    -d '{"expected_registry_revision": '"$rev"', "name": "Notes", "source": {"type": "local", "path": "/data/vault"}}' \
    >/dev/null

begin "4d. waiting until the model and the index are ready"
# `/ready` stays 200 once it has been, which can be before the Vault's first
# index when the model loads quickly, so the Vault's own search state counts.
ready=0
for _ in $(seq 1 300); do
    if curl -sf "$base/ready" >/dev/null &&
        api "$base/api/v1/vaults" |
        jq -e '[.. | objects | select(has("local_content") and has("search")) | .search] | any(. == "ready")' \
            >/dev/null 2>&1; then
        ready=1
        break
    fi
    status=$(curl -sf "$base/api/startup-status" || true)
    # `step` reads `download` before the copy is checked too; only a fetch
    # under way reports bytes.
    if printf '%s' "$status" |
        jq -e '.step == "download" and .downloaded_bytes != null' >/dev/null 2>&1; then
        fail "Hatchdoor started downloading the model: the copy in $models_source is not the pinned files."
    fi
    if printf '%s' "$status" | jq -e '.state == "failed"' >/dev/null 2>&1; then
        fail "model setup failed: $(printf '%s' "$status" | jq -r '.message // "see the container log"')"
    fi
    sleep 2
done
[ "$ready" -eq 1 ] || fail "the Vault's search was not ready within 10 minutes."

begin "4e. searching through MCP"
sid=$(curl -s -D - -o /dev/null "$base/mcp" \
    -H "Authorization: Bearer $mcp_token" \
    -H "Content-Type: application/json" \
    -H "Accept: application/json, text/event-stream" \
    -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"hatchdoor-smoke","title":"Smoke install","version":"1"}}}' |
    tr -d '\r' | sed -n 's/^[Mm]cp-[Ss]ession-[Ii]d: //p')
[ -n "$sid" ] || fail "MCP initialize returned no session."
mcp() {
    curl -s "$base/mcp" -H "Authorization: Bearer $mcp_token" -H "Mcp-Session-Id: $sid" \
        -H "Content-Type: application/json" \
        -H "Accept: application/json, text/event-stream" -d "$1"
}
# The answer is one `data:` line holding the JSON-RPC response.
sse_data() { sed -n 's/^data: //p' | tail -n 1; }
mcp '{"jsonrpc":"2.0","method":"notifications/initialized"}' >/dev/null
vault_id=$(mcp '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_vaults","arguments":{}}}' |
    sse_data | jq -r '[.result.structuredContent | .. | .vault_id? // empty][0] // empty')
[ -n "$vault_id" ] || fail "list_vaults shows no Vault."
search=$(mcp '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"search_notes","arguments":{"scope":"'"$vault_id"'","query":"'"$note_title"'"}}}' | sse_data)
printf '%s' "$search" | jq -e '.result.isError == false' >/dev/null ||
    fail "search_notes returned an error."
printf '%s' "$search" |
    jq -e --arg title "$note_title" '[.result.structuredContent | .. | strings] | any(contains($title))' \
        >/dev/null ||
    fail "search_notes did not return the note '$note_title'."

# --- 5. Remove everything ----------------------------------------------------

begin "5. removing the container, its network and the folder"
compose down --volumes --remove-orphans
if podman container exists "$name"; then
    fail "container $name is still there."
fi
cd /
podman unshare rm -rf "$work"
[ ! -e "$work" ] || fail "could not remove $work."
work=""
say "smoke-install passed: the README Quick Start works with image $image."
