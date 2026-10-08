---
tags: [type/explanation, topic/architecture]
---

# What Hatchdoor is

Hatchdoor is an app you run on your own computer or server. It sits in front of one or more folders of Markdown notes, called Vaults, and gives two ways into them: a website for you, and an MCP connection for your AI agent. Both work on the same files, under the same rules. A Vault can be a folder you already use with Obsidian or another Markdown app, or a new, empty one.

## Your files are the real copy

The Markdown files on disk are your notes. Hatchdoor builds a search database next to them, which lets it search by keyword and by meaning, show backlinks and tags, and draw the graph. You can throw that database away: delete it and Hatchdoor rebuilds it by reading the notes again.

So you can open the same notes in Obsidian, track them with Git, or stop using Hatchdoor, and your files never needed it.

## Two ways in, one set of rules

You edit notes in the browser; see [[How to edit notes with the live editor]]. An agent edits the same notes over MCP, with tools such as `search_notes`, `create_note` and `edit_note` from the [[MCP tools reference]]. Both go through the same checked save: a change is refused if the note changed since it was read. So if you edit a note in the browser and an agent edits it a moment later, neither overwrites the other unnoticed.

The two start with different amounts of trust. You are reviewing your own notes, and an agent acts on its own, so agent access starts read-only and you widen it when you are ready. [[The security model]] covers which password opens which door. [[Search and change notes with your agent]] covers the habit of reading before writing.

## What it isn't

Hatchdoor is not a cloud sync service, and it does not replace your Markdown editor. It has no file format of its own and keeps no cloud copy of your notes. A Vault is a folder you point Hatchdoor at, and you can back it with a Git remote for history and sync (see [[How to set up a Git-backed Vault]]). Hatchdoor does the work around that folder: it indexes it, serves it, and checks every change made to it.

## Why an agent gets full access

Most note apps assume one author writing for their future self. Hatchdoor assumes an agent can be a second, useful author: filing a source you hand it, tidying a rough note, keeping links between notes current. That only works if the agent gets the same guarantees you get in the browser. A save happens completely or not at all, it is checked against the current version first, and the result is plain Markdown whoever wrote it. [[Why keep a second brain]] says more about what that changes. None of it needs a particular folder layout, and [[Home]] compares a few.

---

Related: [[Home]] · [[Why keep a second brain]] · [[The security model]] · [[How indexing and search work]] · [[MCP tools reference]]
