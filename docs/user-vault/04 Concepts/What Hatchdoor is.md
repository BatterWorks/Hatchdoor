---
tags: [type/explanation, topic/architecture]
---

# What Hatchdoor is

Hatchdoor is an app you run on your own computer or server. It sits in front of one or more folders of Markdown notes, called Vaults, and gives two ways into them: a website for you, and an MCP connection for your AI agent. Both work on the same files, under the same rules. A Vault can be a folder you already use with Obsidian or another Markdown app, or a new, empty one.

## Your files are the real copy

The Markdown files on disk are your notes. Hatchdoor builds a search database next to them, which lets it search by keyword and by meaning, show backlinks and tags, and draw the graph. That database is throwaway: delete it and Hatchdoor rebuilds it by reading the notes again.

This is deliberate. It means you can open the same notes in Obsidian, track them with Git, or stop using Hatchdoor entirely, and the files never depended on it.

## Two ways in, one set of rules

You edit notes in the browser; see [[How to edit notes with the live editor]]. An agent edits the same notes over MCP, with tools such as `search_notes`, `create_note` and `edit_note` from the [[MCP tools reference]]. Both go through the same checked save: a change is refused if the note changed since it was read. So if you edit a note in the browser and an agent edits it a moment later, neither silently overwrites the other.

What differs is how much trust each starts with. You, reviewing notes in the browser, are not the same as an agent acting on its own, so agent access starts narrow, read-only, and you widen it when you are ready. [[The security model]] covers which password opens which door. [[Search and change notes with your agent]] covers the read-before-write habit.

## What it isn't

Hatchdoor isn't a cloud sync service, and it doesn't replace your Markdown editor. There is no special file format and no cloud copy of your notes. A Vault is a folder you point Hatchdoor at, optionally backed by a Git remote for history and sync (see [[How to set up a Git-backed Vault]]). Hatchdoor's job is everything around that folder: indexing it, serving it, and checking every change made to it.

## Why an agent gets full access

Most note apps assume one author writing for their future self. Hatchdoor assumes an agent can be a second, useful author: filing a source you hand it, tidying a rough note, keeping links between notes current. That only works if the agent is held to the same guarantees as you in the browser: a save happens completely or not at all, it is checked against the current version first, and the result is still plain Markdown whoever wrote it. [[Why keep a second brain]] says more about what that changes. None of it needs a particular folder layout; [[Home]] compares a few.

---

Related: [[Home]] · [[Why keep a second brain]] · [[The security model]] · [[How indexing and search work]] · [[MCP tools reference]]
