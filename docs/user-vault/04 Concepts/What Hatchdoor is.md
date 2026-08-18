---
tags: [type/explanation, topic/architecture]
---

# What Hatchdoor is

Hatchdoor is a self-hosted web app that sits in front of a folder of plain Markdown files — an Obsidian-style Vault you already own or are starting fresh — and gives two front doors onto the exact same content: a web UI for you, and an MCP-connected agent for whichever assistant you point at it. Neither is the "real" interface; both act on the same files through the same guarded operations.

## Markdown stays authoritative

The files on disk are the source of truth, full stop. Hatchdoor builds a SQLite read model on top of them — for browsing, search (keyword and semantic), backlinks, tags, and graph data — but that database is disposable: delete it, and Hatchdoor rebuilds it by rescanning the Vault. This is a deliberate constraint, not an implementation detail. It's what makes a Vault portable: you can open the same notes in Obsidian, edit them with `git`, or drop Hatchdoor entirely, and nothing about the files themselves depends on it having ever existed.

## Two front doors, one set of rules

The Web UI edits notes directly — see [[How to edit notes with the live editor]]. An agent edits the same notes over MCP, through the guarded tools in [[MCP tools reference]]: `search_notes`, `create_note`, `edit_note`, and the rest. Both paths go through the same optimistic-concurrency-checked writes, so a human editing a note in the browser and an agent editing it a moment later can't silently clobber each other — see [[How indexing and search work]] for how a write becomes visible again.

What separates the two isn't capability so much as posture. A human reviewing a note in the browser is fundamentally different from an autonomous agent acting on your Vault unsupervised, so agent access is designed to be started narrow — read-only first — and widened deliberately. [[The security model]] covers exactly which secret gates which door, and [[Search and change notes with your agent]] covers why read-before-write is the recommended default rather than a hard rule.



## What it isn't

Hatchdoor isn't a hosted sync service, and it doesn't replace your Markdown editor of choice. There's no proprietary format to lock into and no cloud copy of your notes — the Vault is a folder you point Hatchdoor at, optionally backed by a git remote (see [[How to set up a Git-backed Vault]]) for history and sync, and Hatchdoor's job is the operational layer around that folder: indexing it, serving it, and mediating writes to it.

## Why an agent gets first-class access

Most note-taking tools are built for a single author writing to their future self. Hatchdoor's premise is that a capable agent is a second party with legitimate reasons to read and write the same Vault — filing a source you hand it, cleaning up a rough note, keeping cross-links current — and that this only works if the agent is held to the same guarantees a human editing through the browser gets: atomic writes, a current-version check before saving, and content that stays plain Markdown regardless of who touched it last. [[Why keep a second brain]] goes into what that changes about the ordinary capture–organize–distill–express rhythm, and doesn't require any particular Vault layout — see the comparison in [[Home]].

---

Related: [[Home]] · [[Why keep a second brain]] · [[The security model]] · [[How indexing and search work]] · [[MCP tools reference]]
