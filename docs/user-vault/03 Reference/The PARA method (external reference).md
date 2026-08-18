---
tags: [type/reference, topic/vault-organization]
---

# The PARA method (external reference)

PARA is a general-purpose way to organize a Vault's folders, independent of Hatchdoor and of any agent workflow. This page is a dictionary of the method itself, for anyone deciding how to lay out a new Vault, with links to the primary source.

## Primary sources

| Source | Link |
| --- | --- |
| "The PARA Method: The Simple System for Organizing Your Digital Life" — Tiago Forte, Forte Labs | [fortelabs.com/blog/para](https://fortelabs.com/blog/para/) |
| *The PARA Method* (book) | [buildingasecondbrain.com/para](https://www.buildingasecondbrain.com/para) |
| Forte Labs | [fortelabs.com](https://fortelabs.com/) |

## The four categories

PARA sorts everything into exactly four categories, in order of how actionable they are:

| Category | Definition | 
| --- | --- |
| **Projects** | "Short-term efforts (in your work or personal life) that you take on with a certain goal in mind" — e.g. completing a webpage design, buying a computer, finishing a language course. |
| **Areas** | "Important parts of your work and life that require ongoing attention" — work domains (Marketing, Product Management) and personal ones (Health, Finances, Home). |
| **Resources** | Topics "you're interested in and learning about" — graphic design, gardening, photography — without a specific outcome attached. |
| **Archives** | "Anything from the previous three categories that is no longer active, but you might want to save for future reference" — completed projects, inactive areas, abandoned resource interests. |

## The distinctions that actually matter

**Projects vs. Areas** is the one people get wrong most often. The difference is permanence, not importance: an Area is an ongoing responsibility that "continues indefinitely," while a Project has a defined endpoint. Forte's example: listing "strategic planning" or "vacations" as Projects is a mistake — they never finish. Anything that "will end or change" is a Project; anything you maintain indefinitely is an Area.

**Resources vs. Archives**: a Resource is a topic you're still actively engaged with; an Archive is anything — former Project, former Area, former Resource — you're keeping only for future reference, not current use.

## Why it's meant to work across any tool

Forte designed PARA to apply "across any platform" — a computer's file system, cloud storage, or a note-taking app — because it organizes by how actionable something is, not by any one tool's features. That portability is also why it needs nothing special from the tool it's used in: PARA is four folders and a filing habit, not a feature.

## How this shows up in a Hatchdoor Vault

Hatchdoor's own starter Vault layout (`10-topics/`, `20-projects/`, `30-areas/`, `40-reference/`, `90-archive/`) is a PARA variant — "Topics" in the Resources role, plus a numbered `00-inbox/` for unsorted capture that PARA itself doesn't specify. Nothing in Hatchdoor requires this or any other layout; folders are just folders.

One genuine point of contact: `archive_note` (both the MCP tool and the Web UI action) moves a note into a Vault's configured archive folder in one step, which operationalizes exactly what PARA's Archives category asks for — see [[MCP tools reference]]. Beyond that, PARA and Hatchdoor's [[The layer system|layer system]] solve different problems and can be used together without conflict: PARA organizes *where* a note lives; layers control whether it shows up in *default* search and browsing, regardless of which PARA folder it's in.

---

Related: [[MCP tools reference]] · [[The layer system]] · [[The LLM wiki pattern (external reference)]] · [[The Second Brain method (external reference)]] · [[The Zettelkasten method (external reference)]]
