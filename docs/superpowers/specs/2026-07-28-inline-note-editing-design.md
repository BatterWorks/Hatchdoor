# Inline note editing (live preview) — Design

**Date:** 2026-07-28
**Status:** Approved, pending implementation plan.
**Revised 2026-07-28** after an adversarial spec review that found one fatal and nine serious
issues. See "Review corrections" at the end for what changed and why.
**Issues covered:** [#14 — Live editing content](https://github.com/BattermanZ/Hatchdoor/issues/14),
partially [#7 — Improve attachment UX](https://github.com/BattermanZ/Hatchdoor/issues/7)
(the PDF drop stage)
**Roadmap horizon:** v2.5.0 ("Polished, publishable UI/UX")

> **Numbering convention.** `D1`–`D35` refer to *decisions in this document*. A bare `#7`,
> `#14` always means a **GitHub issue**. Decision numbers are stable: the review-driven
> revision edited the content of existing decisions and appended `D25`–`D35` rather than
> renumbering, so the new decisions appear beside the topic they affect rather than in
> numeric order.

---

## Context / why this exists

Editing a note today is a mode switch. You click **Edit** in the note heading
(`NotePage.tsx:555`), the rendered `<ReactMarkdown>` body is replaced wholesale by
`<NoteEditor>` (`NotePage.tsx:587`), and you are dropped into a single full-note `<textarea>`
of raw markdown with a Write/Preview tab pair. When you are done you press Save, or Cancel and
confirm a discard (`NotePage.tsx:378`).

Three things about that are unergonomic, and they were confirmed as the actual complaints:

1. **The mode switch.** Reading and writing are different screens.
2. **Raw markdown while writing.** In the Write tab everything is syntax, and Preview is a
   separate tab, so you never see both.
3. **Save friction.** Explicit Save, explicit Cancel, and a `window.confirm` on discard.

Notion, and Obsidian's Live Preview, solve all three by making the reading view *be* the
writing view.

### What the codebase already gives us

This app has an unusually rich renderer for a markdown reader, and that shapes the whole
design:

- Obsidian-style callouts, including collapsible ones (`RendererComponents.tsx:100`)
- Mermaid diagrams, lazily loaded (`RendererComponents.tsx:157`)
- Inline PDF preview for `.pdf` embeds and links (`renderers.tsx:96`, `PdfPreview.tsx`)
- KaTeX math, GFM tables and task lists
- Wikilinks resolved to real slugs at render time (`wikilinks.ts`)
- Heading anchors looked up by **source line** (`renderers.tsx:194`)

The renderer already receives `node.position` and already maps rendered nodes back to source
lines. The mapping this design needs is a generalisation of that, not new machinery.

### The hard constraint

`AGENTS.md`: **"Keep Markdown authoritative and SQLite disposable."** Notes are plain files in
an Obsidian vault. They are also written by MCP agents, edited directly in Obsidian, and
git-synced. Anything that reformats a file on save produces spurious diffs on every note
touched and fights the other writers.

---

## Decisions

### D1. Live preview at block granularity, with per-line addressing where it is cheap

Markdown syntax is hidden in the rendered view and shown in the **active editable unit**. The
file on disk stays byte-identical to what was typed. There is no document model and no
markdown serializer, so `*emphasis*` never comes back as `_emphasis_`, list indents never
shift, and block IDs, callouts, mermaid fences, and `[[wikilinks]]` are never rewritten.

**The active unit is a whole block, not a single line**, with two exceptions (D25a). This
matters and was measured rather than assumed. Across the 58 notes in `demo-vault/` and
`docs/starter-vault/`, counting editable units as D4 defines them:

| unit | 1 line | >1 line | % multi-line |
|---|---:|---:|---:|
| list item | 374 | 26 | 6% |
| heading | 261 | 0 | 0% |
| paragraph | 133 | 110 | 45% |
| table row | 35 | 0 | 0% |
| callout | 1 | 25 | 96% |
| code block | 0 | 18 | 100% |
| **total** | **804** | **179** | **18%** |

So for 82% of units, block granularity and line granularity are the same thing. Code blocks
are 100% multi-line but revealing the whole fence is the *desired* behaviour, not a
regression. The real gap is hard-wrapped paragraphs and callouts.

**Rejected: true line-level reveal everywhere.** A textarea cannot sit inline inside a
reflowing paragraph. Showing line 3 raw while lines 1, 2, and 4 stay rendered requires
breaking the paragraph into separate flow elements, which would reflow the page every time the
caret changes line. Obsidian avoids this only because CodeMirror keeps everything in one text
buffer where nothing reflows. Matching it means adopting CodeMirror, which D2 rejects.

**Rejected: true WYSIWYG** (TipTap/BlockNote/Milkdown). It re-serializes the whole document on
every save. For a vault with agent and Obsidian co-writers under git sync, that is a
correctness problem, not a cosmetic one.

### D2. Hybrid block editing, not CodeMirror

The rendered tree stays. One editable unit at a time is swapped for a `<textarea>` holding
that unit's own source lines.

Rejected: CodeMirror 6 with decorations, which is what Obsidian actually uses. It is the
higher-ceiling option and ships proper undo, selection, and IME handling for free. It was
rejected for **this** codebase because CodeMirror renders through Lezer, not through React
components. Callouts, mermaid, PDF preview, and wikilink resolution would all have to be
rebuilt as CM widgets *before live editing even matched what the Preview tab already shows
today*. That is most of the work, and it is work already done once.

The accepted costs of D2 are that undo (D14), caret continuity (D9), cross-block selection
(D24), and IME handling (D31) become ours to build.

### D3. No editing mode

Editing is a decoration on the rendered tree, not a state the page enters. `isEditing`
disappears as a concept. `note.content`, the full markdown string including frontmatter,
remains the single source of truth in `NotePage`.

### D4. Fine-grained units

The editable unit is a paragraph, a single heading, a **single list item**, a single table row
(D27), one fenced code block, or one callout line (D25a). Not a whole list or whole table.

Rejected: top-level blocks only. Simpler, but clicking one bullet would drop an entire long
list into raw markdown, which is exactly the pain being fixed.

---

## Source mapping

### D5. Rendered line to file line

The markdown handed to the renderer is a transform of the file:

```
file → parseFrontmatter → body → stripBlockIds → resolveWikilinks → rendered markdown
```

A rendered node's `position` gives line numbers in the **transformed** text. To slice the
right lines out of the **file**:

```
fileLine = renderedLine + frontmatterLineOffset(content)
```

New module `lib/sourceMap.ts` exposes `frontmatterLineOffset()`, `sliceLines()`, and
`replaceLines()`.

### D6. The frontmatter offset must be derived from `parseFrontmatter`, never re-derived

`parseFrontmatter` (`markdown.ts:42`) does `lines.slice(end + 1).join("\n")` at line 67. It
removes exactly `end + 1` lines and **does not** consume a trailing blank line. Frontmatter
followed by zero, one, or two blank lines all yield the same offset.

The trap is the opposite of what it looks like. `parseFrontmatter` returns `body: input`,
meaning offset **0**, in three separate cases:

- `lines.length < 3` or `lines[0].trim() !== "---"` (line 47)
- no closing `---` found (line 59)
- `looksLikeFrontmatterHeader(header)` returns false (line 64)

That last one fires on any note opening with `---` followed by prose rather than `key: value`:

```markdown
---
just prose here
---
# Heading
```

A naive "find the second `---`" offset says 3. The correct offset is **0**. Every block in
that note would be misaddressed by three lines.

Therefore `frontmatterLineOffset` is **not** allowed to pattern-match the boundary itself. It
must be computed from `parseFrontmatter`'s own output, either by exporting a variant that
returns `bodyStartLine` alongside `properties` and `body`, or as
`content.split(/\r?\n/).length - body.split("\n").length`. Anything that re-implements the
boundary will drift from `looksLikeFrontmatterHeader`.

Tested with frontmatter present, absent, unterminated, non-`key: value`, and followed by zero,
one, and two blank lines.

### D25. The line-count invariant must be repaired, then guarded at runtime

`stripBlockIds` and wikilink resolution both change line *contents*. For D5 to hold they must
not change line *counts*.

**`stripBlockIds` is safe.** `markdown.ts:16` is a same-line `.replace(/ \^[a-zA-Z0-9-]+$/gm, "")`.
Verified against trailing IDs, own-line IDs, and ID-shaped text inside fences.

**Wikilink resolution is not safe today, and this is a file-corruption bug.** The pattern at
`wikilinks.ts:23` and `wikilinks.ts:62` is `/(!?)\[\[([^\]]+)\]\]/g`. The class `[^\]]`
excludes `]` but **not `\n`**, so an unclosed `[[` matches across arbitrarily many lines until
it finds `]]` anywhere later in the note, and the replacement collapses them into one line.

This is not a contrived input. A dangling `[[` is exactly the state the wikilink autocomplete
is built around (`getWikilinkTrigger`). Given:

```markdown
TODO link to [[

Another paragraph with [[Real Note]] here.
```

the match runs from the first `[[` to the `]]` of "Real Note", and the rendered body has fewer
lines than the source. Every block below shifts. Under D15 autosave that writes to the wrong
lines and confirms the hash: silent, persisted corruption of the user's file.

Two required changes, both in **stage 1**:

1. **Exclude newlines from the match** in `wikilinks.ts:23`, `wikilinks.ts:62`, and
   `stripNoteWikilinks` in `markdown.ts:24` (same bug, affects `exportContent`). This is a
   behaviour change: a wikilink split across lines stops rendering as a link. Obsidian does not
   support multi-line wikilinks either, so this is treated as a bug fix, and it ships with
   tests covering the dangling-`[[`, newline-in-target, and newline-in-alias cases.
2. **A runtime guard, not only a test.** `sourceMap` asserts
   `renderedBody.split("\n").length === body.split("\n").length` and, if it fails, disables
   inline editing for that note and falls back to source mode (D22) with a visible notice. A
   unit test proves something about the code; it proves nothing about the user's note. The
   guard is what protects the vault.

`wikilinks.ts` and `markdown.ts` therefore move from "consumed" to **owned by stage 1** in the
work packet.

### D26. Not every line belongs to a block, and not every node has a position

The design cannot assume the rendered tree partitions the source. Three verified cases:

- **Display math has no positioned node.** `remark-math` produces `<pre><code class="math-display">`,
  but `rehype-katex` (active at `NotePage.tsx:302`) replaces the wrapper with a position-less
  `<span class="katex-display">`. Inline `$a+b$` is fine, because the containing `p` keeps its
  position.
- **Raw HTML blocks and link reference definitions emit no nodes at all.**
  `mdast-util-to-hast` drops HTML unless `allowDangerousHtml` is set, and `NotePage.tsx:620`
  uses no `rehype-raw`. So `<div>…</div>`, `<details>`, and `[ref]: https://…` are rendered as
  nothing and own no lines.
- **Generated nodes have no position.** The `<section class="footnotes">` wrapper, its
  generated `h2`, its `ol`, and footnote backref `<a>`s all lack `position`. The generated
  `h2` flows through `renderers.tsx`'s `h2` component, which survives today only because
  `renderHeading` falls back to `slugifyHeading(text)` at `renderers.tsx:194`.

Rules:

1. `withEditableBlock` **must tolerate a missing `position`** and render its wrapped component
   unchanged, non-editable, when there is none.
2. Line ranges owned by no block are **unowned**. `mergeBlockUp`, `splitBlock`, and arrow
   navigation must **refuse to cross an unowned range** rather than skipping over it. Merging
   line 25 into line 21 across an intervening `<div>` would silently absorb or delete it.
3. Unowned ranges and position-less blocks are reachable only through source mode (D22). This
   is an accepted limitation and is surfaced in the UI, not hidden.

### D27. Tables get their own rules

Verified: for a row on line 17, `tr` and `td` both report `position` 17–17. mdast distinguishes
cells by column, not by line, so wrapping both yields nested `EditableBlock`s with identical
ranges. The **delimiter row** (`|---|---|`) is represented by no node at all and is unowned.

Therefore:

- The editable unit is the **`tr`**, not the `td`. D4's "single table row" wins; D13's "`Enter`
  inside a table cell" is dropped.
- `Enter` and `Backspace` block ops are **disabled inside tables**. Generic split would insert
  a row before the delimiter and destroy the table; generic merge would join the header row
  across the delimiter into the first body row.
- Restructuring a table (adding rows or columns) goes through source mode.

### D7. Block wrapping via a HOC

Block-level entries in the components map returned by `createNoteMarkdownComponents` are
wrapped by `withEditableBlock(Component)`, which renders `<EditableBlock>` around the original
output. This keeps `renderers.tsx` structurally as it is rather than rewriting every renderer.
Per D26 rule 1, the HOC degrades to a pass-through when `position` is absent.

### D8. Nested and overlapping ranges: innermost wins

Ranges genuinely overlap. Verified on a callout containing a nested quote and a list:
`blockquote` 1–8 contains `blockquote` 3–4 and `ul` 6–8 containing `li` 6–7.

- **Innermost claim wins.** A click resolves to the deepest `EditableBlock` under the pointer;
  outer wrappers do not handle a click their descendant already claimed.
- **A list item does not swallow its sublist.** `li` uses the range from its start line to the
  start of its first child list, not its full `position`.
- **Callout line prefixes are preserved.** Lines inside a callout carry `>` (or `> >` when
  nested). That prefix is part of the file and must survive slicing, display (D25a), split,
  and merge.

### D28. The rendered tree is asynchronously stale, so ranges are frozen while editing

`useResolvedWikilinks` (`wikilinks.ts:12`) is `useState` plus a `useEffect` that **awaits a
`POST /api/resolve-batch`** before calling `setResolved`. So after any content change, the
rendered tree, and therefore every `EditableBlock`'s range, describes the *previous* content
for a full network round-trip.

This is invisible today only because the renderer is unmounted while `NoteEditor` is open
(`NotePage.tsx:587`). Inline editing runs the renderer live during editing, a regime this code
has never been exercised in. Two consequences:

1. **Stale ranges.** Clicking a second block during the resolve window would edit the wrong
   lines.
2. **A `/api/resolve-batch` POST per content mutation.** The effect deps are
   `[markdown, noteRelativePath]`, and D12/D14 both mutate the document string frequently.

Rules:

- The active unit's range is **captured when the unit is entered** and is not re-read from the
  DOM while it is active.
- No new unit may be entered while a resolve is in flight for content newer than the rendered
  tree. The document is marked *settling*; clicks during it are queued to the resolved tree,
  not applied against stale ranges.
- Resolution results are **cached per target** so an edit that adds no new wikilink target
  issues no request. Only genuinely new targets hit the network.

### D29. Line endings are preserved

`parseFrontmatter` splits on `/\r?\n/` and rejoins with `"\n"` (`markdown.ts:46, 67`). Line
counts survive CRLF, so D5 is unaffected. But `sliceLines` and `replaceLines` operate on the
full `content` that gets PUT.

The naive implementation (split `/\r?\n/`, join `"\n"`) would rewrite a CRLF file entirely to
LF on the first block edit: a whole-file spurious diff on every note touched, which is the
exact failure mode D1 cites as the reason WYSIWYG was rejected.

Therefore `sourceMap` **detects the file's dominant line ending on read and reproduces it on
write**, and never normalises. Tested on LF, CRLF, and mixed files.

---

## Interaction model

### D9. Caret placement on click

Clicking rendered text places the caret at the corresponding source offset, via
`caretPositionFromPoint` (with the `caretRangeFromPoint` fallback for WebKit) to get the offset
in the *rendered* text, then walking the source line past syntax tokens to find the Nth content
character.

The mapping is approximate by nature: markdown syntax characters do not exist in the rendered
text. Landing a few characters off is acceptable; always landing at offset 0 is not. A click
past the end of the text goes to end-of-unit.

### D10. Three click exceptions

A click normally places the caret. Three exceptions:

- **Links** navigate, as they do now. This includes footnote backrefs, which are position-less
  and must not attempt to enter a block.
- **Task-list checkboxes** toggle and write back (D18).
- **Collapsible callout summaries** collapse and expand.

### D25a. Callouts and multi-line list items are addressed per line

Callouts are 96% multi-line and are structurally line-prefixed, so every line already stands
alone and revealing one line does not disturb the flow of the others. The same holds for the
6% of list items that span lines.

For these two unit types only, the editable unit is **one source line**, with its `>` or indent
prefix preserved and displayed. Paragraphs, headings, table rows, and code blocks stay
whole-block per D1.

This targets the jarring case (a six-line callout dropping entirely into `>` prefixes) without
the reflow problem that killed line-level paragraphs.

### D11. BlockInput matches the typography it replaces

The textarea is auto-growing and styled to match the rendered unit (heading size, code font,
list indent, callout inset), so nothing shifts when it appears or disappears. Wikilink
autocomplete (`note-page/autocomplete.ts`) and image paste/drop carry over from `NoteEditor`
unchanged.

### D30. Keyboard entry and accessibility

D9 is mouse-only, which is not acceptable for a publishable milestone.

- **Keyboard entry into editing.** A dedicated key (Enter on a focused block, reached by Tab
  navigation) enters the block under focus. There is a keyboard-only path to every editable
  unit.
- **Announcement.** Swapping a rendered `<h2>` for a `<textarea>` is an unannounced live
  change. The `BlockInput` carries an `aria-label` naming the unit type ("editing heading",
  "editing list item") and focus moves to it synchronously on entry, so a screen reader
  announces the transition through the focus change rather than a live region.
- **Structure is not lost.** `EditableBlock` is a wrapper, not a replacement: the rendered
  heading level and list semantics remain in the tree for every non-active unit.
- **Escape always exits** to the rendered view with focus restored to the block, so a keyboard
  user is never trapped.

### D31. IME composition gates every intercepted key

D13 intercepts `Enter` and `Backspace`, and with an IME active `Enter` commits a candidate.
Without gating, Japanese, Chinese, and Korean input would split blocks mid-word.

Every handler in D13 and D14 checks `event.isComposing` (with `compositionstart` /
`compositionend` tracking as the fallback) and does nothing while a composition is active.
D2 explicitly credits CodeMirror with "IME handling for free"; this is the line item that
replaces it, and it is required, not optional.

### D32. Mobile

The app is a PWA and this is a "publishable UI/UX" milestone, so mobile is in scope, not a
follow-up.

- The active `BlockInput` scrolls into view above the virtual keyboard on focus.
- `caretPositionFromPoint` is used on touch identically to mouse; the D9 fallback covers
  engines where it is unreliable.
- The D10 click exceptions get touch-sized targets so a tap on a checkbox or callout summary
  does not accidentally enter the block.

### D33. Search highlighting yields to the active block

`createSearchHighlightPlugin` (`lib/noteSearch.ts:16`) splices `<mark class="search-hit">` into
text nodes, and `NotePage.tsx:318` collects them with a DOM query in a `useLayoutEffect` keyed
on `[markdown, …]`. Entering a block removes that block's marks from the DOM, so
`searchHitsRef` goes stale and `SearchHitNavigator`'s indices shift silently.

The hit collection re-runs when the active unit changes, and `SearchHitNavigator` reports
counts over the currently rendered hits. A hit inside the active unit is simply not navigable
while that unit is being edited.

---

## Structural editing

### D12. Structural ops are pure string transforms

`lib/blockOps.ts` holds `splitBlock`, `mergeBlockUp`, `indentListItem`, `outdentListItem`, and
`toggleCheckbox`. Each takes `(content, range, caret)` and returns `(content, caret)`. No DOM.

`splitBlock` handles continuation: splitting a list item produces a new item with the same
marker and indent (unchecked `[ ]` if the source was a task), splitting inside a callout
prefixes the new line with `>`, and splitting a paragraph or heading produces a bare paragraph.
Ordered-list renumbering is **not** performed; markdown renderers ignore the literal numbers,
and rewriting them would touch lines outside the edited range.

Per D26 rule 2, `splitBlock` and `mergeBlockUp` refuse to cross an unowned line range. Per D27
they are disabled inside tables.

This is where the actual correctness risk lives, so isolating it in a pure, exhaustively
testable module is deliberate.

### D13. Key bindings

All of these are gated on `!event.isComposing` (D31).

| Key | Behaviour |
|-----|-----------|
| `Enter` at end of paragraph or heading | Create a new empty block below, focus it |
| `Enter` inside a fenced code block | Insert a literal newline |
| `Enter` in a list item | Create the next list item, preserving marker and indent |
| `Enter` in a table row | Insert a literal newline is **not** offered; the op is disabled (D27) |
| `Shift+Enter` | Hard line break within the unit |
| `Backspace` at offset 0 | Merge into the previous unit, caret at the join; refuses across unowned ranges and inside tables |
| `Tab` / `Shift+Tab` in a list item | Indent / outdent |
| `ArrowUp` on first line | Previous unit, column preserved; skips nothing, stops at unowned ranges |
| `ArrowDown` on last line | Next unit, column preserved |
| `Escape` | Commit and exit the unit, focus restored to the rendered block |

Navigation order is **source order, not DOM order**. Verified counterexample: with
`remark-gfm`, footnote definitions render inside a generated section at the end of the document
but carry the source positions of wherever they were written, so DOM order and line order
diverge. `ArrowUp`/`ArrowDown` and the D20 drop insertion point both sort by line range.

### D14. Undo is taken over entirely

`lib/editHistory.ts` keeps entries of `{content, focusedUnit, caretRange}`.

- Continuous typing coalesces into one entry, breaking after a ~500ms pause.
- Forced breaks on any structural op and on moving between units.
- `Ctrl/Cmd+Z`, `Ctrl/Cmd+Shift+Z`, and `Ctrl+Y` are intercepted at the editor container and
  **always** `preventDefault`, gated on `!event.isComposing`.
- Undo restores content, refocuses the correct unit, and restores the caret. Autosave then
  persists the undone state like any other edit.

Native textarea undo is deliberately **not** mixed in. A textarea's undo stack dies when it
unmounts, cannot cover structural ops that happen above it on the document string, and cannot
be reliably queried for remaining history. Mixing the two produces unpredictable behaviour;
owning it produces undo that spans the whole note.

This is the clearest cost of choosing D2 over CodeMirror, and it is accepted knowingly.

---

## Persistence

### D15. Autosave on unit commit plus idle flush

Writes fire:

- when a unit is committed (blur, Escape, arrow-out, structural op),
- after a ~2s idle pause while still typing inside one unit,
- on navigate away and on `visibilitychange` to hidden.

Each write sends the full content against the last confirmed `content_hash`, exactly as the
current explicit save does. On success the returned hash becomes the new base. Every write is
therefore a coherent whole document, and a long paragraph is never left unsaved.

Rejected: pure debounce (writes half-typed states, more churn) and keeping an explicit Save
(leaves the save friction in place).

### D16. Self-inflicted revision bumps must be ignored, and the confirmed-hash set is not a scalar

The vault watcher bumps `vaultRevision` on **our own** write. `NotePage.tsx:198` currently reads
any bump during editing as "changed on disk". Under autosave that would fire constantly.

On a revision bump, compare against hashes we wrote and ignore the bump if it matches. Two
refinements the naive version gets wrong:

- **Keep a set of recently confirmed hashes, not just the latest.** Each commit produces two
  bumps (the write path bumps directly, and the watcher fires again after
  `WATCH_DEBOUNCE`, `src/vault_watcher.rs:14`), and bumps arrive asynchronously. A bump
  generated by write *N* can land after write *N+1* is confirmed; comparing only against the
  latest hash reports false divergence.
- **There is no cheap hash endpoint.** `/api/note/:slug` returns the full note body, so a
  "background hash refetch" is a full fetch. Prefer matching against the confirmed-hash set
  without refetching at all, and refetch only when a bump matches nothing known.

The rule "never refetch under an open editor" becomes **"never refetch while the document is
dirty or a unit is active"**, where dirty means local content differs from the last
server-confirmed content.

### D17. Conflicts get a banner, never a modal

On a 409 from an autosave: stop autosaving, keep the local content and its localStorage draft,
and show a non-blocking banner above the note stating that edits are not being saved, with a
**Review** button that opens the existing conflict review panel.

Interrupting someone mid-sentence with a modal is worse than a persistent banner. The same
treatment covers offline failures, with retry on reconnect. `writeDrafts.ts` and
`conflictDiff.ts` carry over unchanged.

Note that `ConflictReviewPanel` is currently a module-private function in `NoteEditor.tsx:454`,
rendered inline at `:326`. Reusing it from a banner outside `NoteEditor` requires extracting it
to its own module; that extraction is part of stage 2.

### D34. Reindex cost must be measured before stage 2 is committed

Each write bumps the revision directly and the watcher fires again after `WATCH_DEBOUNCE`
(`src/vault_watcher.rs:14`), each going through `refresh_now` → `run_reindex` →
`VaultIndex::build_with_config` plus `sqlite.replace_from_index_with_embedder` and
`broadcast_vault_revision` (`src/app_state.rs:190-238`).

Today that cost is paid once per editing *session*. Under D15 it is paid roughly twice per unit
commit and per 2s idle flush. Git sync is separately debounced (`src/git/task.rs:75`) so that
part is fine; the reindex is not.

Stage 2 does not begin until this is measured against a realistic vault. If it is prohibitive,
the options are coalescing self-inflicted reindexes, a lighter single-note reindex path, or
lengthening the D15 idle window. This is explicitly a backend concern, and if a backend change
proves necessary it is a scope expansion that must go back to the user first.

---

## Attachments

### D18. Checkbox toggling writes back directly

A click on a rendered task-list checkbox runs `toggleCheckbox` from `blockOps` against that
line and saves, without entering the unit.

Two verified implementation constraints: `mdast-util-to-hast` emits task checkboxes with
`disabled: true`, and disabled inputs fire no click events, so **the handler goes on the `li`**
(or the `disabled` attribute is removed). And the `input` node carries **no position**, so the
line comes from the parent `li`'s range.

### D19. PDF drop and paste

The render path already works end to end for a note at the vault root:
`![[Attachments/x.pdf]]` → `wikilinks.ts:66` emits a markdown image whose src is
`/vault-assets/Attachments/x.pdf?access_token=…` → `renderers.tsx:91` `img` → `isPdfHref`
splits on `[?#]` first so the token query does not defeat it (`renderers.tsx:178`) →
`<PdfPreview>`.

The backend caps attachments at 10 MB (`DEFAULT_MAX_ATTACHMENT_BYTES`, `src/mcp/config.rs:32`,
enforced at `src/vault/write/attachments.rs:60`, overridable via
`HATCHDOOR_MAX_ATTACHMENT_BYTES`), and Axum's body limit is explicitly raised for the route
(`src/server.rs:182`).

The frontend gap and its exact fix:

1. `firstImageFile` and `firstClipboardImageFile` (`NoteEditor.tsx:194`) widen from `image/*`
   to the **backend's extension allowlist**. That list is not "any file type": it is
   `png, jpg, jpeg, gif, webp, avif, bmp, pdf` (`src/vault/write/paths.rs:60`, enforced at
   `:177`). The frontend must mirror it, or a dropped `.docx` produces a raw 400 "unsupported
   attachment extension". This also caps how far #7 can go frontend-only.
2. **No mime branch is needed in `handleUploadAttachment`.** `normalizeImageForUpload` already
   returns non-images untouched, because `shouldConvertImage` gates on
   `file.type.startsWith("image/")` (`lib/imageUpload.ts:86`). PDFs pass through unmodified
   today.
3. Files over the size limit produce a clear error rather than the current generic upload
   failure.
4. **Duplicate filenames 409 rather than uniquifying.** `import_attachment_bytes` is passed
   `false` for its overwrite argument (`src/handlers/write_api.rs:258`) and there is no dedupe;
   `safeAttachmentFilename` (`NotePage.tsx:636`) does not disambiguate. Dropping the same
   `report.pdf` twice returns a conflict. Stage 0 adds a numeric suffix on the client.

### D20. The drop target becomes the note body

With no single full-note textarea, drops land on the note body. A drop between units inserts a
new block at that point, computed from the drop Y coordinate against unit boundaries **sorted
by line range, not DOM order** (D13). A drop into the actively edited unit inserts at the
caret. Lives in `note-page/attachmentDrop.ts`.

### D35. Attachment paths are broken for notes outside the vault root

Pre-existing, but stage 0 inherits it and must not claim to ship value it does not.

`handleUploadAttachment` uploads to the vault-relative `Attachments/<name>` and inserts
`![[Attachments/x.pdf]]` (`NoteEditor.tsx:159`). `resolveAssetHref` then resolves that
**relative to the note's directory** (`normalizeRelativePath(noteDir, pathPart)`,
`wikilinks.ts:115`), so a note at `Projects/Foo.md`
produces `/vault-assets/Projects/Attachments/x.pdf`, which `vault_asset_handler`
(`src/handlers/assets.rs`) resolves directly with no vault-wide fallback, giving a 404.

This already breaks image embeds today. Stage 0 fixes it by inserting a path that resolves
correctly from the note's location, and adds a regression test for a note in a subfolder.
Without this fix, stage 0 works only for notes at the vault root.

---

## Retained surfaces

### D21. Frontmatter properties become editable inline

`note-page/frontmatter.ts` already parses and rebuilds frontmatter entries for the current
editor. `NoteProperties` in `sections.tsx` gains the same editable fields, so changing tags no
longer requires source mode.

### D22. Source mode is kept as an escape hatch

`NoteEditor` survives almost unchanged behind an explicit toggle. It is not merely a
convenience: per D26 and D27 it is the **only** way to edit display math, raw HTML blocks, link
reference definitions, and table structure, and per D25 it is the fallback when the line-count
guard trips. Its existing tests (`NoteEditor.test.tsx`) become source-mode tests rather than
being deleted.

### D23. Read-only vaults are unaffected

When `writeEnabled` is false, no unit is clickable, no drop target is registered, and the page
behaves exactly as it does today.

### D24. Known gaps, accepted

- **Cross-block selection cannot be typed over.** Browser selection across rendered units still
  works for reading and copying, but replacing a multi-unit selection by typing does not.
  Notion solves this with block-level selection; out of scope.
- **Caret offset mapping is approximate** (D9).
- **Hard-wrapped paragraphs reveal their whole source** (D1). Measured at 45% of paragraphs,
  18% of all units. Revisit after stage 1 if it grates in practice.
- **Display math, raw HTML, link reference definitions, and table structure are source-mode
  only** (D26, D27).
- **An active unit prints as an empty textarea.** `exportContent` is derived from
  `note.content` (`NotePage.tsx:280`) and so is unaffected, but browser print of a page with an
  open unit will show a box. Exiting the unit before printing is the workaround; not worth
  code.
- **No slash commands, drag handles, or block menus.** Explicitly out of scope: the structural
  editing affordances of Notion were not among the stated pains.

---

## Modules

**New** (each with a colocated test file):

| Path | Responsibility |
|------|----------------|
| `lib/sourceMap.ts` | Line mapping, range slice and replace, line-ending preservation, runtime invariant guard |
| `lib/blockOps.ts` | Pure structural transforms: split, merge, indent, outdent, toggle |
| `lib/editHistory.ts` | Document-level undo and redo stack with coalescing |
| `hooks/useNoteAutosave.ts` | Autosave scheduling, confirmed-hash set, conflict and offline state |
| `note-page/InlineEditorProvider.tsx` | Active unit range, caret, enter/exit, op dispatch, settling state |
| `note-page/EditableBlock.tsx` | Per-unit wrapper: range, click handling, swap to input |
| `note-page/BlockInput.tsx` | Auto-growing textarea, key handling, IME gating, autocomplete, paste |
| `note-page/attachmentDrop.ts` | Extension allowlist mirror, insertion point from drop coordinates |
| `note-page/ConflictReviewPanel.tsx` | Extracted from `NoteEditor.tsx` so D17's banner can reuse it |

**Changed:**

| Path | Change |
|------|--------|
| `note-page/wikilinks.ts` | **D25:** exclude newlines from the wikilink match; cache resolve results (D28) |
| `lib/markdown.ts` | **D25:** same fix in `stripNoteWikilinks`; **D6:** expose the frontmatter body start line |
| `note-page/renderers.tsx` | Wrap block components via HOC; checkbox handler on `li` |
| `NotePage.tsx` | Drop `isEditing` gating, mount provider, wire autosave, source toggle |
| `note-page/sections.tsx` | Editable properties in `NoteProperties` |
| `NoteEditor.tsx` | Becomes source mode; widen attachment predicate; export the conflict panel |
| `App.css` | Block input styling matched to rendered typography |

`NotePage.tsx` is already 639 lines. Autosave and conflict logic goes into `useNoteAutosave`,
not into the component, so this reduces it rather than growing it.

---

## Testing

- **`blockOps.test.ts`** carries the weight: split and merge across paragraphs, headings,
  nested list items, callouts with `>` and `> >` prefixes, fenced code blocks; refusal to cross
  unowned ranges; refusal inside tables; indent and outdent at every nesting level; checkbox
  toggle on `- [ ]` and `- [x]`.
- **`sourceMap.test.ts`** covers D6 (frontmatter present, absent, unterminated, non-`key: value`,
  and zero to two trailing blank lines), D25 (the dangling-`[[`, newline-in-target, and
  newline-in-alias cases must all preserve line counts after the fix, and the runtime guard must
  trip on a synthetic mismatch), and D29 (LF, CRLF, mixed).
- **`wikilinks.test.ts`** gains the D25 behaviour change: a wikilink split across lines renders
  as literal text, not a link.
- **`editHistory.test.ts`** covers coalescing windows, forced breaks, redo invalidation, and
  the `isComposing` gate.
- **Component tests** (`@testing-library/react`): click into a paragraph, type, blur, assert the
  content string; Enter splits; Backspace merges; Backspace refuses across an HTML block;
  checkbox toggles without entering the unit; a footnote-bearing note navigates in source order;
  PDF drop inserts the expected embed; a duplicate filename gets a suffix; a note in a subfolder
  embeds a resolvable path; an over-limit file shows a size error.
- **Existing tests** in `NoteEditor.test.tsx` must keep passing as source-mode coverage.

---

## Staging

| Stage | Content | Rationale |
|-------|---------|-----------|
| **0** | PDF drop and paste (D19), subfolder path fix (D35) | Independent and shippable, but only once D35 is fixed |
| **1** | Wikilink newline fix and runtime guard (D25), frontmatter offset (D6), source mapping (D5, D29), click into a unit, edit, commit (D7, D8, D9, D26, D27). Explicit save retained | The go/no-go gate |
| **2** | Autosave, conflict banner, undo (D14–D17, D34), panel extraction | Blocked on the D34 measurement |
| **3** | Structural ops, arrow navigation, IME gating (D12, D13, D31) | Not shippable before stage 2: destructive ops without undo |
| **4** | Checkbox toggle, inline properties, source-mode toggle, body drop target, per-line callouts, a11y and mobile passes (D18, D20, D21, D22, D25a, D30, D32, D33) | Remaining scope |

Stage 1 is the go/no-go gate. If source mapping proves unreliable against real vault notes
after D25 and D6 are fixed, the design is reconsidered before stages 2 to 4 are built on it.

Stage 3 is explicitly **not** independently shippable ahead of stage 2: shipping
`splitBlock`/`mergeBlockUp` without D14 undo would be a destructive-operation release with no
recovery path.

---

## Work packet

**Owned paths:** `frontend/src/lib/{sourceMap,blockOps,editHistory}.ts`,
`frontend/src/hooks/useNoteAutosave.ts`,
`frontend/src/components/note-page/{InlineEditorProvider,EditableBlock,BlockInput,attachmentDrop,ConflictReviewPanel}.*`,
plus **`frontend/src/components/note-page/wikilinks.ts` and `frontend/src/lib/markdown.ts`
from stage 1 onward** (D25, D6), and all their test files.

**Public contract:** none crossing a module boundary outside the frontend. No HTTP or MCP wire
type changes. `PUT /api/note/:slug` and `POST /api/attachment` are consumed exactly as today,
including the `content_hash` optimistic-concurrency guard.

**Coordination paths:** `frontend/src/components/NotePage.tsx`,
`frontend/src/components/note-page/renderers.tsx`,
`frontend/src/components/note-page/sections.tsx`, `frontend/src/components/NoteEditor.tsx`,
`frontend/src/App.css`.

`AGENTS.md` names `src/server.rs`, `src/app_state.rs`, and `frontend/src/App.tsx` as declared
integration points. `NotePage.tsx` and `renderers.tsx` are **not** in that list; they are
treated as coordination paths here by the same reasoning, which is an extrapolation and should
be confirmed rather than cited as policy.

**Consumed dependencies:** `lib/noteHeadings.ts`, `lib/writeDrafts.ts`, `lib/imageUpload.ts`,
`lib/noteSearch.ts`, `api/writeApi.ts`,
`note-page/{frontmatter,autocomplete,conflictDiff}.ts`. Consumed, not edited.

**Forbidden paths and invariants:**

- **No backend changes.** If D34's measurement shows a backend change is required, that is a
  scope expansion and goes back to the user before any code is written.
- Markdown stays authoritative; nothing re-serializes a note; line endings are preserved (D29).
- No new runtime dependencies. The zero-dependency property is why D2 was chosen over
  CodeMirror; adding an editor library later would invalidate that trade.
- The `content_hash` optimistic-concurrency guard is never bypassed or weakened.
- Read-only vaults keep today's behaviour exactly (D23).

**Validation:** `npm run typecheck`, `npm run lint`, `npm run test`, `npm run format:check` in
`frontend/`. Backend checks are not required unless D34 forces a backend change.

---

## Open risks

1. **Wikilink resolution latency (D28).** The rendered tree lagging the document by a network
   round-trip is the least-proven part of the design. The per-target cache should make steady
   state free, but this needs to be exercised in stage 1, not assumed.
2. **Nested list and callout ranges (D8, D25a).** Expected to be the largest single time sink.
3. **Reindex cost under autosave (D34).** Gates stage 2 and may force a scope conversation.
4. **Caret offset mapping (D9).** Accepted as approximate; refine after stage 1 if it grates.
5. **Autosave against agent writers (D16).** Hatchdoor notes have concurrent writers by design.
   Stage 2 should be exercised against a live MCP agent editing the same note.

---

## Review corrections

This document was revised after an adversarial review that verified its claims against source.
The substantive corrections, recorded because several were the design being wrong rather than
merely incomplete:

| Was | Now |
|---|---|
| The line-count invariant holds; add a test | **False.** The wikilink regex matches across newlines and collapses lines. Fixed in code plus a runtime guard (D25) |
| `parseFrontmatter` consumes a trailing blank line | It does not. The real trap is three bail-out paths returning offset 0 (D6) |
| Every line belongs to exactly one positioned block | **False.** Display math, raw HTML, link reference definitions, and generated footnote nodes own no lines or carry no position (D26) |
| `tr` and `td` are distinct editable units | They report identical ranges; the delimiter row is unowned. Tables get their own rules (D27) |
| The rendered tree reflects `note.content` | It lags by an awaited `/api/resolve-batch` round-trip (D28) |
| PDFs must be excluded from `normalizeImageForUpload` | Unnecessary; `shouldConvertImage` already gates on `image/*` (D19) |
| The backend accepts any content type | There is an eight-extension allowlist the frontend must mirror (D19) |
| Syntax is revealed per line | Per block, measured as identical for 82% of units, with per-line callouts closing the worst gap (D1, D25a) |
| Accessibility, IME, mobile, search highlighting | Were absent entirely; now D30, D31, D32, D33 |
| Line endings | Were unspecified; naive implementation would rewrite CRLF files (D29) |
