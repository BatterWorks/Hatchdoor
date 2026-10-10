# Hatchdoor

**Self-host your Obsidian vaults: a web UI for you, MCP for your AI agents. No Obsidian, no plugins required.**

Hatchdoor reads your vault directly from disk, and any folder of Markdown notes works. No Local REST API plugin, no desktop app that has to stay running. Browse, search, and edit your notes in a fast web UI, and point a Model Context Protocol (MCP) client like Claude, Claude Code, Codex, Cursor, or Hermes at the same vault so your agent can read, search (keyword and semantic), create, edit, move, and link notes. One server manages as many vaults as you give it, every write goes through the same atomic vault operations the UI uses, with optimistic concurrency and optional automatic git commit-and-push. The web UI and your agents are two front doors to one vault.

Your Markdown files stay the source of truth. Hatchdoor builds a disposable SQLite read model it can rebuild from the vault at any time.

The runtime image is **distroless and rootless**. It is built on `gcr.io/distroless/cc-debian13:nonroot`, ships no shell or package manager, and runs as an unprivileged `nonroot` user.

- **Source and full README:** https://github.com/BatterWorks/Hatchdoor
- **Documentation:** https://docs-hatchdoor.battercloud.cc
- **Live demo:** https://hatchdoor.battercloud.cc

## Tags

- `latest` and `<version>`: images for Docker
- `podman-latest` and `podman-<version>`: images for Podman

## Quick start

[Install Hatchdoor with Docker Compose](https://docs-hatchdoor.battercloud.cc/v/bef3df28-8c2e-4722-89ad-bd4d0bcb3def/n/install-hatchdoor-with-docker-compose) explains every step below. This is the short version.

Create `compose.yaml` in an empty folder:

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
      # Safe default: only the Docker host can connect.
      - "127.0.0.1:42824:42824"
    volumes:
      - ${HOST_VAULT_PATH:-./vault}:/data/vault
      - ${HOST_CACHE_PATH:-./data/cache}:/data/cache
      - ${HOST_STATE_PATH:-./data/state}:/data/state
      - ${HOST_MODELS_PATH:-./models}:/models
    restart: unless-stopped
    stop_grace_period: 3m
    healthcheck:
      test: ["CMD", "/app/hatchdoor", "--healthcheck"]
      interval: 30s
      timeout: 5s
      retries: 3
      start_period: 40s
```

Next to it, create `.env` with the full path to the folder that holds your notes:

```env
HOST_VAULT_PATH=/absolute/path/to/your/markdown-vault
```

Hatchdoor runs as user `65532`, not as you. Create its data folders and hand them to that user before the first start:

```bash
mkdir -p data/cache data/state models
chmod 700 data/cache data/state models
sudo chown -R 65532:65532 data models
```

`data/state` holds the list of your Vaults and must survive upgrades. `models` holds the downloaded search model.

Start it, then read the log:

```bash
docker compose up -d
docker compose logs hatchdoor
```

The first start stops on purpose. Hatchdoor refuses to run without a web token, so it makes one and prints it. Near the end of the log is a line that contains `HATCHDOOR_WEB_BEARER_TOKEN=` followed by a long random value. Copy that whole `HATCHDOOR_WEB_BEARER_TOKEN=...` part into `.env` as a new line (take the last one if there are several), then run `docker compose up -d` again. This time it stays running.

Open `http://localhost:42824`, enter the token, and pick a search model. A fresh install has no Vaults: the setup checklist that opens next walks you through adding your notes folder and connecting your agent.

With Podman, use `podman compose` and the `podman-latest` image, and with rootless Podman replace the `sudo chown` line with `podman unshare chown -R 65532:65532 data models`.

The [GitHub README](https://github.com/BatterWorks/Hatchdoor#readme) covers MCP client setup, configuration and every environment variable.

## License

AGPL-3.0-only
