---
tags: [type/tutorial, topic/vaults]
---

# Connect your first Vault

A Vault is a folder of Markdown files. The `HOST_VAULT_PATH` value in `.env`
is mounted inside the container as `/data/vault`:

```env
HOST_VAULT_PATH=/absolute/path/to/your/markdown-vault
```

Hatchdoor can see that folder, but it does not turn it into a Vault by itself:
a fresh install opens with **No Vaults Yet**. Choose **Add a Vault**, then
**A folder on this server**, and enter `/data/vault`, or a folder inside it.
Hatchdoor stores the Vault in its registry and indexes it. Existing Markdown is
not rewritten.

- [ ] Confirm the host folder is the Vault you intended.
- [ ] Confirm the container can read it.
- [ ] If agents or the browser should write, confirm the container can write it.
- [ ] If that folder is on ZFS or a FUSE mount, read the filesystem note in [[Install Hatchdoor with Docker Compose]].

If you omit `HOST_VAULT_PATH`, Compose mounts `./vault` next to the deployment. Hatchdoor writes nothing into an empty folder, so a Vault added there stays empty until you add Markdown files to it.

An agent can add the Vault for you instead, with the `create_vault` MCP tool, once you have connected it in [[Connect your agent]].

> [!warning]
> Do not set a path in Settings that exists only on your host. Settings sees paths inside the Hatchdoor container. The standard Compose file exposes only `/data/vault`; add a volume mount before connecting any other local folder.

To connect another local Vault later, do the same from **Settings** → **Add a
Vault** with another already-mounted container path.
Want version history, or a Vault backed by a remote repository instead? See
[[How to set up a Git-backed Vault]].

With the first Vault ready, keep its contents read-only to agents while you
make the first connection in [[Connect your agent]].

---

Previous: [[Install Hatchdoor with Docker Compose]]
Next: [[Connect your agent]]
