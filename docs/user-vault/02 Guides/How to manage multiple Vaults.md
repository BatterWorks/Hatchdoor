---
tags: [type/how-to, topic/vaults]
---

# How to manage multiple Vaults

Hatchdoor is multi-vault by design — there's no selected or default Vault anywhere in the app. [[Connect your first Vault]] and [[How to set up a Git-backed Vault]] cover creating one; this page covers living with more than one: adding another, pausing and resuming, disconnecting, and the same three actions from an agent over MCP.

## Add another Vault

Open **Settings** → **Add a Vault**. The same creation form handles every Vault, first or fifth: **A folder on this server** for a plain directory or an existing local Git checkout, **A managed Git checkout** for a remote Hatchdoor should clone and own. See [[Connect your first Vault]] for the container-path caveat with local folders, and [[How to set up a Git-backed Vault]] for the full field-by-field walkthrough of Git behaviour.

> [!note]
> Demo mode (`HATCHDOOR_DEMO_MODE=true`) removes **Add a Vault** entirely — a public read-only instance has no Settings screen to reach it from.

## Where each Vault shows up

**Settings** lists every Vault, enabled or paused. The rest of the app — the sidebar, search, the graph — only ever shows enabled Vaults; a paused Vault disappears from browsing and search but keeps its entry, its files, and its history untouched. If you're looking for a Vault you know exists and can't find it while browsing, check whether it's paused in Settings before assuming something's wrong.

## Pause and resume a Vault

Open the Vault from the Settings index and use **Pause Vault** / **Resume Vault** in its action row. Pausing:

- Removes the Vault from the sidebar, search, and the graph immediately.
- Leaves every file, the search index, and any Git history exactly as they were — nothing is deleted or rebuilt.
- Disables that Vault's write capability, MCP included, until it's resumed.

There's no separate confirmation step for pausing — it's reversible in one click either direction, unlike disconnecting below.

## Disconnect a Vault

**Disconnect Vault**, in the same action row, is the one Vault-lifecycle action that isn't a toggle. It removes the Vault's *definition* from Hatchdoor's registry — the record of where it lives and how it's configured — while leaving the Vault's own files, folder, Git history, and credentials on disk exactly where they were.

> [!warning]
> Disconnecting forgets the Vault; it does not delete anything. To reconnect, use **Add a Vault** again and point it at the same folder or repository — Hatchdoor treats that as adding new content, not resuming an old identity, so any Vault-scoped settings (exclusion patterns, archive folder, commit identity) need re-entering.

Disconnect has no undo inside Hatchdoor itself, which is why it's a red button with the warning line printed above it rather than a confirmation dialog after the click — the app tells you the consequence before you act, not after.

## The same three actions over MCP

An agent manages Vaults with the same tools it uses for everything else, gated by `HATCHDOOR_MCP_WRITE_ENABLED` like any other write — see [[MCP tools reference#Vault collection: discovery and management]] for full parameters. The pattern is the same for all three:

```text
1. Call list_vaults. Read the target Vault's vault_id and the current registry_revision.
2. Call enable_vault / disable_vault / disconnect_vault with that vault_id and expected_registry_revision.
3. A stale expected_registry_revision is rejected rather than silently racing another writer — re-read list_vaults and retry.
```

Creating a Vault over MCP works the same way, with `create_vault` in place of the three lifecycle calls — see [[How to deploy Hatchdoor with an agent]] for a full walkthrough of standing up Hatchdoor and its first Vault entirely from an agent session.

> [!tip]
> `list_vaults` is always callable, with or without write mode — an agent can inventory every Vault, its status, and its capabilities before deciding anything needs to change. Only the four mutating calls (`create_vault`, `edit_vault`, `enable_vault`/`disable_vault`, `disconnect_vault`) require write mode.

---

Related: [[Connect your first Vault]] · [[How to set up a Git-backed Vault]] · [[How to deploy Hatchdoor with an agent]] · [[MCP tools reference]] · [[HTTP API reference]] · [[Vault lifecycle states]]
