---
tags: [type/how-to, topic/vault-organization]
---

# How to choose a folder layout

Hatchdoor works with any folder layout, including none at all. This page shows one simple layout for a new Vault, for anyone who would rather not start from a blank folder.

Use it if you want a starting point. Ignore it if your notes already have a system that works: Hatchdoor search and links matter far more than tidy folders.

## Suggested folders

- `00-inbox/` for quick capture: anything you have not sorted yet.
- `10-topics/` for subjects you expect to come back to and keep adding to.
- `20-projects/` for active efforts with an outcome or a next step.
- `30-areas/` for ongoing responsibilities that never finish.
- `40-reference/` for stable material: guides, checklists, saved research.
- `90-archive/` for anything finished or no longer active.

The numbers only keep the folders in a fixed order in the explorer and in file browsers. This is a variant of the PARA method, with Topics standing in for PARA's Resources and an inbox added; [[The PARA method (external reference)]] explains the original.

## Where a note goes

Use `10-topics/` for ideas and subjects that grow over time, such as Coffee, Home networking or Productivity.

Use `20-projects/` for outcomes you are working towards, such as Launch a personal website, Plan the kitchen renovation or Prepare a conference talk.

Use `30-areas/` for things you look after, such as Health, Finances or Home.

Use `40-reference/` for material you want to keep: setup notes, checklists, manuals, technical references, saved research.

Use `90-archive/` when something is no longer active but may still be useful. The **Archive** action on a note in the Web UI, and `archive_note` for an agent, moves the note there in one step and keeps its links working. `90-archive/` is Hatchdoor's default archive folder; a Vault's **Archive folder** setting can point it somewhere else.

Moving or renaming a note through Hatchdoor, from the note's actions in the Web UI or with an agent, updates every link that points at it, so changing your mind later costs nothing.

## Index notes

Once a folder grows, one short note that links to its most important notes helps you and your agent find your way around, for example a note called Projects index in `20-projects/`. Keep it short: link to the notes and leave the details in them.

## Keep it flexible

A folder layout should save you time. Not sure where something goes, or deciding takes too long? Put it in `00-inbox/` and move it later.

If you want to keep some notes out of everyday search without moving them, such as old journals or raw imports, that is what layers are for: see [[How to organize a Vault with layers]].

Other ways to organize a Vault, which you can mix with this one: [[The Zettelkasten method (external reference)]] links small notes densely and barely uses folders, and [[How to run an LLM wiki in Hatchdoor]] lets an agent build and maintain the structure for you.

---

Related: [[The PARA method (external reference)]] · [[How to organize a Vault with layers]] · [[How to manage multiple Vaults]]
