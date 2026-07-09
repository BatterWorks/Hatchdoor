---
tags: [type/reference]
---

# Hatchdoor — Getting Started

Hatchdoor turns a folder of Markdown files into a fast, searchable web app.

The vault stays portable: every note is still a normal `.md` file. Hatchdoor scans the vault, builds a generated SQLite cache, and refreshes that cache when files change.

## What Hatchdoor expects

- A folder containing Markdown files.
- Notes stored as `.md`.
- Local attachments stored inside or near the vault.
- A cache directory outside the vault.

Hatchdoor ignores `.hatchdoor-trash` when scanning notes. Delete actions move notes there instead of removing them permanently.

## Open and browse notes

The explorer follows your vault folders. Hatchdoor does not require a special folder scheme.

Each note gets a URL based on its file name. For example:

- `Project Plan.md` becomes a note route like `/n/project-plan`.
- Duplicate names receive a suffix such as `-2`.

## Link notes

Use Obsidian-style wikilinks:

- `[[Note Title]]`
- `[[Note Title|custom link text]]`
- `[[Note Title#Heading]]`

Links to existing notes become normal note links. Links to missing notes are shown as broken links so you can clean them up later.

## Search

Hatchdoor supports both semantic and keyword search.

- Use semantic search when you remember the meaning but not the exact wording.
- Use keyword search when exact terms matter, such as names, commands, tags, IDs, or filenames.

## Editing

When browser write mode is enabled, Hatchdoor can create, edit, move, archive, delete, and upload attachments.

When write mode is disabled, Hatchdoor is still useful as a read-only browser and search interface for an existing vault.

## Git sync

Git sync is optional. When enabled, Hatchdoor can commit and push successful vault writes after they happen.

Keep the generated SQLite cache outside the vault so git tracks only your notes and attachments.

## Related

- [[Welcome to Hatchdoor]]
- [[Hatchdoor — Markdown Feature Showcase]]
- [[Hatchdoor — Starter Vault Organisation]]
