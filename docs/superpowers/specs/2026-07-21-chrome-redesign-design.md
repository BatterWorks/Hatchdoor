# Chrome redesign — working design

**Date:** 2026-07-21, revised 2026-07-28 (sidebar zones session)
**Status:** In progress (brainstorming). Not yet a ratified spec — see "Proposed" and "Open" sections.
**Issues covered:** #12 (sidebar layout), #8 (overloaded submenu), #10 (messy header hierarchy), placement side of #11 (create note).

---

## Context / why this exists

Issue #12 ("sidebar layout is odd") turned out not to be separable from the surrounding
chrome. The complaints across #8/#10/#12 share one root: the app's chrome mixes different
*kinds* of things in the same surfaces, so nothing has a clear job.

Concretely, today:

- The **sidebar** stacks three note lists top-to-bottom — Recently Viewed (client, notes you
  opened), Last Modified (server `/api/recently-modified`, notes changed on disk), and the
  Folder tree. The two recency lists overlap heavily and each independently applies the
  active-note highlight, so opening a note lights it up in all three at once (the literal #12
  bug). The tree — the actual navigation backbone — is buried below both lists.
- The **sidebar header** ("Vault Explorer" + a bare "New" button + Stats/Graph) puts
  whole-vault views and an ambiguous create action in prime real estate (#10). The "New"
  button reads as an explorer action, not "create a document".
- The **topbar ··· menu** is an overloaded grab-bag of nine items mixing destructive
  note-CRUD (edit/rename/move/archive/delete) with utilities (copy/download/link) and
  create — all all-caps, boxed, centered (#8).

Design system: the app follows the "Forge" direction — warm cream / near-black theming, a
single hot-orange accent (`#ff4d1c`), zero border-radius (except kbd + status pills), a
four-family type system (Bricolage display / Newsreader serif / Inter Tight sans / JetBrains
mono). Living reference: `docs/design/design-system.html` (§05 = sidebar, §18 = popover menu). Token
source: `frontend/src/styles/base.css`.

It is also a **mobile app** (PWA). On mobile the sidebar is a slide-in drawer (breakpoint
920px) and the topbar is the always-visible chrome. Mobile button budget is the hard
constraint: the top row currently holds **4 buttons** — ☰ drawer, ⌕ search, ◑ theme, ···
menu.

Guiding principle proposed for the whole redesign: **group actions by what they act *on*, and
put each group where that thing lives** (note-actions with the note, app-globals in the
topbar, navigation in the sidebar, awareness in its own signal).

---

## Decided

1. **Scope:** one coherent chrome redesign, not just the sidebar — #12 + #8 + #10 + the
   placement side of #11.
2. **Sidebar = navigation only.**
3. **Recently Viewed:** keep it, but make it **collapsible** (remembers open/closed state;
   fine to leave closed for users who don't care).
4. **"Last Modified" leaves the sidebar** — it was conflating awareness with navigation.
5. **Change-awareness = a badge you click to open a panel** that lists the changed notes.
   - Does **not** live in the sidebar.
   - Notification semantics: **"since you last looked" = since you last opened the panel.**
     Opening the panel is the acknowledgement — it resets the count and stamps "last seen =
     now" (client-side). Changes after that re-accumulate.
   - **Your own UI edits do not count** — only changes that arrived externally (MCP agents,
     git sync, other devices) via the vault-event stream. That is what makes the badge mean
     "something happened behind my back".
   - **Caps:** badge count display capped (e.g. `9+`); panel shows ~15 most-recent, newest
     first, with a quiet "and N more" if truncated. Bulk-change collapsing (e.g. "142 notes
     changed · sync") is a possible later refinement, **not** v1.
6. **Active-note highlight is canonical in the tree only** (full hot rail + hot-soft
   background). Recently Viewed does not re-highlight the current note. This removes the
   multi-highlight bug by construction.
7. ~~**Stats / Graph → a visually-separated sidebar footer.**~~ **Superseded 2026-07-28 by
   decision #17** — they move to the sidebar *rail* instead. The reasoning holds unchanged
   (they are real whole-vault views, the problem was only prominence, and they stay out of the
   already-crowded topbar); only the destination changed, so the footer can hold the single
   primary create action alone.
8. **Folder-name numeric prefixes** (`10-topics`, `20-projects`, `30-areas`, `40-reference`)
   are the real folder names and encode a deliberate order — shown verbatim, never stripped or
   reformatted.
9. **The topbar needs a genuine overhaul,** not a restyle — it is already full and its ···
   menu is an overloaded grab-bag.
10. **Note-actions move out of the topbar and onto the note header.** The note-specific
    actions (Edit / Rename / Move / Archive / Delete / Copy content / Download .md / Copy
    link) leave the topbar ··· grab-bag and live on the note itself, so they only exist when
    there is a note to act on. **New note** is app-global (must exist with no note open) and
    does **not** move here — it stays a global action. This is the pivot that makes the topbar
    overhaul possible: note-CRUD stops competing with app-level chrome.
11. **The note "Properties" line becomes the disclosure toggle**, replacing the separate
    Show/Hide button. It uses the **same caret affordance as the sidebar folder rows**
    (▸ collapsed → ▾ open, rotating, hot accent when open); clicking the line expands/collapses
    the properties grid. Collapsed state stays persisted per-note (existing
    `propertiesCollapsedStorageKey`).
12. **The freed Show-button slot becomes the "Notes" button** — the note-actions menu from
    decision #10, anchored on the note header at the right of the Properties line. Resulting
    row: `▸ Properties … [ Notes ]`.
    - **Always render this row on a note, even with zero frontmatter.** Today `NoteProperties`
      returns `null` when there are no properties (`sections.tsx:21`); that must change so the
      Notes button is always present on a note (the caret simply has nothing to expand).
    - The dormant `hatchdoor:toggle-note-properties` window event (listener in `NotePage.tsx`,
      no dispatcher) can be repurposed or removed as part of this.

### Still to settle under this decision

- **Mobile button budget.** Working rule: **no net new mobile buttons** (~4 is the ceiling:
  ☰ / ⌕ / ◑ / ···). *Amended 2026-07-28:* the change-badge no longer competes for a slot — it
  lives in the sidebar rail (decision #17), which is inside the drawer and outside this budget.
  Now that note-CRUD has left the topbar, revisit what — if anything — the topbar ··· still
  holds, and where the set-once theme toggle lives.
- **The inline "Edit" button** in the title row (`note-edit-button`): keep as a visible
  one-tap primary, or fold entirely into the Notes menu? (Leaning: keep Edit visible, put the
  rest behind Notes.)

---

---

## Sidebar zones (decided 2026-07-28)

Prompted by a Notion sidebar reference. The pattern borrowed is **structural only** — three
fixed zones and per-row identity. The Forge skin is explicitly unchanged: no rounding, no grey
palette softening, no new type families.

The unlock: the rail lives *inside the sidebar*, so its buttons do **not** consume the
four-slot mobile topbar budget. That is what let #13 and #14 resolve here rather than waiting
on the topbar pass.

16. **The sidebar becomes three zones:** a fixed rail at top, a scrolling nav in the middle, a
    fixed footer at bottom. Only the middle scrolls. Today the whole `explorer-pane` is the
    scroll container and reports `scrollTop` upward (`ExplorerPane.tsx:59`); that handler moves
    onto the middle div and its consumer in `App.tsx` follows.

17. **Rail contents: Stats · Graph · Changes … Settings.** Icon-only, so each needs
    `aria-label` + `title`. Stats and Graph stay `NavLink`s with their active state. Changes is
    a button carrying the count (the badge from decision #5).
    - **Settings is rightmost and visually separated** (gap or hairline): the first three are
      places you go, Settings is app configuration.
    - Settings has no destination yet. It ships **visibly disabled** (dimmed, `aria-disabled`,
      tooltip) so it reserves the slot without offering a dead click, and the layout does not
      shift when it becomes real.
    - **No Home icon.** Hatchdoor has no home concept — `/` renders `EmptyState`. The topbar ☰
      opens the panel and that is the whole story.
    - **Search stays in the topbar ⌕** and gets no rail icon: search must work with the drawer
      closed, and two entry points would drift. Same reasoning keeps the theme toggle out.
    - This deletes the entire `explorer-header` block — the "Vault Explorer" label, the "New"
      button, and the `explorer-page-links` div (`ExplorerPane.tsx:63-94`).

18. **Footer holds one thing: the global "New note" button**, full width, only when
    `writeEnabled`. Resolves the parked global-create placement. Keeping it alone is
    deliberate — a footer with three things in it becomes the next grab-bag. The per-folder `+`
    stays as contextual creation. The create *flow* remains issue #11's job.

19. **Row marks: folders keep the caret, notes get one uniform mark.** This is the minimum fix
    for "folder rows and note rows look nearly identical". Explicitly **no per-note emoji**, no
    per-note icon frontmatter, no colour coding — nothing to author and nothing to maintain.

20. **Recently Viewed loses the active-note class** (`Explorer.tsx:33-37`). This is the
    mechanical change that enforces decision #6 and kills the multi-highlight bug.

21. **`LastModifiedNotesList` is deleted** (enacting decision #4). Its `modifiedNotes` data
    reroutes to the changes panel.

22. **No mobile mirror for the changes count.** The count lives only in the rail. Because the
    rail is inside a drawer that is closed by default on mobile, there is **no indication
    anywhere** that notes changed until you open the drawer. Considered and rejected: a dot on
    the ☰ button. Accepted tradeoff, recorded deliberately.

### Dependency this exposes

Decision #5 says the badge counts only externally-arrived changes, not your own UI edits. The
SSE stream (`useVaultTree.ts:75`) only bumps a revision counter and carries no per-note detail,
and `/api/recently-modified` cannot distinguish your edit from an agent's. **The rail slot and
the panel shell are buildable now; the "external only" counting rule needs backend work that
does not exist yet.** Do not treat the badge as complete without it.

### Testing

`ExplorerPane` has no covering tests today. Add with this work: the rail renders all four
targets; the footer create button is absent when `writeEnabled` is false; Recently Viewed does
not carry `active-note` while the tree does.

### Out of scope for this pass

Topbar overhaul, the note-header "Notes" button (decisions #10–#12), and the create flow (#11).

---

## Open / not yet designed

13. ~~**Change-awareness badge placement.**~~ **Resolved 2026-07-28** — the sidebar rail
    (decision #17), with no mobile mirror (decision #22).
14. ~~**Global "new note" placement.**~~ **Resolved 2026-07-28** — the sidebar footer
    (decision #18). The create *flow* is still issue #11's job.
15. **Folder tree UX/UI review.** Partially addressed — decision #19 settles the folder-vs-note
    distinction. Still open from the original observations:
    - Depth reads weakly — nesting is a thin left border plus a small indent.
    - The open/closed cue is only a tiny 8px caret.
    - Long titles truncate with no mobile-friendly way to see the full name.
    - No per-folder count/affordance.

---

## Relevant code

- Sidebar container: `frontend/src/app/ExplorerPane.tsx`
- Tree + the two recency lists: `frontend/src/components/Explorer.tsx`
- Sidebar styles: `frontend/src/styles/layout-explorer.css`
- Topbar (+ the ··· menu): `frontend/src/app/AppTopbar.tsx`, `frontend/src/styles/topbar.css`
- Data: Recently Viewed is client/localStorage (`App.tsx`); Last Modified is server
  (`useVaultTree.ts` → `/api/recently-modified`); external changes stream via
  `/api/vault-events`.
