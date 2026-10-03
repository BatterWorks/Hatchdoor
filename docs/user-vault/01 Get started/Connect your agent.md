---
tags: [type/tutorial, topic/mcp]
---

# Connect your agent

Hatchdoor exposes a Streamable HTTP MCP endpoint. It is off by default and has
its own password, separate from the web token you used to open the browser.

## The quick way: the setup checklist

A new install opens on the **Set up Hatchdoor** checklist. Its third step, **Connect your agent**, does the whole setup below in one click:

1. Select **Connect an agent**. Hatchdoor turns agent access on, keeps **Let assistants change notes** off, and makes a new MCP password.
2. Pick your agent: **Claude Code**, **Codex**, **OpenClaw**, **Hermes**, or **Other** for any other MCP client. The checklist shows the exact command or settings to paste, with your Hatchdoor address and the new password already filled in.
3. Copy it now. The password is shown only this once. If you lose it, select **Make a new password**; agents still using the old one stop getting in.
4. Paste it into your agent and ask it to use Hatchdoor. The step ticks itself as soon as the agent makes its first request, and names it, for example "Claude Code connected 2 minutes ago".

If you closed the checklist, reopen it from **Help** → **Setup checklist**. If the MCP password is set in the deployment's `.env` file as `HATCHDOOR_MCP_BEARER_TOKEN`, the checklist cannot show it: the config says `<your MCP password>` where it goes.

The rest of this page does the same thing by hand, from Settings.

## Set it up in Settings

In Hatchdoor, open **Settings** → **Agent access (MCP)**. Then:

1. Turn on **Let assistants connect (MCP)**.
2. Under **MCP password**, select **Generate** or enter a strong password.
3. Leave **Let assistants change notes** off for now.
4. Select **Save**.

The change applies to new MCP requests immediately; it does not require a
container restart.

> [!success]
> Your agent can now read, but it cannot create, edit, move, delete, or attach anything. Read-only is the right first connection.

Configure your Streamable HTTP MCP client with your Hatchdoor address:

```text
http://localhost:42824/mcp
```

For an agent client on another device, first follow [[Install Hatchdoor with Docker Compose#Optional: expose Hatchdoor to your LAN]]. Then replace `localhost` with the Hatchdoor server's address. Send this header using the MCP password, not the web token:

```text
Authorization: Bearer <your-mcp-password>
```

> [!warning]
> Do not expose Hatchdoor directly to the public internet just to reach MCP. Keep it on a trusted network or place it behind an authenticated, encrypted access layer.

Agents download and upload files through short-lived links that carry the server's address. If an agent reaches Hatchdoor through such a layer, such as a proxy that adds HTTPS, the links use the address the proxy reports in its `Forwarded` header, or its `X-Forwarded-Proto` and `X-Forwarded-Host` headers. If the proxy sends none of them, or serves Hatchdoor under a path, fill in **Public address** in **Agent access (MCP)** with the address the agent uses, for example `https://notes.example.com`. When set, it always wins.

Do not put the MCP password in a note, a prompt, or a screenshot. MCP is a
second door into your Vault; it stays disabled unless you deliberately enable
it, and it always needs this password even for reading.

## Configure your MCP client

The examples below assume the agent runs on the same machine as Docker. If it runs on another device, use `http://<hatchdoor-server-address>:42824/mcp` after enabling the LAN port mapping described above.

### Claude Code

Create `.mcp.json` in your project, or add `hatchdoor` to its existing `mcpServers` object:

```json
{
  "mcpServers": {
    "hatchdoor": {
      "type": "http",
      "url": "http://127.0.0.1:42824/mcp",
      "headers": {
        "Authorization": "Bearer ${HATCHDOOR_MCP_TOKEN}"
      }
    }
  }
}
```

Set `HATCHDOOR_MCP_TOKEN` in the environment that launches Claude Code, then use `/mcp` in Claude Code to check the connection.

### Codex

Add this to `~/.codex/config.toml`, or to `.codex/config.toml` in a trusted project:

```toml
[mcp_servers.hatchdoor]
url = "http://127.0.0.1:42824/mcp"
bearer_token_env_var = "HATCHDOOR_MCP_TOKEN"
```

Set `HATCHDOOR_MCP_TOKEN` in the environment that launches Codex. Run `codex mcp list` to confirm that the server is configured.

### OpenClaw

Add `hatchdoor` under `mcp.servers` in `~/.openclaw/openclaw.json`:

```json
{
  "mcp": {
    "servers": {
      "hatchdoor": {
        "url": "http://127.0.0.1:42824/mcp",
        "transport": "streamable-http",
        "headers": {
          "Authorization": "Bearer <your-mcp-password>"
        }
      }
    }
  }
}
```

Replace the placeholder with the MCP password, keep this file private, and never commit it. Run `openclaw mcp doctor hatchdoor --probe` to test the connection.

### Hermes

Add this under `mcp_servers` in `~/.hermes/config.yaml`:

```yaml
mcp_servers:
  hatchdoor:
    url: "http://127.0.0.1:42824/mcp"
    headers:
      Authorization: "Bearer ${HATCHDOOR_MCP_TOKEN}"
```

Set `HATCHDOOR_MCP_TOKEN` in your shell environment or in `~/.hermes/.env`. Hermes resolves the variable when it connects to Hatchdoor.

> [!tip]
> Reuse the same `HATCHDOOR_MCP_TOKEN` environment-variable name for Claude Code, Codex, and Hermes, but set its value only on devices that should be allowed into your Vault.

Try this prompt with your agent:

```text
Use Hatchdoor MCP in read-only mode. Start with list_vaults. Then search the Vault collection for notes about [a topic I care about], read the best match, and give me a short summary. Do not change any notes.
```

The agent should begin with `list_vaults`, use `search_notes`, and call
`get_note` only after it has identified the note. It should retain the returned
Vault ID; there is no implicit default Vault for MCP work.

To check that the connection worked, open **Settings** → **Agent access (MCP)** in Hatchdoor. Once the agent has used a tool, the section names it and says how long ago, for example "Claude Code connected 2 minutes ago". If it still says "No agent has connected yet", the agent never got through: check the address, and that the client sends the MCP password and not the web token.

Continue to [[Search and change notes with your agent]] when that read-only
test works.

---

Previous: [[Connect your first Vault]]
Next: [[Search and change notes with your agent]]
