---
tags: [type/reference]
private: true
---

# What's new

What changed in each Hatchdoor release, in a few plain lines, newest release first. Anything you must do before or after upgrading comes first in its release and starts with **Action needed**. The [full changelog](https://github.com/BatterWorks/Hatchdoor/blob/main/CHANGELOG.md) lists every change.

Agents read this page with `read_docs` and the page name `whats-new`.

## v2.10.0 - 2026-11-14

- Settings → Layout has a new **Note width** choice for how wide a note's text may run in the Reading view and the editor, for everyone using the server: Default (today's width, about 80 characters a line), Wide (about 100) or Full (the whole pane).
- Settings has a new Layout section with two switches, **Show Changed on disk** and **Show Recently viewed**, both on by default.
- Highlights can now be coloured the Obsidian way: `==🔴text==`, with one of 🔴 🟠 🟡 🟢 🔵 🟣 straight after the opening signs, is drawn red, orange, yellow, green, blue or purple in the Reading view, the source-mode preview, Help and the editor, with the emoji hidden; the editor shows it while the cursor is on that line.
- A note embedded in another note, `![[Other note]]`, now shows that note's content in the Reading view and the source-mode preview, inside a frame that names the note and links to it.
- Obsidian canvases (`.canvas` files) now survive a rename, and you can see them.
- A link to a block, `[[Note#^id]]` or `[text](Note.md#^id)`, now opens the note at the paragraph, list item, quote or table that carries `^id`, in the Reading view, the source-mode preview and Help.
- An agent can now read a note and edit it in one `batch` call.
- Agents can use `find_link_problems` to page through one Vault's broken or ambiguous note links, isolated notes and dead ends.
- An agent that connects over MCP is now told to read the manual page *How to work in a Vault as an agent* before its first write, so it learns to search before creating a note, read a note before changing it and prefer small edits without you pointing it there.
- Docker and Podman now run the same Hatchdoor image, so Podman users can use the plain `latest` and `<version>` tags like everyone else.
- The manual now says that deleting a note in a Git-backed Vault never removes it from the remote or its Git history, since Hatchdoor commits the delete as a move into `.hatchdoor-trash`, and that a note's address can change hands when another note with the same file name is added, moved, renamed or deleted.
- The web token in an `?access_token=` query parameter now works only where the browser cannot send a header: a `GET` of an attachment (`/api/v1/vaults/{vault_id}/assets/...`), a note download (`/api/v1/vaults/{vault_id}/notes/{slug}/download`) and the live update stream (`/api/v1/vaults/events`).
- Hatchdoor can no longer be shown inside another page: a dashboard tile, a Home Assistant iframe panel or any site that puts it in a frame now gets an empty box or a "refused to connect" message, and has to link to it instead.
- An agent can now add or move a Vault only to a folder inside the Vault mount (`VAULT_PATH`, `/data/vault` in the stock install).
- MCP agents on the `2026-07-28` revision now get a refused tool call, such as an unknown tool, a write while writes are off or an invalid argument, with HTTP 200 and the JSON-RPC error in the body, where it was HTTP 400; the error code and message are unchanged, so a client that reads the body sees no difference.
- Mermaid diagrams are drawn by Mermaid 12.
- The search index no longer stores a second, normalised copy of every note's title and path, each with its own index, that nothing has read since links stopped being resolved from the index.
- In a long note, the editor now draws callouts, diagrams, saved queries, formulas, PDF previews and images near the end of the note as soon as it has read that far.
- A heading with a highlight in it, such as `## Plan for ==this week==`, now shows in the note's "On this page" outline without the `==` signs, and without the colour emoji when the highlight opens with one, the way the outline already drops `*` and `_`.
- An agent that connects after search model setup failed, on an instance whose model was chosen earlier, now opens its session with the normal instructions, led by a line saying setup failed, that note tools are refused until Retry setup in the web app, and that `get_model_setup_status` gives the reason.
- When search model setup fails on an instance whose model was chosen earlier, an agent's tool call is now told that setup failed, to read `get_model_setup_status` for the reason and to have Retry setup pressed in the web app's search, which keeps the chosen model.
- An agent that connects right after a restart, while Hatchdoor checks and loads a search model chosen earlier, now opens its session with the normal instructions, led by a line saying the model is loading and note tools ask to retry until it has.
- In the few seconds after a restart while Hatchdoor checks and loads its search model, an agent's tool call is now told the model is downloading or loading and to retry in a moment.
- A PDF embed with a page anchor, such as `![[report.pdf#page=2]]`, now shows its preview and opens on an instance with a web token.
- A Vault added on a folder inside or around the folder of a Vault you just paused and disconnected no longer writes beside a sync the old Vault was still running there.
- A Markdown link that a move rewrites now spells the extension the linked file has.
- Moving or renaming an attachment no longer hangs when a sync tool or another program replaces the file with a pipe at the moment Hatchdoor reads it back, which blocked every other write to that Vault until something wrote to the pipe; the move is now refused at once, the notes that link to the attachment are put back as they were, and the error says the attachment itself could not be restored, since the other program replaced it.
- An agent that calls a tool Hatchdoor does not have, such as a misspelt name, now gets the "Unknown MCP tool" error at every moment.
- Opening a folder in the folder picker, in Add a Vault or the first-run checklist, now moves keyboard focus straight to the new level's "Use" row.
- A link to a heading or block in the note it is written in, `[[#Heading]]` or `[[#^id]]`, now works in the Reading view, the source-mode preview and the editor: it shows the heading's name, or its alias, and jumps to that place in the note.
- Deleting, moving or archiving a note at the top of the Vault no longer takes along attachments that other notes still show.
- Coming back to a note you left while its save was still on its way no longer warns that the note has changed since.
- The startup warning about variables left in your `.env` no longer tells you to remove `HATCHDOOR_GIT_AUTHOR_NAME` and `HATCHDOOR_GIT_AUTHOR_EMAIL`.
- A Vault added on the folder of a Vault you just paused and disconnected no longer writes beside a sync the old Vault was still running there.
- A Local history Vault whose `.git` folder went missing no longer commits its notes into a Git repository further up, such as a home folder kept in Git.
- One folder Hatchdoor may not open, such as a `.trash/private` or `lost+found` with no permissions, no longer stops the once-a-minute folder re-scan from noticing changes.
- The README's Quick Start now prepares `data/cache` and `models` for Hatchdoor's user along with `data/state`, as the manual's install guide already did, so following it no longer leaves a first start unable to open its cache.
- Following a link right after typing no longer stops the next note from saving.
- The Graph page now works from the keyboard and with a screen reader.
- Note downloads and attachments answer properly in three cases that used to give `500 internal_error`.
- An open tab now keeps following Vault changes after the server restarts behind a reverse proxy.
- The Settings page no longer loses edits or reports the wrong thing after an action.
- Saving a retired setting no longer stops Hatchdoor from starting.
- Hatchdoor now checks every search model file against a SHA-256 it ships, before every load, and loads Nomic Embed Text v1.5 from one fixed revision.
- A model download that stops receiving data no longer leaves first-run setup stuck until a restart.
- A Vault on a folder you provide, plain or an existing Git checkout, now follows that folder while Hatchdoor runs.
- Pausing, editing or disconnecting a Vault while it indexes now answers in seconds, and so does stopping the server.
- Changing a Vault's Git setup works again.
- Git sync no longer hammers a remote with a rejected token, leaks the token on a redirect, or stops on one oddly named file.
- One odd file or folder no longer hides notes or takes a Vault offline.
- One huge note no longer crashes the server.
- A write no longer hangs, or reads a huge file whole, when a sync tool or another program changes a note while Hatchdoor is writing it.
- A Vault's files can no longer hang or exhaust the server.
- A search hit's `heading_path` is now always a real heading of the note, so `get_note_section` finds it.
- Keyword search now finds a Chinese, Japanese, Korean or Thai word inside a sentence.
- Repeated-word keyword searches no longer stall other reads: queries are deduplicated and bounded, with a time limit on SQLite work.
- MCP's limits now hold for every agent, and write refusals are readable.
- A rename, move, archive or delete that Hatchdoor could not finish because it was stopped part-way is now finished the next time it starts.
- A note's owner can still save it after Hatchdoor edits it.
- Saving a note no longer changes who can read it.
- A Hatchdoor running with no web token on its own computer (`HOST=127.0.0.1`, the default outside Docker) no longer lets websites you visit read or change your Vaults.
- The note editor keeps your typing when a background refresh returns late, and its two-second autosave no longer gets cancelled by other page updates.
- The note editor no longer saves into a different note when a note with the same name is created in a folder that sorts earlier and takes over the open note's address.
- Wikilinks now reach the note their author meant.
- Section edits no longer mistake YAML comments for headings and erase the rest of a note, or refuse a body heading because a frontmatter comment has the same text.
- Writes now check exclusion patterns against the filename they actually use, after converting backslashes to slashes and adding `.md` for notes.
- Libraries with published security advisories are updated.
- A Git sync no longer destroys a note kept in a path your ignore rules cover.
- A search result's snippet in the Web UI no longer shows code from a `~~~` block, or from a code block that shows another code block, as if it were text.
- A note whose file ends `.MD` or `.Md` is now treated like a `.md` one in three more places.

## v2.9.0 - 2026-10-27

- A Git-backed Vault's history now says where each change came from, whether the web app, a named agent or an edit made outside Hatchdoor. [[How to set up a Git-backed Vault#Where each change came from]]
- Hatchdoor no longer takes more and more memory each time it updates the search index, and your existing index carries over when you upgrade.
- Search matches are now readable in the dark theme and marked on the note page even when the match is a whole heading, bold word, link or table cell.
- In a saved query, `today()` and `now()` follow your server's `TZ` setting, and a folder condition no longer picks up a note named after the folder.
- Text you highlight with `==` in the editor now shows as a highlight on the rendered page.

## v2.8.2 - 2026-10-14

- The note page and Help no longer show a note's comments, the text between `%%` marks or inside `<!-- -->`, so visitors to a read-only or demo Vault never see them. [[Supported Markdown reference#Comments]]
- A note embedded in another note now shows as a link to it instead of a broken image.
- What's new now shows every release you missed while away from Hatchdoor, not only the newest one.
- Settings now shows an on/off setting as On whenever it is on, including one set in `.env` as `1`, `yes` or `on`.
- Attachment download and upload links now work for a path that starts with `./`, stop working as soon as you change the MCP token, and never appear in the logs.
- An agent installing Hatchdoor from the deploy page now reads the whole page, so it no longer misses the last step that shows you the web token.

## v2.8.1 - 2026-10-11

- **Optional usage report.** Hatchdoor can send its maintainer one small report a day of how your install is set up, and nothing about your notes, to decide which platforms to test and which parts of Hatchdoor people rely on. It is telemetry and stays off unless you turn it on. Settings shows the exact report under Usage report, and every field is listed in [[Usage report reference]]
- The Docker and Podman images now describe themselves as a release in the usage report. The 2.8.0 images reported a development build from source.
- An agent that installs Hatchdoor from the deploy page now always shows you the web token in its last message.

## v2.8.0 - 2026-10-10

- **Action needed:** An install on 2.4.x or earlier must upgrade to a 2.5.0 to 2.7.x release first, and only then to 2.8.0. [[How to upgrade Hatchdoor#Coming from 2.4.x or earlier]]
- **Action needed:** After upgrading, start a new session in every agent that was connected to Hatchdoor, or reconnect it, because until then its searches fail with "Structured content does not match the tool's output schema". [[MCP tools reference#Compact and full search hits]]
- A new install opens on a setup checklist that takes you from nothing to an agent searching your notes, and you can reopen it any time from Help. [[Browse and review through the Web UI#The setup checklist]]
- Hatchdoor can tell you when a newer release is out, if you turn that on in Settings under Updates. [[How to upgrade Hatchdoor#Hear about new releases]]
- **Optional usage report.** Hatchdoor can now send its maintainer one small report a day of how your install is set up, used to decide which platforms to test and which parts of Hatchdoor people rely on. It is telemetry and stays off unless you turn it on in Settings under Usage report. Every field it sends is listed in [[Usage report reference]]
- A note now opens straight into a live editor, so you click anywhere and type, with diagrams, math and callouts drawn as you write. [[How to edit notes with the live editor]]
