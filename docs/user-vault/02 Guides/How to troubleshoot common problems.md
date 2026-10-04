---
tags: [type/how-to, topic/troubleshooting]
---

# How to troubleshoot common problems

Find what you are seeing in the list below and jump to its section. Each section says what is going on, then how to fix it. None of these problems lose notes: your notes are plain files in a folder, and Hatchdoor refuses a change rather than half-make it.

- Hatchdoor stops right after starting, or the container keeps restarting: [[#Hatchdoor won't start]]
- The browser says "Vaults Unavailable": [[#The workspace says "Vaults Unavailable"]]
- The browser asks for a token or password, and you don't know it: [[#The browser asks for a token you don't have]]
- Your agent cannot connect, or gets "not found", "unauthorized" or "forbidden": [[#An agent can't connect over MCP]]
- Your agent cannot download or upload a file: [[#An agent can't download or upload a file]]
- The search model will not download: [[#Model download is stuck or failed]]
- Notes are missing, or not showing up in the sidebar or in search: [[#Notes are missing or not showing up]]
- A Vault shows an error, says "waiting", or never finishes indexing: [[#A Vault won't index or stays in a bad state]]
- "Permission denied", or you can read notes but not edit them: [[#Permission denied reading or writing the Vault]]
- Creating a note works but editing one fails: [[#Editing a note fails but creating one works]]
- Git sync fails, or reports a conflict: [[#Git sync is failing]]
- A Vault seems to have stopped syncing: [[#"Is this Vault still syncing on schedule?"]]
- Search finds nothing, or the wrong notes: [[#Search returns nothing, or not what you expected]]
- A saved query shows a message instead of a table: [[#A saved query shows a message instead of a table]]

## Hatchdoor won't start

On the very first start this is expected. Hatchdoor refuses to run until the browser has a password (the web token), so it makes one, prints it, and stops. The log line starts `HOST=0.0.0.0 is non-loopback but HATCHDOOR_WEB_BEARER_TOKEN is unset: refusing to start unauthenticated on a public interface` and ends with the exact line to paste into `.env`.

Fix: show the newest token with `docker compose logs hatchdoor | grep HATCHDOOR_WEB_BEARER_TOKEN | tail -1`, paste the `HATCHDOOR_WEB_BEARER_TOKEN=...` part into `.env`, then run `docker compose up -d` again.

> [!warning]
> Docker restarts Hatchdoor after each refusal, and every attempt prints a different token. Always take the last one in the log, not the first you see.

If the log says `HATCHDOOR_MCP_ENABLED is set but HATCHDOOR_MCP_BEARER_TOKEN is missing`, you turned on agent access in `.env` without giving it a password. Either add `HATCHDOOR_MCP_BEARER_TOKEN` to `.env`, or remove `HATCHDOOR_MCP_ENABLED` from `.env` and turn agent access on in **Settings** instead (see [[Connect your agent]]), which is the usual way.

If the log says `This install still has the single-Vault setup of Hatchdoor 2.4.x or earlier`, the install is too old to upgrade straight to this version. Hatchdoor stops rather than open on an empty screen that would look like lost notes. Nothing was changed. Fix: run a 2.5.0 to 2.7.x image once, which moves your Vault into the Vault list, then upgrade to this version again.

## The workspace says "Vaults Unavailable"

The browser could not get the list of Vaults from the server: it is offline, the server is restarting, or a proxy in front of it answered with an error such as `502`. Your Vaults and notes are untouched; the app just does not know about them yet, which is why it shows this instead of the "No Vaults Yet" screen. The note you had open is kept and comes back once the list loads. While the server stays unreachable, note counts in the sidebar and on a Vault's Settings page read `–` (not known) rather than `0`.

Fix: check that Hatchdoor is running (`docker compose ps`) and reachable from this device, then press **Try again**. The app also recovers by itself when its live connection to the server comes back.

## The browser asks for a token you don't have

The browser asks for the web token, the password that protects Hatchdoor in the browser. You did not choose it: Hatchdoor made it on the first start, and you pasted it into `.env`. It is the value after `HATCHDOOR_WEB_BEARER_TOKEN=` in the `.env` file next to your `compose.yaml`. [[Install Hatchdoor with Docker Compose#Where do I find my token?]] has the details, including how to set a new one if it is lost.

Don't confuse it with the MCP password, which only agents use. The web token signs in a browser; the MCP password connects an agent.

## An agent can't connect over MCP

First check that agent access is on in **Settings** → **Agent access (MCP)**, and that the agent uses the MCP password from there, not the web token. If it still fails, the error the agent reports tells you which of these it is:

- **`404 Not Found` on `/mcp`, with an empty body**: MCP is disabled. Turn on **Let assistants connect (MCP)** in **Settings** → **Agent access (MCP)**. Read the body before acting on a `404`: an empty one means this, and a `404` that *says* something means the next entry instead.
- **`404 Not Found` whose body reads `Not Found: Session not found`**: MCP is on and the client got in, but the session it is quoting doesn't exist. Either it was never issued, or it was issued before Hatchdoor last restarted. The remedy belongs to the client: it has to run `initialize` again and use the session that comes back.
- **`422 Unprocessable Entity`, "Unexpected message, expect initialize request"**: the same problem from the other side: the client sent an ordinary call with no session at all. The wording describes what the server was expecting to receive rather than what you should do about it; the remedy is the one above, re-initialize.
- **`401`, JSON-RPC error `-32001`, "Missing or invalid MCP bearer token"**: the client's `Authorization: Bearer <token>` header doesn't match the current MCP password. Regenerate or re-copy it from Settings; the client and Settings must hold the exact same value.
- **A write tool (`create_note`, `edit_vault`, etc.) returns "MCP write tools are disabled by HATCHDOOR_MCP_WRITE_ENABLED"**: MCP is connected and reading fine, but **Let assistants change notes** is off. This is a separate toggle from connecting at all; see [[Search and change notes with your agent]] for why that separation exists.

A sixth, less common one: **`403 Forbidden`, "Forbidden MCP origin"**: an `Origin` header was sent that isn't on the allow-list (`HATCHDOOR_MCP_ALLOWED_ORIGINS`). This normally only matters for a browser-based MCP client, not a CLI agent.

> [!note]
> MCP sessions are held in memory only, so restarting Hatchdoor ends every one of them. A client holding a session finds out on its next call, as the `Session not found` or the `422` above, and has to re-initialize. No Hatchdoor setting keeps sessions across a restart, so a client that keeps retrying the same failing call needs restarting or reconnecting; that lever is yours, not Hatchdoor's. Clients that don't use a session at all are unaffected.

## An agent can't download or upload a file

Agents move files through short-lived links that `get_attachment` and `create_upload_link` hand out. A link answers `403` with a `code` that says what went wrong:

- **`transfer_link_expired`**: the link is more than five minutes old. Ask for a new one.
- **`transfer_link_invalid`**: the link was used for a different file, or Hatchdoor restarted or the MCP password changed since it was issued. Ask for a new one.
- **`transfer_link_spent`**: an upload link was already used. Each works once.
- **`mcp_disabled`** or **`mcp_write_disabled`**: MCP, or **Let assistants change notes**, is off.

If the agent cannot reach the link's address at all, or a download saves a small HTML page instead of the file, the link probably starts with `http://` while the agent reached Hatchdoor over HTTPS. Make the proxy send `X-Forwarded-Proto` and `X-Forwarded-Host` (see [[Install Hatchdoor with Docker Compose]]), or set **Public address** in **Settings** → **Agent access (MCP)** to the address agents use. A download over `HATCHDOOR_MCP_MAX_BASE64_BYTES` answers `413`; raise that limit in **Settings** → **Uploads**.

## Model download is stuck or failed

The model downloads from Hugging Face, so this is almost always a network problem between the computer running Hatchdoor and huggingface.co. In the browser, the search panel shows **Could Not Load** with the reason, such as a fetch error for one model file, and a **Retry setup** button. Fix whatever blocked the download, then press **Retry setup**.

An agent sees the same through `get_model_setup_status`, and `GET /api/startup-status` reports `"failed"` with the same message. `POST /api/model/retry` starts the download again.

Two related, more specific errors:

- **`409`, "Gemma terms must be accepted or declined first."**: you tried to retry before choosing a model at all. Accept or decline Gemma first.
- **`409`, "The running search model cannot be changed until Hatchdoor restarts."`**: a model is already active; Hatchdoor doesn't support switching models live. This is expected, not a bug: restart the instance if you genuinely need a different model.

## Notes are missing or not showing up

Work down this list. Most of the time it is the first or second item.

- **The Vault is paused.** A paused Vault disappears from the sidebar and search but keeps all its notes. Look for it in **Settings**, where every Vault is listed, and resume it there. See [[How to manage multiple Vaults#Pause and resume a Vault]].
- **The Vault is still indexing.** A newly added Vault, or one with many new notes, takes a while to read. Notes appear in the sidebar first, and search by meaning catches up after. See the next section.
- **The file is not a note.** Only files ending in `.md` are notes. Hidden folders such as `.obsidian`, `.trash` and `.hatchdoor-trash` are left out, as is anything matching the Vault's **Ignore these files and folders** setting.
- **The note is in a layer.** A folder with a `.hatchdoor-layer` file in it is left out of ordinary search on purpose. Its notes still appear in the sidebar. See [[The layer system]].
- **The note is in a folder Hatchdoor cannot see.** Hatchdoor only sees the folders shared with it at install time. See [[Connect your first Vault#Add a folder Hatchdoor cannot see yet]].
- **You edited it outside Hatchdoor a moment ago.** An edit made in another app can take a few seconds to show up, or about a minute when your notes are in a Windows folder used through Docker Desktop.

## A Vault won't index or stays in a bad state

A Vault's page in **Settings** says in words what is wrong and, where something can be done, offers **Try again**. Start there. An agent reads the same information from `list_vaults`.

Behind those words, Hatchdoor tracks five separate signals for each Vault rather than one "healthy" flag, so a Vault can be broken in one way while working fine in every other:

| Signal | Values | What it means |
| --- | --- | --- |
| `activation` | `active`, `disabled`, `unavailable` | Whether the Vault is running at all |
| `local_content` | `read_write`, `read_only`, `unavailable` | Whether Hatchdoor can read, and write, the notes folder |
| `search` | `unavailable`, `indexing`, `browsable`, `ready`, `stale` | Whether reading and searching work right now |
| `git` | `disabled`, `pending`, `ready`, `unavailable` | Whether Git sync works, for a Git-backed Vault |
| `watcher` | `running`, `disabled`, `unavailable` | Whether Hatchdoor notices files changed outside it |

These are normal, not faults:

- `browsable` rather than `ready` right after adding a Vault means Hatchdoor has read the notes but has not finished preparing search by meaning. That happens once, on the Vault's first index. Restarting Hatchdoor meanwhile loses no work: the next pass picks up where it stopped.
- `stale` means notes changed and Hatchdoor has not finished re-reading them. Give it a moment.
- **waiting** in the sidebar (`index_turn: waiting`) means the Vault is queued behind another one. Vaults index one at a time, and a long first index pauses every five minutes or so to let a waiting Vault through. It resumes from where it stopped.

Anything reporting `unavailable` carries a matching error field (`activation_error`, `search_error`, `git_error` or `watcher_error`) with an error code and a message. Read it before guessing at a cause. [[Vault lifecycle states]] explains every value in depth.

## Permission denied reading or writing the Vault

Hatchdoor runs as its own restricted user, number `65532`, not as you. Your notes folder has to be readable by that user, and writable too if you want to edit notes in the browser or let an agent change them. A folder only your own account can open shows up as this error.

In detail: `local_content` reports `unavailable` with an error code of `vault_path_unreadable` or `vault_path_unavailable`, and the message includes the system's own error, such as `Permission denied (os error 13)`. Give user `65532` access to the folder, for example with `chown` or `chmod` on Linux, as in step 3 of [[Install Hatchdoor with Docker Compose]], then press **Try again** on the Vault's page.

If the message says `No such file or directory (os error 2)` instead, the folder is not there at all, as seen from inside the container: it was moved or renamed, or the mount in `compose.yaml` no longer points at it. Put the folder back, or fix the mount, then run `docker compose up -d` so Hatchdoor starts again with the folder in place.

If you can read and search notes but the edit controls are missing, `local_content` reports `read_only`: user `65532` can read the folder but not write to it. That is not an error, just a read-only Vault. Give that user write access the same way if you want to edit.

## Editing a note fails but creating one works

The symptom is unmistakable once you know it: creating a note succeeds, and every edit, append, rename, move, archive, delete and attachment change fails. Reading, search and Git sync all work, so the Vault looks healthy on every status axis. The error your agent sees is `write_failed`, with a message of `Invalid argument (os error 22)`.

Do not chase permissions for this one. The `chown` above cannot fix it, and `local_content` reports `read_write`, correctly: the folder really is writable.

The cause is the filesystem. Hatchdoor commits a save by swapping the new copy of the note with the old one in a single step, and not every filesystem can do that. ZFS gained the ability in OpenZFS 2.2, and Ubuntu 22.04's standard kernel ships 2.1.5; anything mounted through FUSE cannot do it either. Confirm with `zfs version` on the host and look for `zfs-kmod-2.1.x`.

**Hatchdoor 2.7.0 and later fixes this.** It falls back to checking the note and then replacing it, so every write works again. Upgrade, and nothing else is needed. Nothing was damaged while it was failing: those writes were refused, not half applied.

On a version before 2.7.0, the only other way out is to move the Vault onto a filesystem that can do the swap, such as ext4 or XFS.

After upgrading, a Vault on such a filesystem writes with slightly weaker protection, and Hatchdoor says so rather than leaving you to guess:

- The server log carries one line per Vault at startup, naming the Vault and what the weaker protection costs.
- `GET /api/v1/vaults/{vault_id}/write-capabilities` reports `atomic_compare_and_swap: false` and adds a matching sentence to its `warnings`, which is also what the Web UI shows above the note.

[[Install Hatchdoor with Docker Compose]] has the full explanation of what that weaker protection means in practice.

> [!note]
> This is not one of the five Vault status axes, and no `*_error` field carries it. A Vault in this state is genuinely fine on all five; the answer lives on the write-capabilities route and in that startup log line.

## A write fails for some other reason

Any save that fails for a filesystem reason is written to Hatchdoor's own log from 2.7.0 onwards, not only to the log of the agent or browser that asked. Check the server log first; it names the Vault and the underlying error.

One failure deserves its own treatment: `write_recovery_required`. It means the opposite of every other write failure. The new content *was* written and then could not be checked or put back, because something outside Hatchdoor changed the Vault directory mid-write. Do not retry it. The message names the note and the leftover file holding the previous content, and a person has to decide which version the note should keep. It is logged on the server too.

## Git sync is failing

Check the Vault's Git console for the specific failure rather than assuming. (It is headed **Sync** on a Vault with a remote and **History** on one without, and its button reads **Sync now** or **Commit now** to match.) These need different fixes:

- **Authentication failed**: the stored HTTPS token was rejected by the remote. Re-enter it under **Sign-in** on the Vault's own page; see [[How to set up a Git-backed Vault]].
- **Clone/fetch failed, or the remote is unreachable**: a network or DNS problem, or the repository URL itself is wrong. Confirm the URL resolves from wherever the container runs, not just from your own machine. A remote that stops answering is reported the same way: Hatchdoor gives up after 15 seconds trying to connect or 120 seconds without data, and retries on its own. An interrupted clone needs no cleanup; the next attempt removes what it left behind and clones again.
- **The checkout could not be installed**: the clone itself worked, but Hatchdoor could not move it into place. The message says why, with the underlying error. A common cause is the filesystem described under **Editing a note fails but creating one works**.
- **The push was rejected by the remote** (`managed_git_push_rejected`): Hatchdoor reached the remote and sent its commits, and the remote refused to apply them: a protected branch, a server-side hook, a quota. The message carries the remote's own reason. Nothing landed on the remote, and nothing local was lost. Change the remote's rules or point the Vault at a branch it may push to, then press **Try again**.
- **A conflict with the remote** (`managed_git_conflict`): the same notes changed on both sides. Hatchdoor puts the checkout back exactly as it was before the merge, with no conflict markers left in your notes, and lists the conflicting files. The failure stays on the Vault's status while Hatchdoor keeps committing your saves locally, until a sync succeeds. Hatchdoor never chooses a winner. Publish its side to a branch on the remote and merge it there; see [[#Resolving a sync conflict]].
- **Files changed by hand in the checkout** (`managed_git_dirty_working_copy`, shown as **sync stopped** in the sidebar). Files in the Vault's repository changed outside Hatchdoor where it will not commit them for you: outside the Vault's own folder, or any local change at all on a Pull-only Vault. The Vault's page lists them. Your notes keep saving to disk; only commit and sync wait. Commit, revert or remove those files with Git in that checkout, then press **Try again**.
- **An unfinished merge in the checkout** (`managed_git_operation_in_progress`): the checkout is part-way through a merge (or a rebase, cherry-pick or revert), usually because someone is resolving one by hand or Hatchdoor was stopped mid-sync. Hatchdoor refuses to commit or sync it, because doing so would record conflict markers, and leaves it exactly as found. The Vault's page lists the files that still have conflicts. Finish or abort the operation with Git in that checkout (`git merge --abort` undoes an interrupted merge), then press **Try again**. A Vault using **Local history** reports the same situation as `existing_git_local_history_manual_recovery_required` and needs the same fix.
- **Local commits ahead on a Pull-only Vault**: the checkout has commits the remote doesn't, and a Pull-only Vault never pushes. They aren't Hatchdoor's: such a Vault refuses every write, so anything committed there you committed yourself, by hand or before you switched the Vault to Pull-only. This isn't a failure exactly. Hatchdoor is reporting that local history and the remote have diverged, and staying pull-only rather than silently discarding your commits. Switch the Vault to **Two-way** if you want them pushed, or accept that Pull-only Vaults are meant to be read-mostly.

### Resolving a sync conflict

When a sync stops on a conflict, Hatchdoor's side exists only in its own checkout on the server. You resolve it on your Git host instead, from a branch Hatchdoor publishes there.

1. On the Vault's page, press **Publish my side to a branch**. Hatchdoor commits any pending saves and pushes them to `hatchdoor-recovery/<branch>/<vault id>` on the Vault's remote. The page shows the branch name with a **Copy** button and, for an HTTPS remote, a link to it.
2. On your Git host, or in any clone, merge that branch into the Vault's own branch and resolve the listed files there, the way you would any other merge.
3. Press **Try again** on the Vault's page, or wait for the next scheduled sync. Once the Vault's branch contains your resolution, the sync goes through and the conflict clears.

Hatchdoor only ever adds to the recovery branch. It never force-pushes, never touches the Vault's own branch while publishing, and never deletes the branch, so delete it on the host once you are done, or leave it: the next conflict reuses it. Saves you make after publishing are not on the branch until you press **Publish again**. A note on the conflict list shows a notice in the editor, because editing it before the conflict is resolved can cause the same conflict again.

The page says so when a publish did not go through. If someone added commits to the recovery branch, Hatchdoor leaves it alone rather than overwrite them: merge the branch as it is, or delete it on the host and publish again. If the remote refused the branch, the page gives the remote's reason. The usual one is a token or branch rule that only allows pushing to the Vault's own branch.

An agent can do the same with the `publish_recovery_branch` MCP tool and read the outcome from `list_vaults`; see [[MCP tools reference#Vault collection: discovery and management]].

#### Without the button

You can also resolve the conflict directly in the Git checkout Hatchdoor syncs.

- **Existing checkout**: the folder you pointed the Vault at.
- **Managed checkout**: Hatchdoor keeps the clone at `vaults/<vault id>/repository` in the directory that holds `vaults.json`, which is `/data/state/vaults/<vault id>/repository` inside the container by default. The Vault ID is on the Vault's page.

The container image has no shell and no Git, so work on the mounted volume from the host. The files belong to the image's `nonroot` user (UID 65532), so run Git as that user or fix ownership afterwards, or Hatchdoor will fail to write to the checkout.

1. In the checkout, run `git pull` without switching branches. Git stops on the same files the Vault's page lists. Hatchdoor keeps the remote's token in its registry, not in the checkout, so Git asks for credentials of its own.
2. Edit each listed file to the version you want, then `git add` it and `git commit`.
3. Run `git push`, then press **Try again** on the Vault's page.

While you are part-way through, Hatchdoor reports `managed_git_operation_in_progress` and leaves the checkout alone, so it cannot commit your half-resolved files. Avoid editing the conflicting notes in Hatchdoor until you have pushed.

> [!note]
> A Vault reporting `git: pending` isn't stuck by default. That's the normal state while a clone or fetch is in flight. Only treat it as a problem if it stays `pending` well past the configured sync interval.

## "Is this Vault still syncing on schedule?"

`GET /api/v1/vaults` reports `last_checked_at` and `next_attempt_at` for every Vault with a remote. Answer the question from those rather than from the repository's Git history. `last_checked_at` is when Hatchdoor last *tried*, not when it last succeeded, so read it next to the Vault's Git status: a Vault that is checking on schedule but failing every time shows a recent `last_checked_at` and an `unavailable` status with the reason. A check that finds nothing new leaves no trace in `git log` or `git reflog`, so an unchanged remote-tracking branch is not evidence that Hatchdoor stopped checking; it usually means there was nothing to fetch.

If `next_attempt_at` is in the past by more than a minute or so, something is genuinely wrong. If it's in the future, the Vault is waiting out its interval. **Sync now** on the Vault's page overrides it, and shortening the Vault's sync schedule brings `next_attempt_at` forward to one new interval after `last_checked_at`. The exception is a Vault retrying after a failed check: the retry's own timing wins and `next_attempt_at` does not move. Read it next to the Git status: on a `ready` Vault a shortened schedule moves it, on an `unavailable` one it may not until a check succeeds. Restarting Hatchdoor does not force a sync: a Vault inside its interval resumes the countdown across a restart.

## Search returns nothing, or not what you expected

In the browser, try **Keyword mode** when you are looking for an exact word, name or ID; search by meaning can rank a note with the exact word below notes that are closer in meaning. For agents, check you want a search at all. `search_notes` finds notes by meaning and ranks them; if what you actually want is every note carrying a tag, sitting in a folder, or holding a frontmatter property, that is `query_notes`, which selects rather than ranks and never comes back empty for want of a good enough match. It also reads every layer, so a demoted note it selects is one an ordinary search would not have shown you.

If a search is what you want: search only looks at the **default surface** unless you ask for more. If the note you expected lives under a [[The layer system|layer]], it won't appear in an ordinary search. [[How to organize a Vault with layers]] explains how to search across layers on purpose. If a Vault's `search` status is `browsable` rather than `ready` (see above), semantic search over it isn't available yet, but keyword search and browsing already work.

## A saved query shows a message instead of a table

A fenced `base` block is drawn as a table only when Hatchdoor understands all of it. It supports part of Obsidian's Bases syntax, listed in [[Supported Markdown reference]], and refuses the rest rather than guessing, because a filter applied halfway gives a wrong list that looks like a right one.

- **"Not evaluated."** followed by a reason: the block uses something outside that list that could change which notes appear, such as a formula, a second view, `sort` or a function Hatchdoor does not know, or its YAML does not parse. The reason names the part to change, down to the function or the line of YAML.
- **"Stopped."** followed by a reason: the query is fine, but running it would pass one of Hatchdoor's limits. The saved queries in one note scan at most 20,000 notes between them, each one scanning the whole Vault once, and one note may hold at most 10 saved queries.
- **A line saying something is not supported** under a table that has rows: the block asks for grouping, summaries, or a view type other than a table. Hatchdoor draws the plain table instead. Every row is there; only that presentation was left out.
- **A line about a name** under an otherwise normal table: the `<!-- hatchdoor-query: name -->` marker before that block names it with something other than lowercase letters, digits and hyphens, or another block in the same note uses the same name. The rows are unaffected. A shared name addresses neither block until one of them is renamed.
- **A line about a marker that names nothing**, where the marker sits: a `<!-- hatchdoor-query: name -->` comment has no `base` block after it, or has something other than blank lines between it and the block. Nothing else in the note is affected.
- **"No matches."** is a real answer, not an error: the query was read, checked against every note, and nothing qualified.
- **"Showing the first N notes"** means rows were held back by the `limit` the block sets. **"Truncated:"** means Hatchdoor's own ceiling of 500 rows held them back and more notes qualify.

If a table looks out of date right after you edit another note, the Vault's index may still be catching up (see the `search` status above). The saved query reads its rows from that index, and it is worked out again every time you open the note.

---

Related: [[Connect your agent]] · [[How to set up a Git-backed Vault]] · [[Install Hatchdoor with Docker Compose]] · [[HTTP API reference]]
