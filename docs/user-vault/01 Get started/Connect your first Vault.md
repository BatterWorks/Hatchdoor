---
tags: [type/tutorial, topic/vaults]
---

# Connect your first Vault

A Vault is a folder of Markdown files. The `HOST_VAULT_PATH` value in `.env` is the folder on your computer that Hatchdoor can see. Inside the container it appears as `/data/vault`:

```env
HOST_VAULT_PATH=/absolute/path/to/your/markdown-vault
```

Hatchdoor can see that folder, but it does not turn it into a Vault by itself: a fresh install opens with **No Vaults Yet**. To add it:

1. Choose **Add a Vault** and give the Vault a name.
2. Leave **A folder on this server** selected. The **Folder** list shows the folders Hatchdoor can see, with how many notes each one holds.
3. Click the folder that holds your notes. Use the arrow on a row to go inside a folder, and the trail above the list to go back up. The top row, **Use** followed by the folder's name, picks the folder you are in, so you can pick `/data/vault` itself when all your notes live there.
4. Check the path shown under the list, then choose **Create Vault**.

Hatchdoor stores the Vault in its registry and indexes it. Existing Markdown is not rewritten.

A few things the list tells you:

- **at least 10,000 notes** means Hatchdoor stopped counting early. Very large folders are cut short so the list opens quickly; the folder is still fine to pick.
- **Already a Vault** marks a folder that is a Vault already. It cannot be picked twice. A folder inside a Vault, or one that contains a Vault, is refused when you create it, because two Vaults cannot share notes.
- **No notes found yet** means the shared folder holds no Markdown files. Put your notes in it, then choose **Look again**. To start with an empty Vault and write your notes in Hatchdoor, pick a folder anyway.
- Hidden folders such as `.git` and `.obsidian` are never listed.

- [ ] Confirm the host folder is the Vault you intended.
- [ ] Confirm the container can read it.
- [ ] If agents or the browser should write, confirm the container can write it.
- [ ] If that folder is on ZFS or a FUSE mount, read the filesystem note in [[Install Hatchdoor with Docker Compose]].

If you omit `HOST_VAULT_PATH`, Compose mounts `./vault` next to the deployment. Hatchdoor writes nothing into an empty folder, so a Vault added there stays empty until you add Markdown files to it.

An agent can add the Vault for you instead, with the `create_vault` MCP tool, once you have connected it in [[Connect your agent]].

## Add a folder Hatchdoor cannot see yet

Hatchdoor only sees the folders shared with its container when it was installed. If your notes live somewhere else on your computer, the list cannot show them, and choosing **My folder isn't here** under the list says so. Typing a path that exists only on your computer does not work either: Hatchdoor runs inside the container and looks for the path there.

You have two ways to fix it:

- Point `HOST_VAULT_PATH` in `.env` at the folder that holds your notes, then restart Hatchdoor with `docker compose up -d`. The folder then appears in the list under `/data/vault`.
- Keep the current folder and share another one too. Add a second line under `volumes:` in `docker-compose.yml`, such as `- /home/me/journal:/data/journal`, and restart. That folder is outside `/data/vault`, so the list does not show it: choose **Type a path instead** under the list and enter `/data/journal`.

Or ask your agent to add it. An agent that can edit your deployment can make either change and create the Vault for you.

To connect another local Vault later, do the same from **Settings** → **Add a Vault** and pick another folder. Want version history, or a Vault backed by a remote repository instead? See [[How to set up a Git-backed Vault]].

With the first Vault ready, keep its contents read-only to agents while you make the first connection in [[Connect your agent]].

---

Previous: [[Install Hatchdoor with Docker Compose]]
Next: [[Connect your agent]]
