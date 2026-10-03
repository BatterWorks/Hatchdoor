---
tags: [type/tutorial, topic/vaults]
---

# Connect your first Vault

A Vault is a folder of Markdown notes that Hatchdoor knows about. In this step you tell Hatchdoor which folder holds your notes. Nothing in the folder is changed: Hatchdoor reads your notes where they are and builds its search data elsewhere.

Hatchdoor can only see the folder you named as `HOST_VAULT_PATH` in `.env` during [[Install Hatchdoor with Docker Compose|the install]], and everything inside it. Inside Hatchdoor, that folder is called `/data/vault`. Seeing a folder is not the same as using it: Hatchdoor starts with no Vaults, and a folder becomes a Vault only when you add it.

The easiest way is the **Set up Hatchdoor** checklist that a fresh install opens on. Its first step, **Add your notes**, has **Pick a folder**: choose your folder from the list, check the name Hatchdoor filled in, and select **Add these notes**.

If you closed the checklist, an install with no Vaults shows **No Vaults Yet** instead. From there, or from **Settings** at any time:

1. Choose **Add a Vault** and give the Vault a name.
2. Leave **A folder on this server** selected. The **Folder** list shows the folders Hatchdoor can see, with how many notes each one holds.
3. Click the folder that holds your notes. Use the arrow on a row to go inside a folder, and the trail above the list to go back up. The top row, **Use** followed by the folder's name, picks the folder you are in, so you can pick `/data/vault` itself when all your notes live there.
4. Check the path shown under the list, then choose **Create Vault**.

Hatchdoor adds the Vault and starts indexing it, which means reading every note so it can search them. Notes appear in the sidebar once that first quick read is done. Search by meaning works once the Vault finishes indexing, which takes from seconds to hours depending on how many notes you have and how fast the computer is. The sidebar shows the Vault as indexing until then.

A few things the list tells you:

- **at least 10,000 notes** means Hatchdoor stopped counting early. Very large folders are cut short so the list opens quickly; the folder is still fine to pick.
- **Already a Vault** marks a folder that is a Vault already. It cannot be picked twice. A folder inside a Vault, or one that contains a Vault, is refused when you create it, because two Vaults cannot share notes.
- **No notes found yet** means the shared folder holds no Markdown files. Put your notes in it, then choose **Look again**. To start with an empty Vault and write your notes in Hatchdoor, pick a folder anyway.
- Hidden folders such as `.git` and `.obsidian` are never listed.

If the Vault shows an error once it is added, the usual causes are:

- The wrong folder. Check the path on the Vault's page in **Settings**.
- Hatchdoor cannot read the folder. See [[How to troubleshoot common problems#Permission denied reading or writing the Vault]].
- You can read notes but not edit them. Hatchdoor can read the folder but not write to it. The same troubleshooting section explains how to allow it.

If you left `HOST_VAULT_PATH` out of `.env`, Hatchdoor sees a `vault` folder next to `compose.yaml`. It starts empty, and Hatchdoor never fills it with example notes, so a Vault made from it stays empty until you add Markdown files or write notes in Hatchdoor.

An agent can add the Vault for you instead, with the `create_vault` MCP tool, once you have connected it in [[Connect your agent]].

## Add a folder Hatchdoor cannot see yet

Hatchdoor only sees the folders shared with its container when it was installed. If your notes live somewhere else on your computer, the list cannot show them, and choosing **My folder isn't here** under the list says so. Typing a path that exists only on your computer does not work either: Hatchdoor runs inside the container and looks for the path there.

You have two ways to fix it:

- Point `HOST_VAULT_PATH` in `.env` at the folder that holds your notes, then restart Hatchdoor with `docker compose up -d`. The folder then appears in the list under `/data/vault`.
- Keep the current folder and share another one too. Add a second line under `volumes:` in `compose.yaml`, such as `- /home/me/journal:/data/journal`, and restart. That folder is outside `/data/vault`, so the list does not show it: choose **Type a path instead** under the list and enter `/data/journal`.

Or ask your agent to add it. An agent that can edit your deployment can make either change and create the Vault for you.

To connect another local Vault later, do the same from **Settings** → **Add a Vault** and pick another folder. Want version history, or a Vault backed by a remote repository instead? See [[How to set up a Git-backed Vault]].

With the first Vault ready, keep its contents read-only to agents while you make the first connection in [[Connect your agent]].

---

Previous: [[Install Hatchdoor with Docker Compose]]
Next: [[Connect your agent]]
