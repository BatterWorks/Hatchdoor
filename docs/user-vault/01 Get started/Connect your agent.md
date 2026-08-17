---
tags: [type/tutorial, topic/mcp]
---

# Connect your agent

Hatchdoor exposes a Streamable HTTP MCP endpoint. It is off by default and has
its own password, separate from the web token you used to open the browser.

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

Continue to [[Search and change notes with your agent]] when that read-only
test works.

---

Previous: [[Connect your first Vault]]
Next: [[Search and change notes with your agent]]
