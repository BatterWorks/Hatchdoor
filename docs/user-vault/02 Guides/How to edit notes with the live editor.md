---
tags: [type/how-to, topic/web-ui]
---

# How to edit notes with the live editor

On a writable Vault, the note you're reading *is* the note you edit — there's no separate edit mode to switch into. Click any paragraph, heading, list item, table row, or callout line and it turns into an editable field in place; everything else on the page stays exactly as rendered. This page covers how that works day to day. For the underlying Markdown syntax itself, see [[Supported Markdown reference]]; for browsing and search, see [[Browse and review through the Web UI]].

## Entering a block

- **Mouse:** click the block. The caret lands roughly where you clicked.
- **Keyboard:** `Tab` to the block, then `Enter`. Every editable block is reachable this way, not just by mouse.
- **Touch:** double-tap the line. A single tap keeps its normal job (following a link, toggling a checkbox, expanding a callout), so a stray tap while scrolling never opens an editor by accident. The first time you visit a writable Vault on a touch device, a dismissible banner reminds you: *"Double-tap a line to edit it."* — it only shows once.

Links, task-list checkboxes, and callout summary lines never open for editing on click or tap; they do what they already do (navigate, toggle, expand/collapse).

## Editable units

Each click targets one **block**, not the whole note: one paragraph, one heading, one list item, one table row, one callout line, or one fenced code block. Markdown syntax you don't normally see — `## `, `- `, `> `, the code fence — appears only inside the block you're actively editing, and disappears again once you move on. The rest of the note keeps rendering normally while you edit one piece of it.

Note properties (the frontmatter block — tags, status, and so on) are also editable inline, right above the note body.

## Moving and splitting as you type

| Key | What it does |
| --- | --- |
| `Enter` at the end of a paragraph or heading | Starts a new block below and moves you into it |
| `Enter` inside a list item | Starts the next list item, keeping the same marker and indent (and an unchecked box, if it was a task) |
| `Enter` inside a fenced code block | Inserts a literal newline, same as any text editor |
| `Shift+Enter` | A hard line break within the current block |
| `Backspace` at the very start of a block | Merges it into the block above, caret at the join |
| `Tab` / `Shift+Tab` in a list item | Indent / outdent |
| `↑` / `↓` at the top/bottom line of a block | Moves to the previous/next block, keeping your column |
| `Escape` | Commits your edit and returns to the rendered view, with focus back on that block |

These are disabled inside table rows — restructuring a table (adding rows or columns) needs Source mode, below.

## Saving

There's no Save button. Edits save automatically: when you commit a block (by clicking elsewhere, pressing `Escape`, or moving to another block) and again after about two seconds of typing without a pause. A badge above the note tells you where things stand:

- **Saving…** — a write is in flight.
- **Saved HH:MM** — everything up to that point is on disk.
- **Not saving** — autosave has stopped; see below.

## When editing stops or isn't available

- **"Edits aren't saving. This note changed somewhere else."** — someone or something else (an agent, Obsidian, a git sync) wrote to this note while you were editing. Your local changes are kept; click **Review** to compare your draft against the version on disk and choose which to keep.
- **"Edits aren't saving. Hatchdoor could not reach the vault."** — a connectivity problem. It retries once the connection is back.
- **"This note's source and rendered lines don't line up, so inline editing is off here."** — a rare safety guard that disables inline editing for that specific note rather than risk misplacing an edit. Use **Edit** to open Source mode instead.
- If the Vault is read-only, or you're on a demo deployment, no block is clickable at all — the note behaves as a plain reader.

## Source mode

The **Edit** button next to the note title opens the older, full-note Markdown editor with an explicit Save button. Use it for anything block editing can't do: restructuring a table, editing display math or raw HTML, and resolving a conflict flagged by the **Review** button above. It's always there as a fallback — nothing you can do in Source mode is off-limits, it's just not block-by-block.

---

Related: [[Browse and review through the Web UI]] · [[Supported Markdown reference]] · [[How to import and work with attachments]]
