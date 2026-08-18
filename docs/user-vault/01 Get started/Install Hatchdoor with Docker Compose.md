---
tags: [type/tutorial, topic/installation]
---

# Install Hatchdoor with Docker Compose

Create an empty directory for this deployment and work inside it. You do not
need a Hatchdoor source checkout.

Create `compose.yaml` with this complete content:

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
      # Safe default: only the Docker host can connect. See the LAN option below.
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

> [!note]
> `HOST: 0.0.0.0` makes Hatchdoor reachable through Docker's internal network. The `127.0.0.1:42824:42824` port mapping keeps it accessible only from the machine running Docker.

> [!tip]
> Using Podman instead of Docker? Everything on this page works unchanged — swap `docker` / `docker compose` for `podman` / `podman compose`, but also change the image to `battermanz/hatchdoor:podman-latest` (or `podman-<version>`); the plain `latest` tag above is Docker-only. The `chown` step further down needs `podman unshare` too — see the note there.

## Optional: expose Hatchdoor to your LAN

If an agent or browser on another trusted device needs to connect, replace only the `ports` section above:

```yaml
    ports:
      - "42824:42824"
```

Do not change `HOST: 0.0.0.0`; Hatchdoor needs that value inside the container. After saving the file, run `docker compose up -d` and connect to `http://<hatchdoor-server-address>:42824`.

> [!warning]
> Publishing `42824:42824` listens on every host interface. Use it only on a trusted LAN with the web and MCP passwords enabled. For internet access, put Hatchdoor behind an authenticated, encrypted access layer instead.

Create `.env` with your host-side Vault path:

```env
HOST_VAULT_PATH=/absolute/path/to/your/markdown-vault
```

Leave the other paths unset to use `./data/cache`, `./data/state`, and
`./models` beside `compose.yaml`. You do not need `.env.example` for this
deployment.

Prepare the writable deployment directories before first start. The image runs
as the numeric `nonroot` user, so Docker must not create these bind sources as
root:

```bash
mkdir -p data/cache data/state models
chmod 700 data/cache data/state models
sudo chown -R 65532:65532 data models
```

> [!tip]
> On rootless Podman, `sudo chown` targets the wrong namespace — use
> `podman unshare chown -R 65532:65532 data models` instead. Apply the same
> substitution to a custom `HOST_CACHE_PATH`, `HOST_STATE_PATH`, or
> `HOST_MODELS_PATH`.

The container also needs read access to your Vault. Grant it write access only
if agents or the Web UI should change notes. On Linux, verify access for UID
`65532` without blindly changing ownership of an existing Vault.

Start Hatchdoor:

```bash
docker compose up -d
```

Compose publishes port `42824` on the host's loopback interface. Hatchdoor
still sees its container-side non-loopback listener and correctly refuses the
first run until browser access has a token. Retrieve the token:

```bash
docker compose logs hatchdoor
```

Add the printed assignment to `.env`, then start again:

```env
HATCHDOOR_WEB_BEARER_TOKEN=paste-the-printed-token-here
```

```bash
docker compose up -d
```

> [!warning]
> This is the **web token**. It protects the browser and is not the password you will give an agent later.

Open `http://localhost:42824` and enter the web token. On the first-run screen,
choose a search model:

| Choice | When to choose it |
| --- | --- |
| **Accept terms and set up Gemma** | Recommended; multilingual search |
| **Use Nomic instead** | You decline Gemma terms; English-only search |

Hatchdoor downloads the selected model and indexes the Vault. Wait until setup
is ready, then continue with [[Connect your first Vault]].

---

Previous: [[Welcome to Hatchdoor]]
Next: [[Connect your first Vault]]
