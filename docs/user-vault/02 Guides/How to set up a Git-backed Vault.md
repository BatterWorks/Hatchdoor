---
tags: [type/how-to, topic/vaults, topic/git]
---

# How to set up a Git-backed Vault

Git keeps a history of every change to your notes and can sync them with a copy elsewhere, such as GitHub, GitLab or your own Git server. A Git-backed Vault lets Hatchdoor do that for you: it records your changes and, if you want, pulls and pushes them on a schedule.

This page sets one up from **Settings** in the browser. An agent can do the same over MCP; see [[MCP tools reference]] and [[HTTP API reference]]. If you only want a plain folder with no Git at all, [[Connect your first Vault]] is the shorter path.

## Pick the right starting point first

Three questions decide which option you want. Answer them before you open the form:

- **Does this Vault need version history or a remote at all?** If not, choose **A folder on this server** and leave its Git behaviour at **No Git**. That is a plain folder.
- **Do you already keep this folder in Git yourself, on the same machine Hatchdoor runs on?** Choose **A folder on this server**, then pick a Git behaviour (**Local history**, **Pull-only**, or **Two-way**). Hatchdoor uses your existing working copy in place. It never clones it.
- **Is the source of truth a remote repository you don't have checked out locally?** Choose **A managed Git checkout**. Hatchdoor clones the repository and manages that copy.

## Create the Vault

Open **Settings** → **Add a Vault**, then:

1. Enter a **Name**.
2. Optionally fill **Ignore these files and folders** with comma-separated patterns to leave out of this Vault's search.
3. Under **Where is this Vault?**, choose **A folder on this server** or **A managed Git checkout**.
4. If you chose **A folder on this server**, pick the folder from the **Folder** list, as in [[Connect your first Vault]]. The list shows only folders Hatchdoor can see.
5. Under **Git behaviour**, choose one of the options below. A managed checkout offers only **Pull-only** and **Two-way**, because a managed Vault exists to track a remote.
6. If the behaviour you chose talks to a remote, fill in the fields that appear: **Repository URL**, optionally **Branch** and **Folder within the repository**, **Sign-in**, and the **Sync schedule**.
7. Select **Create Vault**.

## The four Git behaviours

| Behaviour | What it does | Available on |
| --- | --- | --- |
| **No Git** | Nothing. A plain folder with no history and no remote. | A folder on this server |
| **Local history** | Hatchdoor commits your changes locally, shortly after you stop writing. Never contacts a remote. | A folder on this server |
| **Pull-only** | Fetches from the remote on the sync schedule and sends nothing back. The Vault refuses every write, so Hatchdoor never commits on it. | Either |
| **Two-way** | Commits your changes locally, and fetches from and pushes to the remote on the sync schedule. | Either |

Hatchdoor commits a few seconds after the writing stops, so one commit usually holds several saves. Sending those commits to a remote is a separate step that follows the sync schedule, so on Two-way your local history is current well before the remote sees it. The subject names the first few writes it recorded and how many files they touched, and the body carries whatever one-line summary each of those writes supplied. An agent connected over MCP can pass a summary on every write, and the history then says why each note changed. Edits you make in the Vault folder yourself carry no summary, so a commit made up only of those keeps the generic `hatchdoor: vault update`.

> [!warning]
> Local history creates a hidden `.git` folder inside the Vault's notes folder to hold its history, and that folder only grows: every image and PDF ever attached stays in it, even after you delete the file from the Vault. On a Vault with large attachments, choose Local history only if you accept that growth.

## The shared remote fields

These appear when the behaviour you chose talks to a remote, which is **Pull-only** or **Two-way**, for a folder or a managed checkout:

- **Repository URL**: required for Pull-only and Two-way. Local history does not show it, having nothing to fetch or push.
- **Branch** (optional): leave blank to track the remote's default branch.
- **Folder within the repository** (optional): leave blank to use the repository root as the Vault.
- **Sign-in**: **No sign-in** for a public repository, or **Access token** for a private one. The token works over HTTPS only, is never shown again once saved, and is stored apart from every other secret Hatchdoor holds.
- **Sync schedule**: how often Hatchdoor checks the remote when you don't sync by hand. Hatchdoor cannot be told when something is pushed, so it has to ask. It can be anything from 1 minute to 1440 minutes (24 hours). The default is the slowest, once a day, so set a shorter interval for a Vault you want kept current.

The schedule is measured from the Vault's last completed check and survives a restart: Hatchdoor remembers when each Vault last checked, so a restart or a redeploy resumes the countdown and does not start a new one. A Vault already past its interval when Hatchdoor starts syncs straight away, and one still inside its interval waits out the rest.

A shorter schedule also applies to the check the Vault is already waiting on. Change a Vault from daily to hourly and its next check moves to an hour after its last one, which may be now. A longer schedule leaves the pending check where it is and applies after it, so a check the Vault was about to make is never pushed back.

The exception is a Vault that is currently retrying a failure. After a check fails for a reason worth retrying, such as a remote that was briefly unreachable, Hatchdoor schedules the retry itself, seconds away and not on your schedule. Shortening the interval leaves that retry alone, so a failing remote is not hit more often. If you shorten the schedule of a Vault showing `unavailable` and its next check does not move, the retry is in progress and your edit was not ignored. The new schedule applies once a check succeeds.

## Editing an existing Vault's Git settings

Open the Vault from **Settings**. Its page has a **Save Vault** button in the header. On a Vault with Git it also has a **Sync** console, which shows whether the last sync went through and has a **Sync now** button, or **Try again** after a failure.

Two kinds of edit behave differently:

- **Ordinary edits** save at once with **Save Vault**: the name, ignored patterns, archive folder, commit identity, sync schedule, or a switch between Pull-only and Two-way on the same repository.
- **Identity changes** point the Vault at different content: a different folder path, repository URL, branch or subdirectory. On a local folder you can always edit these fields. On a Vault that comes from Git they are read-only until you select **Edit**. Saving one shows a confirmation first:

> [!note]
> "This runs as one step: the Vault pauses, the change saves, and the Vault starts back up. It stays out of the sidebar and All Vaults for that moment." Confirming also clears any stored sign-in token, even if you didn't touch it, so sign in again afterward if the Vault still needs one. If you're moving into Local history, the disk-growth warning above appears again in the same confirmation.

If the last step, starting the Vault again, fails, the Vault stays paused and hidden. A banner appears with a **Try to bring this Vault back** button that retries that step alone.

## If a commit or a sync fails

Every Git-backed Vault has a console on its Settings page. On a Vault with a remote it is headed **Sync** and its button reads **Sync now**; on a Local history Vault it is headed **History** and reads **Commit now**, because there is nothing to sync with. Either way it reports what happened in words and not as a code: a rejected sign-in, an unreachable remote, local edits Hatchdoor cannot reconcile, or commits on a Pull-only Vault that it is not allowed to push. Each failure message says what happened, says nothing was lost, names the one thing that clears it, and ends in **Try again**.

One failure happens while setting a Vault up: a managed checkout that cloned without error but could not be moved into place. The message gives the underlying error. A common cause is the filesystem, since installing the checkout needs the same single-step file swap a note save needs (see [[Install Hatchdoor with Docker Compose#If your notes are in a Windows folder, on ZFS or on a FUSE mount]]). If a restart or a dropped connection interrupts a clone, the next attempt removes what it left behind and clones again. You have nothing to clean up. See [[How to troubleshoot common problems]].

A sync with a remote never hangs. A remote that takes more than 15 seconds to connect, or sends nothing for 120 seconds during a transfer, is reported as unreachable and retried. A conflict with the remote leaves the checkout exactly as it was before the merge, with no conflict markers in your notes, and a push the remote refuses, because of a protected branch or a server-side hook for example, is reported as a failure with the remote's reason. Either failure stays on the Vault's console while Hatchdoor keeps committing your saves locally. Only a successful sync clears it, and a successful commit does not. On a Two-way Vault, a conflict can be resolved without access to the server: **Publish my side to a branch** on the Vault's page pushes Hatchdoor's side to a branch on the remote for you to merge there. [[How to troubleshoot common problems]] lists each failure and its fix.

After a failed commit Hatchdoor waits five minutes before trying again on its own, however much you write meanwhile, so a standing problem doesn't fill the log with the same error. Fix the cause and it resumes by itself. Press **Commit now** or **Try again** if you do not want to wait.

---

Related: [[Connect your first Vault]] · [[Install Hatchdoor with Docker Compose]] · [[HTTP API reference]]
