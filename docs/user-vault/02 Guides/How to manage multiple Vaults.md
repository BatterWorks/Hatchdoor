---
tags: [type/how-to, topic/vaults]
---

# How to manage multiple Vaults

You can keep several Vaults in one Hatchdoor, for example one for work notes and one for home. Each is its own folder, and you can pause or remove each one on its own. [[Connect your first Vault]] and [[How to set up a Git-backed Vault]] cover adding one. This page covers living with more than one: adding another, pausing and resuming, disconnecting, and asking an agent to do the same.

Hatchdoor never picks a Vault for you: there is no "default" Vault. Agents name the Vault they mean on every call.

## Add another Vault

Open **Settings** → **Add a Vault**. The same form adds every Vault, first or fifth:

- **A folder on this server** for a plain folder, or a folder you already keep in Git yourself. Pick it from the list, or make an empty one there with **New folder**, as in [[Connect your first Vault]].
- **A managed Git checkout** for a Git repository Hatchdoor should copy down and keep in sync. [[How to set up a Git-backed Vault]] walks through every field.

Two Vaults cannot share notes, so a folder that is already a Vault, sits inside one, or contains one is refused with `409 vault_path_overlap`. The message names the Vault in the way and how the folders relate, such as `This folder is inside the disabled Vault "Archive"`. A disabled Vault still keeps its folder. A Vault also cannot overlap Hatchdoor's own data folders: the one holding `vaults.json` (`/data/state` in the container), the cache folder, or the folder of the settings file. Hatchdoor refuses such a Vault with `400 invalid_vault_definition`, whether you are creating, editing or resuming it. Managed Git checkouts are the exception, since Hatchdoor itself places them under the state folder.

> [!note]
> A public demo instance (`HATCHDOOR_DEMO_MODE=true`) has no Settings screen, so it has no **Add a Vault** either.

## Where each Vault shows up

**Settings** lists every Vault, running or paused. The rest of the app (the sidebar, search, the graph) shows only running Vaults. A paused Vault disappears from browsing and search but keeps its entry, its files and its history. If a Vault you know exists is missing from the sidebar, check in Settings whether it is paused before assuming something is wrong.

## Pause and resume a Vault

Open the Vault from the list in Settings and use **Pause Vault** or **Resume Vault** on its page. Pausing:

- Removes the Vault from the sidebar, search and the graph straight away.
- Leaves every file, the search index and any Git history as they were. Nothing is deleted or rebuilt.
- Stops all changes to that Vault, from agents too, until you resume it.

Pausing asks for no confirmation, because one click undoes it.

## Disconnect a Vault

**Disconnect Vault**, on the same page, makes Hatchdoor forget the Vault. It removes the Vault from Hatchdoor's list, including where it lives and how it is set up, and leaves the Vault's own files, folder and Git history on disk where they were.

> [!warning]
> Disconnecting forgets the Vault and deletes no notes. To reconnect, use **Add a Vault** again and pick the same folder or repository. Hatchdoor treats it as a new Vault, so its own settings (ignored files, archive folder, commit identity) need entering again.

Hatchdoor has no undo for disconnecting. That is why the button is red and the warning sits above it, where you read it before you click.

## Ask an agent to do it

An agent manages Vaults with MCP tools, and needs **Let assistants change notes** turned on for anything but looking. See [[MCP tools reference#Vault collection: discovery and management]] for every parameter. You can ask in plain words, for example "pause my Work Vault in Hatchdoor". The agent then follows this pattern:

```text
1. Call list_vaults. Read the target Vault's vault_id and the current registry_revision.
2. Call enable_vault / disable_vault / disconnect_vault with that vault_id and expected_registry_revision.
3. If the registry_revision is out of date, the call is refused rather than racing another change. Re-read list_vaults and retry.
```

Creating a Vault works the same way, with `create_vault`. [[How to deploy Hatchdoor with an agent]] shows an agent setting up Hatchdoor and its first Vault from scratch.

> [!tip]
> `list_vaults` always works, with or without write access, so an agent can look at every Vault, its status and what it allows before deciding anything needs to change. Every tool that changes a Vault or asks it to do something needs write access: `create_vault`, `edit_vault`, `enable_vault`, `disable_vault`, `disconnect_vault`, `sync_vault`, `retry_vault`, `refresh_vault` and `publish_recovery_branch`.

---

Related: [[Connect your first Vault]] · [[How to set up a Git-backed Vault]] · [[How to deploy Hatchdoor with an agent]] · [[MCP tools reference]] · [[HTTP API reference]] · [[Vault lifecycle states]]
