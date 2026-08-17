---
tags: [type/tutorial, topic/vaults]
---

# Connect your first Vault

A Vault is a folder of Markdown files. The `HOST_VAULT_PATH` value in `.env`
is mounted inside the container as `/data/vault`:

```env
HOST_VAULT_PATH=/absolute/path/to/your/markdown-vault
```

On a first start, Hatchdoor recognizes that folder as its first local Vault and
stores the Vault definition in its registry. Existing Markdown is not seeded
or rewritten. Its folder name becomes the initial Vault name.

- [ ] Confirm the host folder is the Vault you intended.
- [ ] Confirm the container can read it.
- [ ] If agents or the browser should write, confirm the container can write it.

If you omit `HOST_VAULT_PATH`, Compose mounts `./vault` next to the deployment.
An empty folder receives the starter Vault. This is useful for a trial, not a
reason to copy your real notes into the container.

> [!warning]
> Do not set a path in Settings that exists only on your host. Settings sees paths inside the Hatchdoor container. The standard Compose file exposes only `/data/vault`; add a volume mount before connecting any other local folder.

To connect another local Vault later, open **Settings** → **Add a Vault** →
**A folder on this server** and enter the already-mounted container path.

With the first Vault ready, keep its contents read-only to agents while you
make the first connection in [[Connect your agent]].

---

Previous: [[Install Hatchdoor with Docker Compose]]
Next: [[Connect your agent]]
