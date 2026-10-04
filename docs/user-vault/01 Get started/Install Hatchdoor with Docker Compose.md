---
tags: [type/tutorial, topic/installation]
---

# Install Hatchdoor with Docker Compose

This page gets Hatchdoor running on your computer. You create two small text files, run one command, copy a password from the output, and pick a search model. It takes about ten minutes plus the model download. You do not need to download Hatchdoor's source code.

You type the commands below in a terminal. On Linux and macOS that is the Terminal app. On Windows, use PowerShell or the WSL terminal.

## 1. Create the deployment folder

Make a new, empty folder for Hatchdoor, for example `hatchdoor` in your home folder, and open your terminal inside it. Everything Hatchdoor keeps about itself (its settings, its search data, the downloaded model) ends up in this folder. Your notes stay wherever they are.

In that folder, create a file named `compose.yaml` with exactly this content:

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

You don't need to change anything in it. In case you are curious:

- The `ports` line means only this computer can open Hatchdoor. Other devices on your network cannot, until you choose to allow it (see [[#Optional: expose Hatchdoor to your LAN]]).
- `HOST: 0.0.0.0` is needed inside the container for Docker to reach Hatchdoor at all. It does not open Hatchdoor to your network; the `ports` line decides that.
- `stop_grace_period: 3m` gives Hatchdoor time to finish a Git sync before it stops. Without it, Docker stops it after 10 seconds, even mid-sync.

## 2. Tell Hatchdoor where your notes are

In the same folder, create a file named `.env` with one line, the full path to the folder that holds your notes:

```env
HOST_VAULT_PATH=/absolute/path/to/your/markdown-vault
```

For example `/home/alex/Notes` on Linux or `/Users/alex/Documents/Notes` on macOS. Hatchdoor can only see this folder and what is inside it. If you put several Vaults inside one parent folder, point `HOST_VAULT_PATH` at the parent and you can add each of them later.

If you leave the line out, Hatchdoor uses a `vault` folder next to `compose.yaml`, which starts empty.

## 3. Prepare the data folders

Hatchdoor runs as a restricted user with the number `65532`, not as you. Create its data folders and hand them to that user before the first start, or Docker creates them owned by the administrator account and Hatchdoor cannot write to them:

```bash
mkdir -p data/cache data/state models
chmod 700 data/cache data/state models
sudo chown -R 65532:65532 data models
```

Hatchdoor also needs to read your notes folder. It needs to write to it only if you want to edit notes in the browser or let an agent change them. On Linux, check that user `65532` can read (and, if you want, write) the folder. Don't change the owner of an existing notes folder without thinking about what else uses it.

## 4. Start Hatchdoor and get your web token

Start it:

```bash
docker compose up -d
```

The first start stops on purpose. Hatchdoor refuses to run without a password for the browser, so it makes one for you and prints it. Show the output with:

```bash
docker compose logs hatchdoor
```

Near the end is a line that contains `HATCHDOOR_WEB_BEARER_TOKEN=` followed by a long random value. Copy that whole `HATCHDOOR_WEB_BEARER_TOKEN=...` part and add it as a new line in `.env`:

```env
HATCHDOOR_WEB_BEARER_TOKEN=paste-the-printed-token-here
```

If you see several such lines, take the last one: Docker retries the start, and each attempt prints a new value. Then start Hatchdoor again:

```bash
docker compose up -d
```

This time it stays running.

> [!warning]
> This is the **web token**, the password for your browser. It is not the password you will give an agent later; that one is made separately in Settings.

## Where do I find my token?

The browser asks for the web token the first time you open Hatchdoor, and again on any new browser or device. It is the value after `HATCHDOOR_WEB_BEARER_TOKEN=` in the `.env` file next to your `compose.yaml`, on the machine that runs Hatchdoor. Copy everything after the `=` sign and paste it into the prompt.

If a browser is already signed in, **Settings** shows it too: choose **Show web access token** under the list of Vaults.

Lost it, or want a new one? Put any long random value after `HATCHDOOR_WEB_BEARER_TOKEN=` in `.env` and run `docker compose up -d`. Every browser then asks for the new token once.

## Choose a search model

Open `http://localhost:42824` in your browser and enter the web token. Hatchdoor first asks which search model to download. The model is what lets Hatchdoor find notes by meaning, not only by exact words.

| Choice | What you get |
| --- | --- |
| **Accept terms and set up Gemma** | Recommended. Searches in many languages, and uses less memory: about 0.5 GB while indexing. You accept Google's Gemma terms to use it. |
| **Use Nomic instead** | English only, and uses more memory: about 1.3 GB while indexing. Choose it if you don't want to accept the Gemma terms. |

Hatchdoor downloads the model you chose, which can take a few minutes, and keeps it in the `models` folder so it never downloads it again.

A fresh install has no Vaults yet, so there is nothing to index. Once the model is ready, Hatchdoor opens the **Set up Hatchdoor** checklist. Continue with [[Connect your first Vault]].

## Optional: expose Hatchdoor to your LAN

By default only the computer running Hatchdoor can open it. If an agent or browser on another device in your home network needs to connect, replace only the `ports` section of `compose.yaml`:

```yaml
    ports:
      - "42824:42824"
```

Do not change `HOST: 0.0.0.0`. After saving the file, run `docker compose up -d` and open `http://<address-of-the-hatchdoor-computer>:42824` from the other device.

> [!warning]
> This lets every device on your network reach Hatchdoor, protected by the web token and the MCP password. Use it only on a network you trust. Never put Hatchdoor directly on the internet. If you need to reach it from outside, put it behind a service that adds its own sign-in and encryption, such as a VPN.

## Using Podman instead of Docker

Everything on this page works with Podman. Use `podman` and `podman compose` wherever it says `docker` and `docker compose`, and change the image to `battermanz/hatchdoor:podman-latest` (or `podman-<version>`); the plain `latest` image is for Docker only.

In step 3, rootless Podman needs `podman unshare chown -R 65532:65532 data models` in place of `sudo chown`. Do the same for a custom `HOST_CACHE_PATH`, `HOST_STATE_PATH` or `HOST_MODELS_PATH`.

## Behind a reverse proxy

Agents download and upload files through short-lived links that carry the server's address. Behind a proxy that adds HTTPS, Hatchdoor learns the address agents used from the proxy's `Forwarded` header, or its `X-Forwarded-Proto` and `X-Forwarded-Host` headers. Caddy, Traefik and Nginx Proxy Manager send them by default. Plain nginx and openresty need `proxy_set_header X-Forwarded-Proto $scheme;` and `proxy_set_header X-Forwarded-Host $http_host;` (`$http_host` keeps a non-standard port, `$host` drops it) in the `location` block. If your proxy cannot send them, or serves Hatchdoor under a path such as `/notes`, set **Public address** in **Settings** → **Agent access (MCP)** instead.

## If your notes are in a Windows folder, on ZFS or on a FUSE mount

Read this if your notes folder is one of these:

- A Windows folder used through Docker Desktop, such as `C:/Users/alex/Documents/Notes`. That is the normal setup on Windows, so it applies to every Windows install.
- ZFS older than OpenZFS 2.2 (Ubuntu 22.04's standard kernel ships 2.1.5). Check with `zfs version` if you are unsure.
- A FUSE mount.

On Linux with ext4, XFS, btrfs or ZFS 2.2 or later you can skip it.

Hatchdoor normally saves a note by writing the new version beside the old one and swapping the two in a single step. That swap is what lets it notice that you also saved the same note in Obsidian at the same moment, and refuse rather than overwrite your change. Those filesystems cannot do the swap.

Your notes still work there: you can edit, move, rename, archive and delete them. Hatchdoor checks the note and then replaces it, in two steps. Saving over a note that changed since it was read is still refused. What you lose is one narrow case: if another program saves the note in the instant between Hatchdoor's check and its replacement, that change is overwritten instead of reported. If you are the only one editing, or use one app at a time, this costs you nothing.

You do not have to set anything. Hatchdoor tests the filesystem when it opens a Vault, writes one line to its log for each Vault that cannot do the swap, and shows the same warning above the note in the browser.

A Windows folder has one more difference. It does not tell Hatchdoor when a note changes from the Windows side, so a note you add or edit in another program shows up within about a minute instead of within seconds. See [[How indexing and search work#What happens when a note changes]].

---

Previous: [[Welcome to Hatchdoor]]
Next: [[Connect your first Vault]]
