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
    stop_grace_period: 3m
    healthcheck:
      test: ["CMD", "/app/hatchdoor", "--healthcheck"]
      interval: 30s
      timeout: 5s
      retries: 3
      start_period: 40s
```

> [!note]
> `HOST: 0.0.0.0` makes Hatchdoor reachable through Docker's internal network. The `127.0.0.1:42824:42824` port mapping keeps it accessible only from the machine running Docker.

> [!note]
> `stop_grace_period: 3m` lets Hatchdoor finish shutting down cleanly. On `docker compose down` or a restart, it waits for any Git sync that is running to finish or fail. A remote that stops answering is given up on after 15 seconds to connect or 120 seconds without data. Without the setting, Docker waits only 10 seconds and then kills the process mid-sync.

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

> [!note] Behind a reverse proxy
> Agents download and upload files through short-lived links that carry the server's address. Behind a proxy that adds HTTPS, Hatchdoor learns the address agents used from the proxy's `Forwarded` header, or its `X-Forwarded-Proto` and `X-Forwarded-Host` headers. Caddy, Traefik and Nginx Proxy Manager send them by default. Plain nginx and openresty need `proxy_set_header X-Forwarded-Proto $scheme;` and `proxy_set_header X-Forwarded-Host $http_host;` (`$http_host` keeps a non-standard port, `$host` drops it) in the `location` block. If your proxy cannot send them, or serves Hatchdoor under a path such as `/notes`, set **Public address** in **Settings** → **Agent access (MCP)** instead.

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

### A note on the filesystem holding your Vault

Hatchdoor saves a note by writing the new version beside the old one and then
swapping the two in a single step. That swap is what lets it notice you also
saved the note in Obsidian and refuse rather than overwrite your change.

Not every filesystem can do it. ZFS gained the ability in OpenZFS 2.2, and
Ubuntu 22.04's standard kernel ships 2.1.5, so a Vault on ZFS there cannot;
neither can anything mounted through FUSE. ext4, XFS, btrfs and ZFS 2.2 or
later all can. Check with `zfs version` if you are unsure.

A Vault on a filesystem that cannot do the swap still works, and you can still
edit, move, rename, archive and delete notes in it. Hatchdoor falls back to
checking the note and then replacing it as two steps. Saving against a note
that changed under you is still refused. What you lose is the narrow case
where something outside Hatchdoor saves the note in the instant between the
check and the replacement: that change is overwritten instead of reported. If
you are the only one editing, or you always edit through one tool at a time,
this costs you nothing.

You do not have to configure any of this. Hatchdoor tests the filesystem when
it opens a Vault and writes one line to its log for each Vault that cannot do
the swap, saying so in these terms. If the Vault's write settings are shown in
the Web UI, it says so there too.

> [!note]
> Before version 2.7.0 there was no fallback, so a Vault on one of those
> filesystems could create notes but not edit, move or delete them, and the
> failure reached you only as `Invalid argument (os error 22)` in your agent's
> log. If you saw that, upgrading fixes it. Nothing was damaged: those writes
> were refused, not half applied.

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
