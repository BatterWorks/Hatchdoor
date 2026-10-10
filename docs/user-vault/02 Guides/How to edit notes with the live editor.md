---
tags: [type/how-to, topic/web-ui]
---

# How to edit notes with the live editor

On a Vault Hatchdoor can write to, a note opens straight into its editor. There is no edit mode to switch into: click anywhere in the body and type. The Markdown syntax you do not normally see, such as `##`, `- ` and `**`, appears only on the line your caret is on and disappears when you move off it, so the note reads as a page while you write it. This is the same model as Obsidian's Live Preview. For the Markdown syntax, see [[Supported Markdown reference]]. For browsing and search, see [[Browse and review through the Web UI]].

## Three views of a note

- **Editing**, the default on a writable Vault. The note is set in a column of about 75 characters a line, the same in Editing and Reading, because longer lines are tiring to read. The whole body is one editor: select across paragraphs, cut and paste whole sections, and undo as far back as you like with `Ctrl+Z` (`Cmd+Z` on a Mac). Mermaid diagrams, math, saved-query tables, PDF embeds and callouts render here too, the way the Reading view draws them, and turn back into their source when your caret lands on them. A saved query shows the rows for the note as it is on disk, so a block you are changing reads as not yet evaluated until it saves.
- **Reading**, the rendered page. Select **Reading** above the note to switch, and **Editing** to switch back; your browser remembers the choice. A read-only Vault and a public demo show this view only.
- **Source**, the whole note as Markdown with a Save button, across the full width of the page. Select **Source** above the note, or press `E`. Use it for a conflict review, or when you want to see the file exactly as it is on disk.

## Formatting

On a Mac, press `Cmd` wherever this page says `Ctrl`.

| Key | What it does |
| --- | --- |
| `Ctrl+B` / `Ctrl+I` | Bold / italic |
| `Ctrl+E` | Inline code |
| `Ctrl+Shift+X` | Strikethrough |
| `Ctrl+K` | A link, with the address selected for typing over |
| `Enter` in a list | Continues the list; `Enter` on an empty item ends it |
| `Tab` / `Shift+Tab` in a list | Indent / outdent |
| `Ctrl+F` | Opens the find bar above the note |
| `Escape` | Leaves the editor, which saves what you typed |

Select a word or a phrase and a small toolbar appears over it with bold, italic, strikethrough, code, highlight and link. Three more buttons at its end turn the line you are on into a heading, a bullet or a to-do, and back again on a second press. A selection that runs over more than one line shows no toolbar; the keys above still work on it.

The find bar has a field for what to find, with **Next**, **Previous** and **Select all**, and a field for what to put in its place, with **Replace** and **Replace all**. `Enter` in the find field goes to the next match and `Shift+Enter` to the one before. Three options narrow the search: **Match case**, **Regex** for a regular expression, and **Whole word**. The bar stays at the top of the note while you step through matches, and `Escape` closes it.

Type `/` at the start of a line for a menu of what a line can be: a heading, a bulleted or numbered list, a to-do, a quote, a callout, a code block, a divider or a table, or plain text again. Keep typing to narrow it, and `Enter` picks.

Type `[[` and part of a note's title to link to it. Pick one with the arrow keys and `Enter`, and Hatchdoor writes the link in the Vault's link style: `[[Note Title]]` in a wikilink Vault, `[Note Title](path/to/Note%20Title.md)` in a Markdown-link Vault. Only notes in the same Vault are offered. A wikilink in the editor is looked up the way the Reading view looks it up, by title, alias or path, so it reads as resolved or missing the same in both, and clicking it opens that note, at the heading when the link names one. [[Supported Markdown reference#Which link style Hatchdoor writes]] explains how the style is worked out.

Paste or drop an image or another attachment into the editor and Hatchdoor uploads it and writes the embed on a line of its own, beside your caret or where you dropped it. Images show under the line that embeds them. [[How to import and work with attachments]] covers where files land.

## On a phone

Tap into the text to edit. There is no floating toolbar on a phone, because the phone draws its own Cut, Copy and Paste over a selection in the same place; instead a bar above the keyboard carries outdent, indent, bullet, to-do, heading, bold, italic, strikethrough, code, highlight, link, undo and redo, and **Done** closes the keyboard.

## Properties

You can also edit a note's properties, the frontmatter block with its tags, status and so on, in place above the note body, in the Editing and Reading views. **Source** shows them as fields above the Markdown.

## Saving

There is no Save button in the editor. An edit saves when you leave the editor, by clicking elsewhere or pressing `Escape`, and also after about two seconds without typing. A badge above the note shows the state:

- **Saving…**: a save is under way.
- **Saved HH:MM**: everything up to that point is on disk.
- **Not saving**: saving has stopped. See below.

Hatchdoor writes the note exactly as you typed it, with the file's own line endings. Nothing reformats it.

## If the page goes away mid-edit

Your browser keeps a local copy of what you are typing, and Hatchdoor writes that copy out whenever the tab is closed or hidden. A crash, a reload, a closed tab, or Hatchdoor updating itself in the background cannot take the last few seconds of typing with it.

Reopen the note and the unsaved text is back, above a notice saying so, and the interrupted save finishes on its own. If the note changed on disk while you were away, the text is held rather than written over the newer version: the notice sends you to **Source** instead, where you can compare the two.

If your browser cannot store that local copy, in private browsing, with a full disk or with site data blocked, a notice says so, because the save is then the only thing keeping your edit.

Hatchdoor also waits before updating itself. A new version installs in the background and normally takes effect on a reload; while you have an unsaved edit or the editor has focus, that reload is held back until the edit is saved or you leave the note.

## When editing stops or isn't available

- **"Edits aren't saving. This note changed somewhere else."**: an agent, Obsidian or a Git sync wrote to this note while you were editing. Your changes are kept. Click **Review** to compare your draft with the version on disk and choose which to keep.
- **"Edits aren't saving. Hatchdoor could not reach the vault."**: a save failed for any other reason, either because the connection dropped or because the server refused it. Saving stays stopped for this note and does not start again by itself. Your text is kept in the browser: reload the page once the connection is back and the held save goes through, or select **Review** to open **Source** and save from there. If it keeps failing, the problem is on the server, and the server log records every failed save with its cause. See [[How to troubleshoot common problems]].
- **"This note changed on disk while your edit was waiting to save."**: the note moved somewhere else while your edit was stuck (a save the server refused, a connection that is down). Nothing of yours is written over: open **Source** to put the two versions side by side and decide.
- **"This note is part of a sync conflict with the Vault's remote."**: the Vault's last sync stopped because this note changed both here and on the remote. You can still edit it, but an edit made before the conflict is resolved may cause the same conflict again. [[How to troubleshoot common problems#Resolving a sync conflict]] explains how to resolve it from the Vault's settings.
- **`conflict` or `sync stopped` next to the Vault in the sidebar** does not stop your edits. Notes keep saving to disk; only the Vault's Git commit and sync wait until the problem is dealt with from the Vault's settings. See [[How to troubleshoot common problems#Git sync is failing]].
- If the Vault is read-only, or you are on a public demo, the note shows in Reading view and you can only read it.

> [!note]
> The detection behind the two messages above that say the note changed, somewhere else or on disk, depends on the filesystem holding the Vault. Hatchdoor normally saves a note by swapping the new copy with the old one in a single step, which is how it notices that something else got there first. Some filesystems cannot do that swap: ZFS before 2.2, anything mounted through FUSE, and a Windows folder used through Docker Desktop. There a save checks the note and then replaces it as two steps, and a change that lands in between is overwritten rather than caught. Your own edit is never lost either way, and Hatchdoor tells you when a Vault is in that state: the banner above the note says so, and the server log names the Vault once at startup. [[Install Hatchdoor with Docker Compose#If your notes are in a Windows folder, on ZFS or on a FUSE mount]] explains which filesystems are affected.

## Source mode

**Source** above the note, or the `E` key, opens the whole note in a Markdown editor with a Save button. It is always available, and it can change anything in the note. Opening it re-reads the Vault's link style, so a change made in Obsidian applies to the next link without restarting anything. The `[[` completion and attachment paste and drop work here as in the live editor.

---

Related: [[Browse and review through the Web UI]] · [[Supported Markdown reference]] · [[How to import and work with attachments]]
