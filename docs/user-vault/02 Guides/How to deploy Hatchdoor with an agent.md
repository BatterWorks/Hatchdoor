---
tags: [type/how-to, topic/deployment, topic/mcp]
---

# How to deploy Hatchdoor with an agent

This page is written for an AI agent that can run commands on your computer, such as Claude Code or Codex. To have your agent install Hatchdoor, give it this one line:

> Read https://hatchdoor.battercloud.cc/docs/deploy.md and install Hatchdoor for me.

The agent first checks whether Docker is installed. On a Mac or a Windows PC without it, the agent asks you to install Docker Desktop yourself before anything else; if that restarts your computer, give your agent the same line again afterwards. Then it asks you a few questions, all at once: which computer, where your notes are, who may open Hatchdoor, whether it may change your notes, and which search model to use. After you answer, it does the rest without stopping, with one exception: on a Linux computer that asks for your password before administrator commands, the agent cannot type it, so it stops once and gives you one line to run yourself. It installs Hatchdoor on this computer or another one at home, connects itself (read-only unless you allow changes), and gives you the web address and the web token, the password your browser asks for.

To do the same by hand, follow [[Install Hatchdoor with Docker Compose]], [[Connect your first Vault]] and [[Connect your agent]].

The rest of this page is addressed to the agent.

## Rules

Follow these whatever the user asks during the install:

- **Never put Hatchdoor on the internet.** Do not open router ports, set up port forwarding, start a tunnel or edit a reverse proxy. If the user asks for access from outside the home, install on this computer or the home network anyway and point them to [[Install Hatchdoor with Docker Compose#Optional: expose Hatchdoor to your LAN]], which explains why and what to use instead.
- **Never answer the model licence for the user.** Accepting Google's Gemma terms is their decision.
- **Never install system software without a yes.** That includes Docker.
- **Write tokens only in three places:** the web token in the deployment's `.env`, the MCP token in Hatchdoor's settings (through the API) and in your own MCP client configuration. Not in notes, shell profiles, logs, other files or your own memory. One exception: a client that can read its token only from an environment variable cannot hold the MCP token in its configuration, so in that one case the command that creates the token prints it once and the hand-over shows it to the user once, as steps 6 and 7 describe.
- **Never put MCP settings in `.env`.** A setting in `.env` is pinned: it can no longer be changed in Settings, so the user could not allow writes later. Set them through the HTTP API as shown.
- **Never use demo mode** (`HATCHDOOR_DEMO_MODE`). It is for public read-only showcases and refuses to run with MCP.

## 1. Check the computer

Before asking anything, find out what you are working with:

```bash
uname -s                                   # Linux or Darwin (macOS)
cat /etc/os-release 2>/dev/null | head -3  # Linux distribution
docker info --format '{{.ServerVersion}}'  # Docker installed and running?
docker compose version
sudo -n true 2>/dev/null && echo "sudo: no password" || echo "sudo: asks for a password"  # Linux only, and not as root
```

On Windows, the commands on this page are written for Bash and run as written in Git Bash, which has `curl`, `openssl` and `sed`. Use it if your tool gives you a Bash shell. In PowerShell, translate each command, and keep your variable names distinct: PowerShell treats `$HD` and `$hd` as the same variable. Windows needs none of the `sudo`, `chown` and `setfacl` lines: leave them out.

- `docker info` prints a version: Docker is ready.
- `docker info` fails with a permission error on Linux: Docker is installed. Run every `docker` command on this page with `sudo`.
- `docker info` says it cannot connect, but the `docker` command exists: Docker is installed and stopped. On Linux, start it with `sudo systemctl start docker`, or put that line in the script when `sudo` asks for a password. On macOS or Windows, ask the user to open Docker Desktop, as part of the questions below.
- `docker` is not found, or `docker compose version` fails: Docker or its Compose plugin is missing. On Linux, ask question 6 below. On macOS or Windows, the user installs Docker Desktop first. Send one message about that and nothing else, then wait. Leave the questions of step 2 out of it, even to save time: installing Docker Desktop can restart the computer and end this conversation, and answers given now would be lost. Say first that Docker Desktop is needed only if Hatchdoor should run on this computer: if they want it on another computer at home that you reach over SSH, they tell you so now, skip the install, and you go on to the questions. Give them https://www.docker.com/products/docker-desktop/ and tell them to download it, install it, open it and accept its terms. Say what to expect on Windows: the installer may ask for a restart, and Docker Desktop may then report that WSL is not installed and show a command to run, followed by a second restart. Ask them to come back when Docker Desktop shows that the engine is running, and to start you again with the same one line if this conversation is gone by then. Then run `docker info` again and go on to the questions.

On Linux, the last line says whether `sudo` asks for a password. If it does, you cannot run `sudo` yourself: every `sudo` command on this page then goes the way [[#When sudo asks for a password]] describes. If you are root (`id -u` prints `0`), skip the check and leave `sudo` off every command.

If the user picks another computer in question 1, run the same checks there over SSH once they have answered. You cannot check that computer before asking, so when it runs Linux, question 6 is always part of the interview: its yes or no covers Docker being missing there too.

## 2. Ask everything at once

Ask all of these in one message, so the user can answer in one go. Show the default where a question has one; questions 5 and 6 have none. Explain each one in a sentence; the user may never have used Hatchdoor or Docker.

1. **Which computer should Hatchdoor run on?** This one (the default), or another computer on the home network that you can reach over SSH. For another one, ask for its SSH address, such as `alex@homeserver`.
2. **Where are your notes?** Start empty (the default), an existing folder of Markdown notes on that computer (ask for its full path), or a Git repository (ask for its HTTPS address, and for a private one an access token that can read it). Say with that choice that notes from a Git repository start read-only in Hatchdoor, for the browser and for you alike, and that this can be changed afterwards in the Vault's settings.
3. **Who should be able to open Hatchdoor?** Only the computer it runs on (the default), or every device on the home network. If the answer to question 1 is another computer, do not ask: it has to be the home network, or neither the user's browser nor you could reach it from here. Say so in the questions message.
4. **May I change your notes, or only read them?** Read only is the default. Writes can be allowed later in Settings at any time. Notes from a Git repository start read-only whatever the answer; a yes takes effect once the user has switched that Vault to Two-way.
5. **Which search model?** There is no default; the user must choose:
   - **Gemma**: searches in many languages and uses less memory, about 0.5 GB while indexing. Using it means accepting Google's Gemma terms, at https://ai.google.dev/gemma/terms.
   - **Nomic**: English only, uses about 1.3 GB while indexing, no terms to accept.
6. **Only when Docker or its Compose plugin is missing on Linux, or the user may pick another Linux computer:** may I install Docker if it is missing, following Docker's official instructions for your distribution? Installing it needs administrator rights on that computer. There is no default; the user must choose.

On Linux, if `sudo` asks for a password on this computer, add one sentence to the questions message: you cannot type their password, so part-way through you will give them one line to run in a terminal of their own.

After the user answers, do not stop to ask anything else until the hand-over in step 7, unless something fails that you cannot fix. The one planned stop is that line, on a Linux computer whose `sudo` asks for a password.

## When `sudo` asks for a password

This section is about the computer Hatchdoor will run on. It applies when that computer runs Linux and `sudo -n true` failed there. For another computer you learn that only after the answers, so the message below is the first the user hears of it: explain it there.

You cannot type the user's password, so you cannot run any `sudo` command yourself. Never ask for the password, and never change how `sudo` works. Put every command of steps 1, 3 and 4 that needs `sudo` into one script, and have the user run it once:

1. Do the parts of step 4 that need no `sudo` first: the deployment folder, `compose.yaml` and `.env`.
2. Write the script into the deployment folder as `admin-steps.sh`. The user runs it as the administrator, so leave `sudo` off its lines, and write every path in full: the script does not run in your shell or your folder. Start it with `set -e`. Then, in this order:
   - Docker's installation commands and `systemctl enable --now docker`, only if Docker is missing and the answer to question 6 was yes.
   - `systemctl start docker`, only if Docker was installed and stopped.
   - The `chown` line from step 4.
   - The `setfacl` line from step 4, only for an existing notes folder and only if `setfacl` is installed.
   - `docker compose up -d`, after a `cd` into the deployment folder.
   - A last line that prints that it finished.

   Running it a second time must do no harm.
3. Tell the user in one message what the script does, and that it asks for their password because you cannot type it. Ask them to open a new terminal window, run this line, and tell you when it says it finished. It has to be a terminal of their own: a command run through you, such as one typed after Claude Code's `!`, gives `sudo` nowhere to ask for the password.

   ```bash
   sudo bash ~/hatchdoor/admin-steps.sh
   ```

   For another computer, the line is `ssh -t alex@homeserver 'sudo bash ~/hatchdoor/admin-steps.sh'`.
4. When they say it finished, carry on with step 5. If it failed, ask for the last lines it printed, fix the script and ask them to run it again. If `sudo` refuses them altogether, they are not an administrator on that computer, and someone who is has to run the line.

The script holds no token and may stay in the folder. If a later `docker` command needs `sudo`, for example to read the logs after a failure, ask the user to run it the same way.

## 3. Install Docker, if needed

Only with a yes to question 6. Follow the page for the user's distribution at https://docs.docker.com/engine/install/, using Docker's own package repository; it installs the Compose plugin too. Then start Docker and make it start at boot:

```bash
sudo systemctl enable --now docker
```

Install only what Docker's page lists. The yes to question 6 covers Docker and nothing else, `setfacl` included: step 4 says what to do without it.

Do not add the user to the `docker` group; that is a change they did not ask for. Use `sudo docker` instead.

## 4. Install Hatchdoor

For another computer, run the commands in this step there, over SSH. API calls in step 5 run from wherever you are.

Your tool may run each command in a new shell, where the variables and functions an earlier command set are gone. Keep the lines that set one in the same command as the lines that use it: `NOTES` in this step, `REV` and `MCP_TOKEN` in step 5, `SID` and `mcp` in step 6. `HD` and `WEB_TOKEN` are used all through steps 5 and 6, so set them again at the start of each command with the two lines step 5 opens with. Never write a token to a temporary file to carry it from one command to the next.

Create the deployment folder:

```bash
mkdir -p ~/hatchdoor && cd ~/hatchdoor
mkdir -p data/cache data/state models vault
chmod 700 data/cache data/state models
```

Create `compose.yaml` in it with exactly this content:

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
    stop_grace_period: 3m
    healthcheck:
      test: ["CMD", "/app/hatchdoor", "--healthcheck"]
      interval: 30s
      timeout: 5s
      retries: 3
      start_period: 40s
```

If the answer to question 3 is the home network, change the `ports` line to `- "42824:42824"`. Leave `HOST: 0.0.0.0` as it is either way: it is the listener inside the container, and only the `ports` line decides who can connect.

Write `.env` with a new web token, readable only by its owner:

```bash
umask 077
printf 'HATCHDOOR_WEB_BEARER_TOKEN=%s\n' "$(openssl rand -hex 32)" > .env
chmod 600 .env
```

If the notes are an existing folder, set `NOTES` to its full path (the example is a placeholder) and add it:

```bash
NOTES='/home/alex/Notes'
printf 'HOST_VAULT_PATH=%s\n' "$NOTES" >> .env
```

On Windows, write the folder with forward slashes, such as `C:/Users/alex/Documents/Notes`.

Hatchdoor runs as user `65532`, not as the user. On Linux, give that user its folders:

```bash
sudo chown -R 65532:65532 data models vault
```

On Linux with an existing notes folder, also let user `65532` read and write it, without changing its owner:

```bash
sudo setfacl -R -m u:65532:rwX -m d:u:65532:rwX "$NOTES"
```

Many Linux systems do not have `setfacl` installed. If it is missing, leave the folder's permissions alone; do not install it or change the folder's owner. Hatchdoor can usually still read the notes, so search works, but neither the browser nor an agent can change them. Step 5 shows whether that happened, and step 7 says how to tell the user. macOS and Windows need neither command; Docker Desktop handles access.

Start Hatchdoor:

```bash
docker compose up -d
```

## Finding the home-network address

Steps 5 and 7 need the address other devices at home use to reach the computer Hatchdoor runs on. A computer often has several addresses: Docker's own networks, a VPN, a second network card. Use the one its default route goes through. Each command below prints that one IPv4 address and sends nothing over the network. For another computer, run it there over SSH.

```bash
# Linux:
ip -4 route get 1.1.1.1 | sed -n 's/.* src \([0-9.]*\).*/\1/p'
# macOS:
ipconfig getifaddr "$(route -n get default | awk '/interface:/ {print $2}')"
# Windows, in Git Bash:
powershell.exe -NoProfile -Command "(Find-NetRoute -RemoteIPAddress 1.1.1.1 | Select-Object -First 1).IPAddress"
```

In PowerShell, run the part between the double quotes of the Windows line.

Use that address wherever this page says home-network address. Two cases need more care, and neither is a reason to stop and ask:

- **The command prints nothing.** The computer has no default route. List its IPv4 addresses (`ip -4 addr` on Linux, `ifconfig` on macOS, `ipconfig` on Windows).
- **The default route belongs to a VPN.** There are two signs. The address starts with `100.`. Or the route goes through a VPN's interface: `ip -4 route get 1.1.1.1` names it on Linux, such as `tun0`, `wg0` or `tailscale0`; `route -n get default` names it on macOS, a `utun` interface; `Find-NetRoute -RemoteIPAddress 1.1.1.1` names it on Windows, as the `InterfaceAlias` of a VPN adapter. List the IPv4 addresses the same way.

In both cases, first leave out `127.0.0.1` and every address that belongs to Docker or a VPN: on Linux the `docker0` and `br-` interfaces, on macOS and Windows any adapter named for Docker, WSL or a VPN. Of what is left, use the address on a wired or Wi-Fi interface that starts with `192.168.`, `10.` or `172.16.` to `172.31.`. In the hand-over, name every address you could have used and say which one you used, so the user can try another if the first does not open.

## 5. Set it up through the API

Point `HD` at Hatchdoor and load the web token into your shell, without printing it. On this computer:

```bash
HD=http://127.0.0.1:42824
WEB_TOKEN=$(sed -n 's/^HATCHDOOR_WEB_BEARER_TOKEN=//p' ~/hatchdoor/.env)
```

For another computer, use its home-network address (see [[#Finding the home-network address]]), for example `HD=http://192.168.1.20:42824`, and read the token over SSH: `WEB_TOKEN=$(ssh alex@homeserver "sed -n 's/^HATCHDOOR_WEB_BEARER_TOKEN=//p' ~/hatchdoor/.env")`.

Wait until Hatchdoor answers:

```bash
until curl -sf "$HD/health" >/dev/null; do sleep 2; done
```

**Apply the model choice** from question 5. Hatchdoor starts downloading the model in the background.

```bash
# Gemma:
curl -sf -X POST "$HD/api/model/accept-gemma" -H "Authorization: Bearer $WEB_TOKEN"
# or Nomic:
curl -sf -X POST "$HD/api/model/decline-gemma" -H "Authorization: Bearer $WEB_TOKEN"
```

**Turn on agent access** with its own token. Never reuse the web token. Set `HATCHDOOR_MCP_WRITE_ENABLED` to `"true"` only if the answer to question 4 was yes. Set it that way for a Git repository too, although that Vault starts read-only: the setting then already matches the user's answer on the day they switch the Vault to Two-way. If your client can read its token only from an environment variable, skip this block for now: step 6 says when to run it.

```bash
MCP_TOKEN=$(openssl rand -hex 32)
curl -sf -X PATCH "$HD/api/settings" \
  -H "Authorization: Bearer $WEB_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"updates": {
        "HATCHDOOR_MCP_ENABLED": "true",
        "HATCHDOOR_MCP_WRITE_ENABLED": "false",
        "HATCHDOOR_MCP_BEARER_TOKEN": "'"$MCP_TOKEN"'"
      }}'
```

The MCP token exists only in the `MCP_TOKEN` variable until your client configuration holds it. If your variables do not last between commands, save it there in this same command, once the `curl` has succeeded: for Claude Code, that is the `claude mcp add` line from step 6. When you need the token again, read it from that configuration into a variable, as `WEB_TOKEN` is read from `.env`. Do not use a command that prints it, such as `claude mcp get`. The one command that may print it is the last one of step 6, for a client that reads its token only from an environment variable.

**Create the Vault.** A fresh install has none, and Hatchdoor never creates one by itself. Every change to the Vault list must name the list's current `registry_revision`, so read it first:

```bash
REV=$(curl -sf "$HD/api/v1/vaults" -H "Authorization: Bearer $WEB_TOKEN" \
  | sed -n 's/.*"registry_revision":\([0-9]*\).*/\1/p')
```

For an empty start or an existing folder, the Vault is the whole mounted folder:

```bash
curl -sf -X POST "$HD/api/v1/vaults" \
  -H "Authorization: Bearer $WEB_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"expected_registry_revision": '"$REV"', "name": "Notes",
       "source": {"type": "local", "path": "/data/vault"}}'
```

For a Git repository, Hatchdoor clones it into its own data folder and pulls changes every 15 minutes. Add `https_credentials` only for a private repository:

```bash
curl -sf -X POST "$HD/api/v1/vaults" \
  -H "Authorization: Bearer $WEB_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"expected_registry_revision": '"$REV"', "name": "Notes",
       "source": {"type": "managed_git",
                  "repository_url": "https://github.com/alex/notes.git",
                  "mode": "pull_only", "poll_interval_secs": 900},
       "https_credentials": {"token": "<access token>"}}'
```

`pull_only` means Hatchdoor only reads from the repository and never pushes, and it refuses every change to the notes, from the browser and from an agent alike. Always create a Git Vault this way, whatever the answer to question 4 was: changing notes needs the Two-way mode and an access token that can push, and setting those up is the user's decision to make later. [[How to set up a Git-backed Vault]] explains the modes, which the user can switch between in the Vault's settings. Until its first clone finishes, a Git Vault reports that its repository is unavailable; that is expected.

**Wait until everything is ready.** `/ready` answers `200` once the model has downloaded and the Vault has finished its first index. Depending on the connection and the number of notes, that takes from under a minute to much longer:

```bash
until curl -sf "$HD/ready" >/dev/null; do sleep 5; done
```

On Windows, expect the long end of that. After the model's download has finished, loading it took about 17 minutes on a slow test computer. All that time `/ready` answers `503` and the Vault shows `"search": "unavailable"`. `docker logs hatchdoor` says that the search model is still being set up, and adds one `ERROR` line for every `503` it answered you, and those lines are the wait, not a failure. That is slow, not stuck. Keep waiting: if your tool ends a command that runs too long, run the same wait again. Do not restart the container, because that starts the loading over. Stop waiting only when the Vault list, read with the next command, shows an error. The log on Windows also warns that compare-and-swap is unavailable on the Vault's filesystem. Every Windows folder used through Docker Desktop gets that warning. It needs no action and no line in the hand-over; [[Install Hatchdoor with Docker Compose#If your notes are in a Windows folder, on ZFS or on a FUSE mount]] explains it.

Then read the Vault list once more:

```bash
curl -sf "$HD/api/v1/vaults" -H "Authorization: Bearer $WEB_TOKEN"
```

The Vault should show `"search": "ready"`. `"local_content": "read_write"` means Hatchdoor can write to the notes folder. `"read_only"` means it can read the folder but not write to it, so neither the browser nor an agent can change notes, whatever the answer to question 4 was (see the `setfacl` note above). Step 7 has the lines to give the user in that case. A Git Vault is the exception to reading this field: it shows `"local_content": "read_write"` and still refuses every change, because it is `pull_only`. Go by the fact that you created the Vault from a Git repository, and use the Git lines in step 7. A Vault that failed shows the reason in `activation_error`, `search_error` or `git_error`. Fix what you can, and say what you could not in the hand-over. [[Vault lifecycle states]] explains each state.

## 6. Connect yourself and prove it works

Add Hatchdoor to your own MCP client configuration as a Streamable HTTP server at `$HD/mcp`, with the header `Authorization: Bearer <MCP token>`. Write the token into the client's own configuration, nowhere else. Claude Code, for example:

```bash
claude mcp add --transport http --scope user hatchdoor "$HD/mcp" \
  --header "Authorization: Bearer $MCP_TOKEN"
```

[[Connect your agent#Configure your MCP client]] lists the configuration for Codex, OpenClaw and Hermes. Where it uses an environment variable, put the token straight into the configuration file instead. If your client can read its token only from an environment variable, follow [[#A client that reads its token only from an environment variable]] below instead of the rest of this step.

Then prove it works. If your client loads new servers straight away, call the tools from it. Otherwise call them over HTTP, as below. Put your own client name in `clientInfo`: Settings shows it as the connected agent.

```bash
SID=$(curl -s -D - -o /dev/null "$HD/mcp" \
  -H "Authorization: Bearer $MCP_TOKEN" \
  -H "Content-Type: application/json" \
  -H "Accept: application/json, text/event-stream" \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"Claude Code","version":"1"}}}' \
  | tr -d '\r' | sed -n 's/^[Mm]cp-[Ss]ession-[Ii]d: //p')
mcp() {
  curl -s "$HD/mcp" -H "Authorization: Bearer $MCP_TOKEN" -H "Mcp-Session-Id: $SID" \
    -H "Content-Type: application/json" \
    -H "Accept: application/json, text/event-stream" -d "$1"
}
mcp '{"jsonrpc":"2.0","method":"notifications/initialized"}'
mcp '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_vaults","arguments":{}}}'
```

Each answer arrives as a `data:` line holding the JSON-RPC response. The proof needs two answers:

1. `list_vaults` shows the Vault you created. Note its `vault_id`.
2. One `search_notes` call succeeds: its result has `"isError": false`. When the user brought notes, search for a few words from one of their note titles; that note must be among the results (`note_title`). On an empty start, an empty `results` list passes.

```bash
mcp '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"search_notes","arguments":{"scope":"<vault_id>","query":"<words from a note title>"}}}'
```

If either fails, [[How to troubleshoot common problems#An agent can't connect over MCP]] matches the error to its cause.

### A client that reads its token only from an environment variable

Such a client has nowhere allowed to keep the MCP token: not its configuration, and you must not set the variable yourself, in a shell profile or anywhere else. So do not create the token until the very end, and hand it to the user the moment it exists.

1. In step 5, skip the **Turn on agent access** block. Do everything else, and note the Vault's `vault_id` from the Vault list. Until the command in item 3 of this list runs, no MCP token exists and agent access stays off.
2. Add Hatchdoor to your client configuration with the name of the variable in place of the token, the way [[Connect your agent#Configure your MCP client]] shows for your client. Do not set the variable.
3. As your last action before the hand-over, run **one command** made of these parts, in this order:
   - the two lines that set `HD` and `WEB_TOKEN`, from the start of step 5;
   - the **Turn on agent access** block from step 5, with `>/dev/null || exit 1` added to the end of its `curl`, so that nothing after it runs, and no token is printed, when the settings call fails;
   - the proof above: its whole first code block (the `SID` and `mcp` lines, `notifications/initialized` and `list_vaults`), then the `search_notes` call with the `vault_id` you noted;
   - a last line that prints the token: `printf 'MCP token: %s\n' "$MCP_TOKEN"`.
4. Check the two answers of the proof in that command's output. Copy the token from its last line into the hand-over message and nowhere else.

The command is the only place the token is created, and it needs no temporary file and no variable that outlives it. If the proof fails, fix the cause and run the whole command again: it makes a new token that replaces the old one.

In the hand-over, replace the MCP token line under "Where your tokens live" with the lines step 7 gives for this case.

## 7. Hand over

End with one message that contains exactly this, filled in. It is the only place a token is ever shown to the user.

```text
Hatchdoor is installed and running.

Open it at:  <web address: http://localhost:42824, or http://<home-network address>:42824>
Web token:   <the web token>
  This is the password your browser asks for. Copy it now: I won't show it again.

Where your tokens live:
- The web token is in <deployment folder>/.env on <computer>, readable only by your user.
- The MCP token, the separate password I use, is in Hatchdoor's own settings
  (Settings > Agent access (MCP)) and in my MCP configuration.

I can <only read your notes | read and change your notes>.
To let me change notes later: Settings > Agent access (MCP) > Let assistants change notes.

To upgrade Hatchdoor later, ask me, or open Help > How to upgrade Hatchdoor.
```

When the address is a home-network address, it is the one [[#Finding the home-network address]] gave you. If you had to pick it from several, add one line under "Open it at" that names the others, such as `If that does not open, try: http://10.0.0.5:42824`.

On Windows, a home-network address does not open from another device until the user allows it. Windows shows a **Windows Security** window that asks whether to allow networks to access **Docker Desktop Backend**, and you can neither see it nor answer it. So when the address is a home-network address and Hatchdoor runs on Windows, add these lines under "Open it at", after any line that names other addresses:

```text
  Windows asks whether to allow "Docker Desktop Backend" on your networks.
  Look for a Windows Security window and choose Allow. Until you do, the address
  above opens on this computer only.
```

If the Vault list in step 5 showed `"local_content": "read_only"`, the two lines about changing notes would mislead: allowing changes in Settings is not enough when Hatchdoor cannot write to the folder. Replace them with these, ending the last line the way the answer to question 4 went:

```text
Hatchdoor can search and show your notes, but it cannot write to <notes folder>.
So for now notes cannot be changed in the browser or by me.
To edit notes in the browser: give user 65532 write access to that folder. See
Help > How to troubleshoot common problems > Permission denied reading or writing the Vault.
For me to change notes as well: Settings > Agent access (MCP) > Let assistants change notes
must also be on. <It is off, as you asked. | It is already on, as you asked.>
```

If the Vault you created in step 5 was a Git repository, the two lines about changing notes would mislead too: Hatchdoor only reads from the repository, so the Settings switch is not enough. Replace them with these, ending the last line the way the answer to question 4 went:

```text
Your notes come from your Git repository, and Hatchdoor only reads from it.
So for now notes cannot be changed in the browser or by me.
Turning on "Let assistants change notes" is not enough to change that.
To change notes in Hatchdoor: open the Vault in Settings, switch it to Two-way
and give it an access token that can push to the repository. See
Help > How to set up a Git-backed Vault.
For me to change notes as well: Settings > Agent access (MCP) > Let assistants change notes
must also be on. <It is off, as you asked. | It is already on, as you asked.>
```

If your client reads its token only from an environment variable, replace the MCP token line under "Where your tokens live" with these, and say which file or screen the variable goes in for the user's system:

```text
- The MCP token, the separate password I use, is in Hatchdoor's own settings
  (Settings > Agent access (MCP)). I cannot store it myself, so you have to:
  MCP token:   <the MCP token>
  Copy it now: like the web token, I won't show it again.
  Set it as the variable <variable name> in <where to set it>, then restart me.
```

These replacements combine: a Git Vault set up by a client that reads its token from an environment variable gets both.

For a private Git repository, add that its access token is stored in Hatchdoor's Vault list, in `data/state` of the deployment folder, and can be replaced in the Vault's settings. Add one line for anything you could not finish, such as a notes folder Hatchdoor cannot read.

---

Related: [[Install Hatchdoor with Docker Compose]] · [[Connect your first Vault]] · [[Connect your agent]] · [[How to upgrade Hatchdoor]]
