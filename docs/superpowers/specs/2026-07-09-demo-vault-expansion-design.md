# Demo Vault Expansion

Date: 2026-07-09
Status: Approved

## Problem

The public Hatchdoor demo vault (`demo-vault/`) has 14 notes in a simple
5-folder PARA layout (`10-projects`, `20-areas`, `30-resources`, `40-archive`,
`People`). It's too sparse to show off search, backlinks, graph density, and
tag variety the way a real, lived-in vault does. The goal is to grow it into
something that feels like a real "second brain" — fictional, but rich enough
that search results, graph view, and backlinks all have something to chew on.

Inspiration comes from the live personal Hatchdoor vault's structure (via
`get_tree`), not its content — no real names, hosts, IPs, addresses, or
personal details carry over. Everything in the demo stays fictional, as it is
today.

## Non-goals

- Not migrating to a different tag or frontmatter schema than what the demo
  already uses (`tags: [demo/..., ...]`).
- Not adding real attachments/media beyond what already exists
  (`Media/demo-dashboard.png`) unless a specific note calls for a small new
  one.
- Not changing Hatchdoor's app code — this is vault content only.

## Folder restructure

Move from the current 5-folder scheme to one modeled on the real vault:

```
00-inbox/          quick captures: bookmarks, one-line ideas, a shipment/receipt-style note
10-topics/         fictional hobby/interest hub notes
  Gift ideas — [Person]/   one subfolder, 2 notes
  Hosts/                   6-8 fictional homelab host notes
20-projects/        Beacon Launch, Greenhouse Sensor Kit (existing) + 2 new
30-areas/            Operations Runbook, Content Calendar (existing) + Homelab, Family
40-reference/        existing 5 resource notes + Homelab Atlas + 5-6 new runbook/cheatsheet notes
50-people/            Lea Martin, Noa Chen (existing) + 2-3 new
90-archive/           Old CRM Trial (existing) + 1 new
_templates/           project, area (2 lightweight templates)
README.md            stays at vault root
Media/                stays at vault root
```

Existing notes are moved, not rewritten, except for internal links/prose that
reference old folder names (see "Updates to existing notes" below).

Target size: roughly 55-60 notes total (up from 14).

## Homelab Atlas

The standout new piece: a fictional homelab modeled loosely on the real
vault's `Hosts` + `Homelab Atlas` pattern, using a **"Rack" naming
convention** instead of any real prefix — e.g. `RackNAS`, `RackDock`,
`RackBrain`, `RackGate`, `RackVPN`, `RackPlay`.

- `10-topics/Hosts/` — 6-8 short host notes, one per fictional machine.
  Each note: role/purpose, fictional IP or hostname, 2-4 bullet
  "setup notes," and a link back to Homelab Atlas. Roles should cover a
  believable small homelab: a NAS, a reverse proxy/gateway, a media server, a
  VPN box, a dev/code box, a smart-home hub.
- `30-areas/Homelab.md` — ongoing area note: current focus, a short "what's
  running" summary, links into the atlas and a couple of host notes.
- `40-reference/Homelab Atlas.md` — hub note that lists and links every host,
  styled like a small infrastructure index (a table: name, role, status).

This is intentionally the richest new corner of the vault — homelab visitors
exploring the demo should find a small but coherent fleet to click through.

## Content inventory (approximate, final copy may vary slightly)

| Folder | Existing | New | Notes |
|---|---|---|---|
| `00-inbox` | 0 | 4 | bookmark capture, shopping/idea note, shipment-tracking note, quick idea capture |
| `10-topics` (root) | 0 | 7 | Coffee, Houseplants, 3D Printing, Home Cooking, Cycling, Book List, Gift Ideas hub |
| `10-topics/Gift ideas — [Person]` | 0 | 2 | two gift-idea notes for one fictional person |
| `10-topics/Hosts` | 0 | 6-8 | see Homelab Atlas above |
| `20-projects` | 2 | 2 | Home Office Refresh + one more |
| `30-areas` | 2 | 2 | Homelab, Family (generic, light-touch) |
| `40-reference` | 5 | 6-7 | Homelab Atlas + DNS setup runbook, backup runbook, coffee brew guide, book-notes entry, 2 more cheatsheet-style notes |
| `50-people` | 2 | 2-3 | new fictional collaborators, at least one linked into the homelab area |
| `90-archive` | 1 | 1 | one more archived/closed item |
| `_templates` | 0 | 2 | `project`, `area` — empty-ish skeleton notes, matches a real detail from the live vault |

All new notes follow the existing tone: short, skimmable, fictional,
frontmatter tags like `tags: [demo/..., ...]`, and at least 1-2 outgoing
wikilinks so the graph stays connected (no orphan notes unless a note is
deliberately archived/isolated).

## Updates to existing notes

- `README.md` and `30-resources/How to Explore This Demo.md` (moving to
  `40-reference/`) need their folder list and walkthrough steps rewritten for
  the new structure, and their "demo tasks" list expanded to reference a
  couple of new notes (e.g. "browse the Homelab Atlas," "open a host note").
- `What Hatchdoor Does.md` "What to try next" list stays mostly as-is; add a
  link to Homelab Atlas.
- Any existing note whose prose mentions an old folder path (e.g. "lives in
  the archive folder") gets updated to the new folder name.
- Wikilinks are resolved by title, not path, so moving files between folders
  does not break existing `[[Note Title]]` links — only prose that names a
  folder needs updating.

## Risks / open considerations

- Frontmatter tags should get a couple of new values (e.g. `topic/demo`,
  `host/demo`, `person/demo` already exists) to keep tag-browsing useful;
  final tag list is decided during implementation, following the existing
  `demo/...` + `<category>/...` pattern.
- Keep total scope to ~55-60 notes as designed — do not silently balloon
  further; if implementation reveals the count should grow meaningfully, flag
  it rather than just adding notes.
