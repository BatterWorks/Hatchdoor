# Inline note editing (Notion-style live preview) — Design

**Date:** 2026-07-28
**Status:** Approved, pending implementation plan
**Issues covered:** [#14 — Live editing content](https://github.com/BattermanZ/Hatchdoor/issues/14),
partially [#7 — Improve attachment UX](https://github.com/BattermanZ/Hatchdoor/issues/7)
(the PDF drop stage)
**Roadmap horizon:** v2.5.0 ("Polished, publishable UI/UX")

> **Numbering convention.** `D1`–`D24` refer to *decisions in this document*. A bare `#7`,
> `#14` always means a **GitHub issue**.

---

## Context / why this exists

Editing a note today is a mode switch. You click **Edit** in the note heading
(`NotePage.tsx:555`), the rendered `<ReactMarkdown>` body is replaced wholesale by
`<NoteEditor>` (`NotePage.tsx:587`), and you are dropped into a single full-note `<textarea>`
of raw markdown with a Write/Preview tab pair. When you are done you press Save, or Cancel and
confirm a discard.

Three things about that are unergonomic, and they were confirmed as the actual complaints:

1. **The mode switch.** Reading and writing are different screens. You cannot fix a typo
   without leaving the thing you were reading.
2. **Raw markdown while writing.** In the Write tab everything is syntax. `**bold**` is not
   bold, `# Heading` is not a heading, and the callout you carefully formatted is four lines
   of `>` prefixes. Preview is a separate tab, so you never see both.
3. **Save friction.** An explicit Save, an explicit Cancel, and a `window.confirm` on discard
   (`NotePage.tsx:377`).

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
- Heading anchors keyed by **source line** (`renderers.tsx:183`)

That last point matters more than it looks. The renderer already receives `node.position` and
already maps rendered nodes back to source lines. The mapping this design needs is not new
machinery; it is a generalisation of something already load-bearing.

### The hard constraint

`AGENTS.md`: **"Keep Markdown authoritative and SQLite disposable."** Notes are plain files in
an Obsidian vault. They are also written by MCP agents, edited directly in Obsidian, and
git-synced. Anything that reformats a file on save produces spurious diffs on every note you
touch and fights the other writers.

---

## Decisions

### D1. Live Preview, not WYSIWYG

Markdown syntax is **hidden where the cursor is not, and shown where it is**. The file on disk
stays byte-identical to what was typed. There is no document model and no markdown serializer,
so `*emphasis*` never comes back as `_emphasis_`, list indents never shift, and block IDs,
callouts, mermaid fences, and `[[wikilinks]]` are never rewritten.

Rejected: true WYSIWYG (TipTap/BlockNote/Milkdown). It re-serializes the whole document on
every save. For a vault with agent and Obsidian co-writers under git sync, that is a
correctness problem, not a cosmetic one.

### D2. Hybrid block editing, not CodeMirror

The rendered tree stays. Exactly one block at a time is swapped for a `<textarea>` holding
that block's own source lines.

Rejected: CodeMirror 6 with decorations, which is what Obsidian actually uses. It is the
higher-ceiling option and ships proper undo, selection, and IME handling for free. It was
rejected for **this** codebase because CodeMirror renders through Lezer, not through React
components. Callouts, mermaid, PDF preview, and wikilink resolution would all have to be
rebuilt as CM widgets *before live editing even matched what the Preview tab already shows
today*. That is most of the work, and it is work already done once.

The accepted cost of D2 is that undo, caret continuity, and cross-block selection become ours
to build (D12, D9, D24).

### D3. No editing mode

Editing is a decoration on the rendered tree, not a state the page enters. `isEditing`
disappears as a concept. `note.content`, the full markdown string including frontmatter,
remains the single source of truth in `NotePage`.

### D4. Fine-grained blocks

The editable unit is a paragraph, a single heading, a **single list item**, a single table
row, one fenced code block, or one callout. Not a whole list or whole table.

Rejected: top-level blocks only. Simpler (no split/merge inside lists) but clicking one bullet
would drop an entire long list into raw markdown, which is exactly the pain being fixed.

---

## Architecture

### D5. Source mapping is the load-bearing piece

The markdown handed to the renderer is a transform of the file:

```
file → parseFrontmatter → body → stripBlockIds → resolveWikilinks → rendered markdown
```

A rendered node's `position` gives line numbers in the **transformed** text. To slice the
right lines out of the **file**, those line numbers must survive the transforms, offset only
by the frontmatter block:

```
fileLine = renderedLine + frontmatterLineOffset(content)
```

New module `lib/sourceMap.ts` exposes `frontmatterLineOffset()`, `sliceLines()`, and
`replaceLines()`.

### D6. The line-count invariant gets an explicit test

`stripBlockIds` (removes trailing `^block-id`) and wikilink resolution (`[[Note]]` becomes
`[Note](/note/slug)`) both change line *contents* but must not change line *counts*. The
existing heading-anchor code already depends on this silently. This design makes the
dependency explicit with a dedicated test in `sourceMap.test.ts`.

If that invariant ever breaks, every block edit writes to the wrong lines. It is the single
highest-consequence assumption in the design and is therefore guarded rather than assumed.

The offset itself has a second trap: `frontmatterLineOffset` must count exactly the lines
`parseFrontmatter` removes from the front of the file, including any blank line it consumes
after the closing `---`. Off by one here misaddresses every block in every note that has
frontmatter, which is most of them. Tested with frontmatter present, absent, and followed by
zero, one, and two blank lines.

### D7. Block wrapping via a HOC

Block-level entries in the components map returned by `createNoteMarkdownComponents` are
wrapped by `withEditableBlock(Component)`, which renders `<EditableBlock>` around the original
output. This keeps `renderers.tsx` structurally as it is rather than rewriting every renderer.

`<EditableBlock>` knows its `[startLine, endLine]` file range, and renders `<BlockInput>` in
place of its children when it is the active block.

### D8. Nested list items use a truncated range

A list item containing a sublist must not swallow its children. `li` uses the range from its
start line to the start of its first child list, not its full `position`. The same applies to
a callout, where the `>` prefix is part of every line and must be preserved across split and
merge.

These are the fiddly cases and are expected to absorb real implementation time.

### D9. Caret placement on click

Clicking rendered text places the caret at the corresponding source offset, via
`caretPositionFromPoint` (with the `caretRangeFromPoint` fallback for WebKit) to get the
offset in the *rendered* text, then walking the source line past syntax tokens to find the Nth
content character.

This mapping is approximate by nature: markdown syntax characters do not exist in the rendered
text. Landing a few characters off is acceptable. Always landing at offset 0 is not. Where the
click falls past the end of the text, the caret goes to end-of-block.

### D10. Three click exceptions

In the rendered view, a click normally places the caret. Three exceptions:

- **Links** navigate, as they do now.
- **Task-list checkboxes** toggle and write back, without entering the block (D18).
- **Collapsible callout summaries** collapse and expand.

### D11. BlockInput matches the typography it replaces

The textarea is auto-growing and styled to match the rendered block (heading size, code font,
list indent, callout inset), so nothing on the page shifts when it appears or disappears.
Wikilink autocomplete (`note-page/autocomplete.ts`) and image paste/drop carry over from
`NoteEditor` unchanged.

---

## Structural editing

### D12. Structural ops are pure string transforms

`lib/blockOps.ts` holds `splitBlock`, `mergeBlockUp`, `indentListItem`, `outdentListItem`, and
`toggleCheckbox`. Each takes `(content, range, caret)` and returns `(content, caret)`. No DOM.

`splitBlock` is responsible for continuation: splitting a list item produces a new item with
the same marker and indent (and an unchecked `[ ]` if the source item was a task), splitting
inside a callout prefixes the new line with `>`, and splitting a paragraph or heading produces
a bare paragraph. Ordered-list renumbering is **not** performed; markdown renderers ignore the
literal numbers, and rewriting them would touch lines outside the edited range.

This is where the actual correctness risk lives, so isolating it in a pure, exhaustively
testable module is deliberate.

### D13. Key bindings

| Key | Behaviour |
|-----|-----------|
| `Enter` at end of paragraph or heading | Create a new empty block below, focus it |
| `Enter` inside a fenced code block or table cell | Insert a literal newline |
| `Enter` in a list item | Create the next list item, preserving marker and indent |
| `Shift+Enter` | Hard line break within the block |
| `Backspace` at offset 0 | Merge into the previous block, caret at the join |
| `Tab` / `Shift+Tab` in a list item | Indent / outdent |
| `ArrowUp` on first line | Previous block, column preserved |
| `ArrowDown` on last line | Next block, column preserved |
| `Escape` | Commit and exit the block |

### D14. Undo is taken over entirely

`lib/editHistory.ts` keeps entries of `{content, focusedBlock, caretRange}`.

- Continuous typing coalesces into one entry, breaking after a ~500ms pause.
- Forced breaks on any structural op and on moving between blocks.
- `Ctrl/Cmd+Z` and `Ctrl/Cmd+Shift+Z` (plus `Ctrl+Y`) are intercepted at the editor container
  and **always** `preventDefault`.
- Undo restores content, refocuses the correct block, and restores the caret. Autosave then
  persists the undone state like any other edit.

Native textarea undo is deliberately **not** mixed in. A textarea's undo stack dies when it
unmounts, cannot cover structural ops that happen above it on the document string, and cannot
be reliably queried for whether it still has history. Mixing the two produces unpredictable
behaviour; owning it produces undo that spans the whole note.

This is the clearest cost of choosing D2 over CodeMirror, and it is accepted knowingly.

---

## Persistence

### D15. Autosave on block commit plus idle flush

Writes fire:

- when a block is committed (blur, Escape, arrow-out, structural op),
- after a ~2s idle pause while still typing inside one block,
- on navigate away and on `visibilitychange` to hidden.

Each write sends the full content against the last confirmed `content_hash`, exactly as the
current explicit save does. On success the returned hash becomes the new base. Every write is
therefore a coherent whole document, and a long paragraph is never left unsaved.

Rejected: pure debounce (writes half-typed states, more git-sync churn) and keeping an
explicit Save (leaves the save friction in place).

### D16. Self-inflicted revision bumps must be ignored

The vault watcher bumps `vaultRevision` on **our own** write. `NotePage.tsx:198` currently
reads any bump during editing as "changed on disk". Under autosave that would fire constantly.

On a revision bump, refetch the note hash in the background and ignore the bump if it equals
the hash we last wrote. Only genuine divergence flags.

Correspondingly, the rule "never refetch under an open editor" becomes **"never refetch while
the document is dirty or a block is active"**, where dirty means local content differs from the
last server-confirmed content.

### D17. Conflicts get a banner, never a modal

On a 409 from an autosave: stop autosaving, keep the local content and its localStorage draft,
and show a non-blocking banner above the note stating that edits are not being saved, with a
**Review** button that opens the existing `ConflictReviewPanel`.

Interrupting someone mid-sentence with a modal is worse than a persistent banner. The same
treatment covers offline failures, with retry on reconnect. `writeDrafts.ts` and
`conflictDiff.ts` carry over unchanged.

---

## Attachments

### D18. Checkbox toggling writes back directly

A click on a rendered task-list checkbox runs `toggleCheckbox` from `blockOps` against that
line and saves. It never enters the block.

### D19. PDF drop and paste

The backend already accepts any content type and caps attachments at 10 MB
(`DEFAULT_MAX_ATTACHMENT_BYTES`, `src/mcp/config.rs:32`; enforced at
`src/vault/write/attachments.rs:60`, overridable via `HATCHDOOR_MAX_ATTACHMENT_BYTES`). The
render path already exists: `![[Attachments/x.pdf]]` resolves to an `img` node whose src ends
in `.pdf`, and `renderers.tsx:96` renders `<PdfPreview>`.

The gap is entirely in the frontend drop filter. Three changes:

1. `firstImageFile` and `firstClipboardImageFile` (`NoteEditor.tsx:194`) widen from `image/*`
   to `image/*` plus `application/pdf`.
2. `handleUploadAttachment` (`NotePage.tsx:537`) branches on mime type. Images pass through
   `normalizeImageForUpload`; everything else uploads as-is. **PDFs must not be normalised**:
   that function does a canvas re-encode to WebP and would destroy the file.
3. Files over the limit produce a clear size error rather than the current generic upload
   failure.

### D20. The drop target becomes the note body

With no single full-note textarea, drops land on the note body. A drop between blocks inserts
a new block at that point, computed from the drop Y coordinate against block boundaries. A
drop into the actively edited block inserts at the caret, as today. Lives in
`note-page/attachmentDrop.ts`.

---

## Retained surfaces

### D21. Frontmatter properties become editable inline

`note-page/frontmatter.ts` already parses and rebuilds frontmatter entries for the current
editor. `NoteProperties` in `sections.tsx` gains the same editable fields, so changing tags no
longer requires source mode.

### D22. Source mode is kept as an escape hatch

`NoteEditor` survives almost unchanged behind an explicit toggle, for bulk edits, repairing a
broken table, or pasting large markdown. Its existing tests (`NoteEditor.test.tsx`) become
source-mode tests rather than being deleted.

### D23. Read-only vaults are unaffected

When `writeEnabled` is false, no block is clickable, no drop target is registered, and the
page behaves exactly as it does today.

### D24. Known gaps, accepted

- **Cross-block selection cannot be typed over.** Browser selection across rendered blocks
  still works for reading and copying, but replacing a multi-block selection by typing does
  not. Notion solves this with block-level selection; that is out of scope.
- **Caret offset mapping is approximate** (D9).
- **No slash commands, drag handles, or block menus.** Explicitly out of scope: the structural
  editing affordances of Notion were not among the stated pains.

---

## Modules

**New** (each with a colocated test file):

| Path | Responsibility |
|------|----------------|
| `lib/sourceMap.ts` | Rendered line to file line mapping; slice and replace line ranges |
| `lib/blockOps.ts` | Pure structural transforms: split, merge, indent, outdent, toggle |
| `lib/editHistory.ts` | Document-level undo and redo stack with coalescing |
| `hooks/useNoteAutosave.ts` | Autosave scheduling, hash tracking, conflict and offline state |
| `note-page/InlineEditorProvider.tsx` | Active block range, caret, enter/exit, op dispatch |
| `note-page/EditableBlock.tsx` | Per-block wrapper: range, click handling, swap to input |
| `note-page/BlockInput.tsx` | Auto-growing textarea, key handling, autocomplete, paste |
| `note-page/attachmentDrop.ts` | Accept predicate and insertion point from drop coordinates |

**Changed:**

| Path | Change |
|------|--------|
| `note-page/renderers.tsx` | Wrap block components via HOC; checkbox click handler |
| `NotePage.tsx` | Drop `isEditing` gating, mount provider, wire autosave, source toggle |
| `note-page/sections.tsx` | Editable properties in `NoteProperties` |
| `NoteEditor.tsx` | Becomes source mode; widen attachment accept predicate |
| `App.css` | Block input styling matched to rendered typography |

`NotePage.tsx` is already 639 lines. Autosave and conflict logic goes into
`useNoteAutosave`, not into the component, so this stage reduces it rather than growing it.

---

## Testing

- **`blockOps.test.ts`** carries the weight: split and merge across paragraphs, headings,
  nested list items, callouts with `>` prefixes, fenced code blocks, and table rows; indent
  and outdent at every nesting level; checkbox toggle on both `- [ ]` and `- [x]`.
- **`sourceMap.test.ts`** pins the D6 line-count invariant against `stripBlockIds` and
  wikilink resolution, plus frontmatter offset with and without frontmatter present.
- **`editHistory.test.ts`** covers coalescing windows, forced breaks, and redo invalidation.
- **Component tests** (`@testing-library/react`): click into a paragraph, type, blur, assert
  the resulting content string; Enter splits; Backspace merges; checkbox toggles without
  entering the block; PDF drop inserts the expected embed; over-limit file shows a size error.
- **Existing tests** in `NoteEditor.test.tsx` must keep passing as source-mode coverage.

---

## Staging

Four scope items plus a new editing model is too much for one change.

| Stage | Content | Rationale |
|-------|---------|-----------|
| **0** | PDF drop and paste (D19, minus D20's body target) | Independent, small, ships value immediately |
| **1** | Source mapping, click into a block, edit, commit. Explicit save retained | Proves the foundation before anything is built on it |
| **2** | Autosave, conflict banner, undo history (D14–D17) | The persistence model |
| **3** | Structural ops and arrow navigation (D12, D13) | The Notion feel |
| **4** | Checkbox toggle, inline properties, source-mode toggle, body drop target (D18, D20, D21, D22) | Remaining scope |

Stage 1 is the go/no-go gate. If source mapping proves unreliable in practice against real
vault notes, the design should be reconsidered before stages 2 to 4 are built on it.

---

## Work packet

**Owned paths:** `frontend/src/lib/{sourceMap,blockOps,editHistory}.ts`,
`frontend/src/hooks/useNoteAutosave.ts`,
`frontend/src/components/note-page/{InlineEditorProvider,EditableBlock,BlockInput,attachmentDrop}.*`,
and their test files.

**Public contract:** none crossing a module boundary outside the frontend. No HTTP or MCP wire
type changes. `PUT /api/note/:slug` and `POST /api/attachment` are consumed exactly as they
are today, including the `content_hash` optimistic-concurrency guard.

**Coordination paths:** `frontend/src/components/NotePage.tsx`,
`frontend/src/components/note-page/renderers.tsx`,
`frontend/src/components/note-page/sections.tsx`,
`frontend/src/components/NoteEditor.tsx`, `frontend/src/App.css`. Per `AGENTS.md`, `NotePage`
and `renderers` are declared integration points with no default feature owner.

**Consumed dependencies:** `lib/markdown.ts`, `lib/noteHeadings.ts`, `lib/writeDrafts.ts`,
`lib/imageUpload.ts`, `api/writeApi.ts`, `note-page/{frontmatter,autocomplete,wikilinks,conflictDiff}.ts`.
Consumed, not edited, except where a decision above names the edit.

**Forbidden paths and invariants:**

- No backend changes. Markdown stays authoritative; nothing re-serializes a note.
- No new runtime dependencies. The zero-dependency property is the reason D2 was chosen over
  CodeMirror; adding an editor library later would invalidate that trade.
- The `content_hash` optimistic-concurrency guard is never bypassed or weakened.
- Read-only vaults (`writeEnabled === false`) keep today's behaviour exactly (D23).

**Validation:** `npm run typecheck`, `npm run lint`, `npm run test`, `npm run format:check` in
`frontend/`. Backend checks are not required for stages 0 to 4 as specified.

---

## Open risks

1. **The line-count invariant (D6).** Highest consequence. Guarded by test, verified in stage 1
   against real vault notes.
2. **Nested list and callout ranges (D8).** Expected to be the largest single time sink.
3. **Caret offset mapping (D9).** Accepted as approximate; refine after stage 1 if it grates.
4. **Autosave against agent writers (D16).** Hatchdoor notes have concurrent writers by design.
   Stage 2 should be exercised against a live MCP agent editing the same note.
