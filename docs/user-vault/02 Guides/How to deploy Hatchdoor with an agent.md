---
tags: [type/how-to, topic/deployment, topic/mcp]
---

# How to deploy Hatchdoor with an agent

Use this guide when an agent with shell and HTTP access should stand up Hatchdoor on a machine it controls, without a human clicking through the first-run screen. It assumes Docker Compose and produces a running instance with MCP enabled and one Vault ready to search.

> [!note]
> This is the unattended path. If a person is doing the deployment by hand, follow [[Install Hatchdoor with Docker Compose]], [[Connect your first Vault]], and [[Connect your agent]] instead — they cover the same ground with screenshots and explanation.

## Before you start

Decide the Vault source:



- **Local** — Markdown already sits in a folder Docker can mount.
- **Managed Git** — Hatchdoor should clone and keep a remote repository's Markdown in sync (`pull_only`) or also push local commits back (`two_way`).

Both are covered in step 5; pick one before you begin.

## 1. Write the deployment files

Create an empty deployment directory and, inside it, `compose.yaml`:

```yaml
services:
  hatchdoor:
    image: battermanz/hatchdoor:latest
    container_name: hatchdoor
    env_file:
      - .env
    environment:
      HOST: 0.0.0.0
      PORT: "42824"
      VAULT_PATH: /data/vault
      HATCHDOOR_CACHE_DB: /data/cache/hatchdoor-cache.sqlite3
    ports:
      - "127.0.0.1:42824:42824"
    volumes:
      - ${HOST_VAULT_PATH:-./vault}:/data/vault
      - ${HOST_CACHE_PATH:-./data/cache}:/data/cache
      - ${HOST_STATE_PATH:-./data/state}:/data/state
      - ${HOST_MODELS_PATH:-./models}:/models
    restart: unless-stopped
    healthcheck:
      test: ["CMD", "/app/hatchdoor", "--healthcheck"]
      interval: 30s
      timeout: 5s
      retries: 3
      start_period: 40s
```

> [!warning]
> `HOST: 0.0.0.0` is required — it is the container's own listener, not a public exposure setting. From Hatchdoor's point of view this is **never** a loopback address, regardless of whether the `ports` mapping above is restricted to `127.0.0.1`. That has one consequence you cannot skip: see step 2.

Generate a web token and a Vault directory before the first start, so the container does not need a fail-then-restart cycle:

```bash
mkdir -p data/cache data/state models vault
chmod 700 data/cache data/state models
sudo chown -R 65532:65532 data models vault

WEB_TOKEN=$(openssl rand -hex 32)
cat > .env <<EOF
HATCHDOOR_WEB_BEARER_TOKEN=${WEB_TOKEN}
HOST_VAULT_PATH=$(pwd)/vault
EOF
```

If you are pointing at a Local Vault that already has Markdown, set `HOST_VAULT_PATH` to that folder instead of the empty one created above.

## 2. Start Hatchdoor

```bash
docker compose up -d
```

Wait for the health check:

```bash
until docker compose ps --format json | grep -q '"Health":"healthy"'; do sleep 2; done
```

> [!warning]
> Because `HOST` can never be loopback inside the container (step 1), Hatchdoor's own startup check (`check_web_auth`) treats this as a public bind and refuses to run without `HATCHDOOR_WEB_BEARER_TOKEN` set. Pre-generating the token, as above, means the first start already succeeds. If you skip that step, the container exits immediately and prints a freshly generated token to its logs (`docker compose logs hatchdoor`) for you to add to `.env` before starting again.

## 3. Turn on MCP, without a restart

MCP is off by default and is read live from settings on every request — no restart is needed once it is turned on. Generate a separate MCP token (never reuse the web token) and patch it in:

```bash
MCP_TOKEN=$(openssl rand -hex 32)

curl -sf -X PATCH http://127.0.0.1:42824/api/settings \
  -H "Authorization: Bearer ${WEB_TOKEN}" \
  -H "Content-Type: application/json" \
  -d '{
    "updates": {
      "HATCHDOOR_MCP_ENABLED": "true",
      "HATCHDOOR_MCP_WRITE_ENABLED": "true",
      "HATCHDOOR_MCP_BEARER_TOKEN": "'"${MCP_TOKEN}"'"
    }
  }'
```

> [!warning]
> Do not set `HATCHDOOR_DEMO_MODE=true` on a deployment an agent needs to manage. Hatchdoor refuses to start at all with demo mode and MCP enabled together — demo mode is for a public, read-only instance nobody configures further, never for this flow.

From here on, address `http://127.0.0.1:42824/mcp` with `Authorization: Bearer ${MCP_TOKEN}` as an MCP Streamable HTTP client.

## 4. Finish first-run model setup

Call `get_model_setup_status`. If setup is still pending, call `accept_gemma_terms` (recommended, multilingual) or `decline_gemma_terms` (English-only, lower memory). Poll `get_model_setup_status` until the model has finished downloading.

## 5. Create the Vault

Call `list_vaults` first — every write below needs its `expected_registry_revision`.

**Local Vault**, for Markdown already mounted into the container:

```json
{
  "name": "create_vault",
  "arguments": {
    "expected_registry_revision": 0,
    "name": "Primary",
    "source": {
      "type": "local",
      "path": "/data/vault"
    }
  }
}
```

**Managed Git Vault**, for a remote repository Hatchdoor should clone and keep in sync:

```json
{
  "name": "create_vault",
  "arguments": {
    "expected_registry_revision": 0,
    "name": "Primary",
    "source": {
      "type": "managed_git",
      "repository_url": "https://github.com/<owner>/<repo>.git",
      "branch": "main",
      "vault_subdirectory": "notes",
      "mode": "pull_only",
      "poll_interval_secs": 900
    }
  }
}
```

Add `https_credentials` alongside `source` if the repository is private; omit it for a public repository. Use `mode: "two_way"` only if this Vault should also push the agent's own commits back to the remote.

## 6. Confirm it is ready

Poll `list_vaults` or `get_stats` until the Vault's phase reaches `ready`. Then run one read to prove the path works end to end:

```text
search_notes with a term you expect to match, then get_note on the best result.
```

> [!success]
> Hatchdoor is deployed, MCP is live, and the Vault is searchable — all without a human opening the browser. Keep the Web UI available anyway: `http://127.0.0.1:42824` with the web token from step 1 is how a person audits what the agent has done.

---

Related: [[Install Hatchdoor with Docker Compose]] · [[Connect your first Vault]] · [[Connect your agent]]
