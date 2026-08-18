---
tags: [type/how-to, topic/vaults, topic/git]
---

# How to set up a Git-backed Vault

This covers adding or editing a Vault's Git behaviour from the Web UI's **Settings** screen — not the MCP/API path, which [[How to deploy Hatchdoor with an agent]] and [[HTTP API reference]] already cover. If you only want a plain folder with no Git at all, [[Connect your first Vault]] is the shorter path.

## Pick the right starting point first

Three questions decide which option you want, before you open the form:

- **Does this Vault need version history or a remote at all?** If not, choose **A folder on this server** and leave its Git behaviour at **No Git** — a plain folder, nothing else.
- **Do you already keep this folder in Git yourself, on the same machine Hatchdoor runs on?** Choose **A folder on this server**, then pick a Git behaviour (**Local history**, **Pull-only**, or **Two-way**) — Hatchdoor uses your existing working copy in place. It never clones it.
- **Is the source of truth a remote repository you don't have checked out locally?** Choose **A managed Git checkout** — Hatchdoor clones the repository itself and owns that checkout.

## Create the Vault

Open **Settings** → **Add a Vault**, then:

1. Enter a **Name**.
2. Optionally fill **Ignore these files and folders** with comma-separated patterns to leave out of this Vault's search.
3. Under **Where is this Vault?**, choose **A folder on this server** or **A managed Git checkout**.
4. If you chose **A folder on this server**, enter its **Folder path** — the path as the Hatchdoor container sees it, not your host machine's path (see [[Connect your first Vault]] if that distinction is new).
5. Under **Git behaviour**, choose one of the options below. A managed checkout only offers **Pull-only** and **Two-way** — a managed Vault exists specifically to track a remote, so there's no "no remote" option for it.
6. If the behaviour you chose talks to a remote, fill in the fields that appear: **Repository URL**, optionally **Branch** and **Folder within the repository**, **Sign-in**, and the **Sync schedule**.
7. Select **Create Vault**.

## The four Git behaviours

| Behaviour | What it does | Available on |
| --- | --- | --- |
| **No Git** | Nothing — a plain folder, no history, no remote. | A folder on this server |
| **Local history** | Hatchdoor commits every change locally. Never contacts a remote. | A folder on this server |
| **Pull-only** | Also fetches from the remote on the sync schedule. Content flows in; Hatchdoor's own commits stay local and are never pushed. | Either |
| **Two-way** | Also pushes Hatchdoor's own commits back to the remote. | Either |

> [!warning]
> Local history creates a hidden `.git` folder inside the Vault's own notes folder to hold its history, and that folder grows permanently: every image and PDF ever attached stays in it, even after you delete the file from the Vault. Don't reach for Local history on a Vault with large attachments unless you're prepared for that growth.

## The shared remote fields

These appear whenever the chosen behaviour talks to a remote (**Pull-only** or **Two-way**, on either source):

- **Repository URL** — required for Pull-only/Two-way; not shown for Local history, which has nothing to fetch or push.
- **Branch** (optional) — leave blank to track the remote's default branch.
- **Folder within the repository** (optional) — leave blank to use the repository root as the Vault.
- **Sign-in** — **No sign-in** for a public repository, or **Access token** for a private one. The token is HTTPS-only, write-only (never shown again once saved), and stored separately from every other credential Hatchdoor holds.
- **Sync schedule** — how often Hatchdoor checks the remote absent a manual sync, since "Hatchdoor has no way to be told when something is pushed." Anywhere from 1 minute to 1440 minutes (24 hours); the default is the slowest setting, once a day, so a Vault you want to stay current sooner needs a shorter interval set deliberately.

## Editing an existing Vault's Git settings

Open the Vault from **Settings** and its own page has a **Save Vault** button in the header, plus a **Sync** console (when Git applies) showing whether the last sync was healthy and a **Sync now** (or **Try again**, if something failed) button.

Two kinds of edit behave differently:

- **Ordinary edits** — name, ignored patterns, archive folder, commit identity, sync schedule, or switching between Pull-only and Two-way on the *same* repository — save immediately with **Save Vault**.
- **Identity changes** — a different folder path, repository URL, branch, or subdirectory — change what the Vault actually points at. For a local folder these fields are always editable directly; for a Git-sourced Vault they're read-only until you select **Edit** to unlock them. Saving one of these shows a confirmation first:

> [!note]
> "This runs as one step: the Vault pauses, the change saves, and the Vault starts back up. It stays out of the sidebar and All Vaults for that moment." Confirming also clears any stored sign-in token, even if you didn't touch it — sign in again afterward if the Vault still needs one. If you're moving into Local history, the disk-growth warning above appears again in the same confirmation.

If the final restart step ever fails, the Vault is left paused and hidden rather than silently broken — a banner appears with a **Try to bring this Vault back** button to retry just that step.

## If a sync fails

The Sync console reports what happened in plain language rather than a code — things like a rejected sign-in, an unreachable remote, local edits Hatchdoor isn't sure how to reconcile, or unpushed commits sitting on a Pull-only Vault it isn't allowed to push. Every failure sentence says what happened, confirms nothing was lost, and states the one thing that clears it, ending in **Try again**.

---

Related: [[Connect your first Vault]] · [[Install Hatchdoor with Docker Compose]] · [[HTTP API reference]]
