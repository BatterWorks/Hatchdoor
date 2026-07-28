# Chrome redesign — working design

**Date:** 2026-07-21, revised 2026-07-28 (sidebar zones session)
**Status:** In progress (brainstorming). Not yet a ratified spec — see "Proposed" and "Open" sections.
**Issues covered:** #12 (sidebar layout), #10 (messy header hierarchy), #11 (create note, both
placement and interaction), #8 (submenu presentation).

> **Numbering convention.** `D1`–`D30` refer to *decisions in this document* (the numbered
> lists below). A bare `#8`, `#11`, `#12` always means a **GitHub issue**. These collide by
> coincidence — decision D8 is folder prefixes, issue #8 is the submenu — so cross-references
> always carry the `D` or the word "issue".

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

1. **Scope:** the sidebar (#12), the note-header Properties toggle (#10), the create-note
   dialog (#11, both placement and interaction), and submenu presentation (#8). *Amended
   2026-07-28:* the topbar's **structure** is out of scope — icons, menu contents, and order are
   frozen — but the menu's **presentation** is in.
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
   decision D17** — they move to the sidebar *rail* instead. The reasoning holds unchanged
   (they are real whole-vault views, the problem was only prominence, and they stay out of the
   already-crowded topbar); only the destination changed, so the footer can hold the single
   primary create action alone.
8. **Folder-name numeric prefixes** (`10-topics`, `20-projects`, `30-areas`, `40-reference`)
   are the real folder names and encode a deliberate order — shown verbatim, never stripped or
   reformatted.
9. ~~**The topbar needs a genuine overhaul,** not a restyle.~~ **Reversed 2026-07-28: the
   topbar's *structure* is not changing.** Nothing is added, removed, or reordered; every
   control stays in its current position; the ··· menu keeps its contents. The observation that
   the menu is an overloaded grab-bag still stands and is simply not being acted on now.
   - *Clarified after the icon decision:* "structure frozen" is not "file untouched". The
     earlier wording said "icons stay exactly where they are", which meant **placement** but
     read as "do not touch the icons at all". Presentation changes are in scope — the ··· menu's
     rows (D24–D26) and the glyph swap to Material Symbols Sharp (D32) both edit
     `AppTopbar.tsx`. What is frozen is *what is there and in what order*, not *how it renders*.
10. ~~**Note-actions move out of the topbar and onto the note header.**~~ **Dropped 2026-07-28**
    as a consequence of the topbar reversal. Edit / Rename / Move / Archive / Delete / Copy
    content / Download .md / Copy link all stay in the topbar ··· menu where they are today.
11. **The note "Properties" line becomes the disclosure toggle**, replacing the separate
    Show/Hide button (`sections.tsx:30-37`). It uses the **same caret affordance as the sidebar
    folder rows** (▸ collapsed → ▾ open, rotating, hot accent when open); clicking the line
    expands/collapses the properties grid. Collapsed state stays persisted per-note (existing
    `propertiesCollapsedStorageKey`). `aria-expanded` and `aria-controls` move from the removed
    button onto the new clickable header.
    - **This decision survives the topbar reversal intact** and is now standalone: it is a
      note-header change with no topbar dependency.
    - The dormant `hatchdoor:toggle-note-properties` window event (listener at
      `NotePage.tsx:214-219`, no dispatcher anywhere) is **removed** as part of this.
12. ~~**The freed Show-button slot becomes the "Notes" button.**~~ **Dropped 2026-07-28** —
    with decision D10 gone there are no note-actions to house, so the freed slot simply closes
    up. The row is just `▸ Properties`.
    - The related requirement to *always* render the row on a note with zero frontmatter goes
      away with it. `NoteProperties` keeps its existing `return null` when there are no
      properties (`sections.tsx:22-24`).
    - The inline "Edit" button in the title row (`note-edit-button`, `NotePage.tsx:562-569`)
      **stays visible as-is**. The open question was whether to fold it into the Notes menu;
      there is no Notes menu now, so it is settled by default.

### Note on issue #8

Briefly dropped when the topbar left scope, then **re-entered on 2026-07-28 as a presentation-only
change** — see the issue #8 section below. The distinction that makes both true: the topbar's
structure (icons, menu contents, order) is frozen; how the menu renders its rows is not.

---

## Sidebar zones (decided 2026-07-28)

Prompted by a Notion sidebar reference. The pattern borrowed is **structural only** — three
fixed zones and per-row identity. The Forge skin is explicitly unchanged: no rounding, no grey
palette softening, no new type families.

The unlock: the rail lives *inside the sidebar*, so its buttons do **not** consume the
four-slot mobile topbar budget. That is what let D13 and D14 resolve here rather than waiting
on the topbar pass.

16. **The sidebar becomes three zones:** a fixed rail at top, a scrolling nav in the middle, a
    fixed footer at bottom. Only the middle scrolls. Today the whole `explorer-pane` is the
    scroll container and reports `scrollTop` upward (`ExplorerPane.tsx:59`); that handler moves
    onto the middle div and its consumer in `App.tsx` follows.

17. **Rail contents: Stats · Graph · Changes … Settings.** Stats and Graph stay `NavLink`s with
    their active state. Changes is a button carrying the count (the badge from decision D5).
    - **Icon-only, resolved 2026-07-28** — see D31. Briefly blocked when review found Forge had
      no icon set; unblocked by adopting Material Symbols Sharp. Each icon still needs an
      `aria-label` since there is no visible text.
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
    stays as contextual creation. The create *flow* is designed in the issue #11 section below (D29, D30).

19. **Row marks: folders keep the caret, notes get a mono index.** *Revised 2026-07-28 after
    design-system review.* The original wording said "one uniform mark"; the system already
    specifies the answer and it is better than a geometric mark. §05: *"Folders use a CSS-only
    chevron caret that rotates open. Notes show a mono index."* The documented note row is
    `<a class="note-link"><span class="idx">001</span>Title</a>`.
    - The index uses the mono utility face already in the four-family system, so it introduces
      no new device, and it encodes ordering rather than decorating.
    - `NoteNode` (`Explorer.tsx:208-230`) currently renders no index at all. This is
      implementation drift from a correct system, the same failure mode as issue #8.
    - Still explicitly **no per-note emoji**, no icon frontmatter, no colour coding.

20. **Recently Viewed loses the active-note class** (`Explorer.tsx:33-37`). This is the
    mechanical change that enforces decision D6 and kills the multi-highlight bug.

21. **`LastModifiedNotesList` is deleted** (enacting decision D4). Its `modifiedNotes` data
    reroutes to the changes panel.

22. **No mobile mirror for the changes count.** The count lives only in the rail. Because the
    rail is inside a drawer that is closed by default on mobile, there is **no indication
    anywhere** that notes changed until you open the drawer. Considered and rejected: a dot on
    the ☰ button. Accepted tradeoff, recorded deliberately.

23. **Sidebar section heads adopt the documented `.side-head`.** §05 specifies
    `01 · RECENT · ──────— · 04`: a mono section number, an uppercase display-face label, a
    hairline rule filling the gap, and a mono count. The implementation instead uses a plain
    `<p class="recent-notes-title">` and a separate `explorer-notes-label` (`ExplorerPane.tsx:118`).
    Adopting `.side-head` gives the "Notes 142" count a documented home (`.side-count`) and makes
    both sections structurally identical.
    - *Caveat worth recording:* the `.side-num` counters (01, 02) are decorative here. With only
      two sections, the numbering encodes nothing a reader needs. Adopt the head structure;
      treat the numerals as optional and drop them unless they earn their place.

### Icon system (decided 2026-07-28)

**Forge had no icon set.** The topbar's apparent icons are literal unicode characters in JSX
(`☰`, `◑`, `···`, `+`); `frontend/scripts/gen-icons.mjs` generates PWA app icons only. This
briefly blocked D17. A typographic rail (`.side-label` device, uppercase labels instead of
glyphs) was the alternative considered. Resolved by adopting a library instead.

31. **Adopt Material Symbols Sharp.**
    - **Licence: Apache 2.0.** Free for commercial and open-source use, no in-UI attribution;
      a NOTICE file in the repo satisfies it. This was the deciding constraint.
    - **Geometry:** the Sharp variant is drawn with right angles and squared terminals, the only
      mainstream set that agrees with the zero-radius rule. Outlined and Rounded do not.
    - **Variable axes:** the weight axis lets icon stroke sit with the display face rather than
      fighting it. The `fill` axis gives the active state for free — outline when inactive,
      filled when active — mapping onto the existing hot-accent treatment.
    - **No dependency is installed.** For ~9 icons, copy the SVGs out of
      `google/material-design-icons` and inline them as components. No icon font, no npm
      package, no CDN request. This matters: Hatchdoor is a PWA and the offline path stays clean.
    - **Rejected:** Pixelarticons (closest geometric match to the rect-built wordmark, but went
      freemium — 816 free under CC BY 4.0, full set paid — and reads 8-bit against the serif);
      Lucide, Feather, Heroicons, Tabler, Phosphor (all permissive, all rounded caps and
      corners, would read as imported from another product).
    - **Honest caveat:** Material Symbols is a stroke language with uniform line weight, while
      the wordmark is solid filled rects. Sharp is *compatible* with zero-radius, not identical
      in construction. Closest permissive match, not a perfect one.

32. **The topbar's unicode glyphs are replaced with Sharp icons.** This stays inside the topbar
    freeze on the same line drawn for issue #8: **positions and order do not move, presentation
    does.** Nothing is added, removed, or reordered.
    - Leaving the topbar on unicode was not an option once the rail has drawn icons directly
      below it: four typed characters beside four real icons reads as broken. Unicode glyphs
      also render per-platform and ignore the weight axis, so they would drift in size and
      stroke against the rail.

    | Where | Now | Sharp icon |
    |---|---|---|
    | Rail | — | `bar_chart` (Stats) |
    | Rail | — | `graph_3` (Graph) |
    | Rail | — | `inbox` (Changes) |
    | Rail | — | `settings` (Settings) |
    | Topbar | `☰` | `menu` |
    | Topbar | `⌕` | `search` |
    | Topbar | `◑` | `light_mode` / `dark_mode` |
    | Topbar | `···` | `more_horiz` |
    | Sidebar | `+` | `add` |

    - **The theme toggle becomes two icons, not one.** Today `◑` is a static half-circle
      whatever the state, which does not say what pressing it does. Showing the mode you would
      switch *to* is a real improvement that touches only the glyph.
    - **Left alone:** the folder caret (§05 specifies CSS-only, and it works) and the Hatchdoor
      wordmark (hand-built rects, it is the brand).

*Still open:* a permanently disabled Settings control is conspicuous. If it ships disabled it
needs a tooltip saying what it will be, not a bare dead control. Deferring it until issue #13 is
built is the other option. **Not yet decided.**

### Dependency this exposes

Decision D5 says the badge counts only externally-arrived changes, not your own UI edits. The
SSE stream (`useVaultTree.ts:75`) only bumps a revision counter and carries no per-note detail,
and `/api/recently-modified` cannot distinguish your edit from an agent's. **The rail slot and
the panel shell are buildable now; the "external only" counting rule needs backend work that
does not exist yet.** Do not treat the badge as complete without it.

### Testing

`ExplorerPane` has no covering tests today. Add with this work: the rail renders all four
targets; the footer create button is absent when `writeEnabled` is false; Recently Viewed does
not carry `active-note` while the tree does.

### Out of scope for this pass

The topbar's *structure* (see D9 — `AppTopbar.tsx` is still edited for D24–D26 and D32) and the
create flow (issue #11). The note-header work is limited to decision D11's Properties toggle.

---

## Issue #8 — submenu presentation (decided 2026-07-28)

Re-entered scope after being dropped. **This does not reopen the topbar**: contents, order, and
icon placement are untouched (decision D9's reversal stands). Only how the menu *presents* its
items changes, which is what the issue asks for.

The issue reads: *"boxes in boxes isn't good design, all caps text and centre alignment is
harder for people to visually scan."* All three complaints are accurate, and two of them are
already fixed in the codebase for the wrong audience.

### Root cause

The correct treatment exists only inside `@media (max-width: 920px)`
(`responsive.css:65-72`): `justify-content: flex-start`, `text-transform: none`,
`letter-spacing: 0`. **Mobile is already correct. Desktop is not.** Above 920px the rows fall
through to the `.close-note` base (`ui-common.css:3-25`), which is `text-transform: uppercase`,
`letter-spacing: 0.12em`, `justify-content: center`, plus a 1px border per row.

The design system agrees with the fix and has since it was written — §18 specifies
`.menu .ui-btn { justify-content: flex-start; text-transform: none; letter-spacing: 0; }`.

24. **Hoist the alignment, case, and tracking rules out of the mobile media query** into
    `topbar.css` as unconditional `.topbar-menu .close-note` rules. The media query keeps only
    what is genuinely mobile: `min-height: 44px` (touch target) and the larger font size.

25. **Menu rows lose their per-row border.** Only the popover is bordered; rows are borderless
    and indicate hover with a `--paper-2` fill. This is the "boxes in boxes" fix and it is the
    one part the system does **not** already prescribe: `.ui-btn.ghost` keeps
    `border-color: var(--rule)`, so §18's own mock nests bordered buttons inside a bordered
    popover. **The design system needs amending here, not just the code.**

26. **Hairline dividers group the nine items** into navigate / utility / destructive. Grouping
    is presentation, not restructuring, so it stays inside the topbar freeze. Order within each
    group is unchanged.

```
BEFORE (desktop >920px)          AFTER
┌────────────────────┐           ┌────────────────────┐
│ ┌────────────────┐ │           │  Edit              │
│ │      EDIT      │ │           │  Rename            │
│ └────────────────┘ │           │  Move              │
│ ┌────────────────┐ │           │ ·················· │
│ │     RENAME     │ │           │  Copy content      │
│ └────────────────┘ │           │  Download .md      │
│ ┌────────────────┐ │           │  Copy link         │
│ │  COPY CONTENT  │ │           │ ·················· │
│ └────────────────┘ │           │  Archive           │
│       ...          │           │  Delete            │
└────────────────────┘           └────────────────────┘
```

---

## Issue #11 — create-note dialog (decided 2026-07-28)

The issue reads: *"hard to visually differentiate the labels from the forms as the text looks
similar and the forms are VERY subtle. The folder selector interaction is very odd. I would use
a dropdown with the option to make a new topic folder."*

### Root cause

**The dialog is not in the design system.** There is no fields section and no dialog section —
the system covers tokens, type, topbar, sidebar, drawer, note blocks, prose, code, tables,
search dialog, popover, states, badges, and buttons, and stops there. With nothing to build
from, `NoteActionsDialog` invented its own vocabulary, and it contradicts the system's defining
rule:

| Element | Current | System |
|---|---|---|
| `.modal-panel` | `border-radius: 8px` | zero radius except kbd + status pills |
| `.modal-panel input` | `border-radius: 6px`, `1px var(--rule)` | `.search-input`: zero radius, `var(--rule-strong)`, hot border on focus |
| `.folder-suggestions button` | `border-radius: 4px`, bespoke | should be a system button |
| `.modal-panel textarea` | hardcoded `ui-monospace, SFMono-Regular, Menlo…` | `var(--font-mono)` (JetBrains Mono) |
| `.modal-panel label` | no treatment, default text | — nothing documented |

The `--rule` border on `--paper` at 1px is measurably the weakest field treatment available in
the palette, which is the "VERY subtle" complaint. Labels having literally no treatment is the
"labels look like the forms" complaint.

27. **Labels adopt the `.side-label` idiom** already used for sidebar section heads: display
    face, uppercase, 0.12em tracking, ~0.72rem, `--muted`. Labels become chrome; field text
    becomes content. Reuses an existing device rather than inventing a second label style.

28. **Fields adopt `.search-input`** (`--rule-strong`, zero radius, `--font-sans`, hot border on
    focus). The panel drops its 8px radius. The textarea drops its hardcoded stack for
    `var(--font-mono)`.

29. **The folder picker becomes a `<select>`**, replacing the free-text input plus the flat wall
    of every folder rendered as a chip (`FolderSuggestions`, `NoteActionsDialog.tsx:136-155`).
    The list shows folder names verbatim including numeric prefixes (decision D8). A final
    `New folder…` option reveals a text input for the new folder name.
    - `FolderSuggestions` is deleted, along with `.folder-suggestions` in `App.css:269-290`.
    - `MoveForm` uses the same picker — worth sharing one component between create and move
      rather than diverging.

30. **A live path line sits below the name field:** `10-topics / Weekly review.md`, mono,
    updating as you type. Nothing currently tells you what you are about to create or where.
    This makes the outcome legible at the moment it matters and surfaces the numeric prefixes
    when they are actually relevant. It is the one addition beyond fixing what the issue names.

```
AFTER
┌──────────────────────────────┐
│ Create note                  │
│                              │
│ FOLDER                       │ ← display face, caps, 0.12em, muted
│ [ 10-topics             ▾ ]  │ ← select; last option "New folder…"
│                              │
│ NOTE NAME                    │
│ [ Weekly review           ]  │ ← --rule-strong, zero radius, hot focus
│                              │
│ 10-topics / Weekly review.md │ ← mono, live
│                              │
│ CONTENT                      │
│ [                         ]  │
│                              │
│            [ Cancel ][Create]│
└──────────────────────────────┘
```

### Design-system additions this requires

Both issues need the system extended, not just the code changed. Three additions:

- **Form fields** (§new): label, input, select, textarea, focus and error states. Generalise
  from `.search-input`, which is currently the only documented field.
- **Dialog / modal** (§new): panel, backdrop, action row. Currently undocumented, which is why
  it drifted.
- **§18 amendment:** menu rows are borderless. The existing mock demonstrates the nesting the
  issue objects to.

---

## Open / not yet designed

13. ~~**Change-awareness badge placement.**~~ **Resolved 2026-07-28** — the sidebar rail
    (decision D17), with no mobile mirror (decision D22).
14. ~~**Global "new note" placement.**~~ **Resolved 2026-07-28** — the sidebar footer
    (decision D18). The create *flow* is designed below (D29, D30).
15. **Folder tree UX/UI review.** Partially addressed — decision D19 settles the folder-vs-note
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
