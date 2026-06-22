# Write layer — heavy items (progress log)

Branch: `development`. Frontend dir: `frontend/`.
Gates: `npm run typecheck`, `npm run lint`, `npx vitest run`.
Doing these ONE AT A TIME, fully tested, updating this log after each.
Not committing unless asked (per standing rule); will offer at checkpoints.

## The list (ordered: frontend-only & contained first, backend last)

1. [DONE] Wikilink/tag autocomplete in the editor
   - `[[` → note-title typeahead; `#` → tag typeahead.
   - Candidate source: flattened note list from the explorer tree (titles +
     slugs) passed App → NotePage → NoteEditor. Tags: best-effort source TBD
     (no /api/tags endpoint; stats has only top_tags). May scope to wikilinks
     first + tags if a cheap source exists.

2. [DONE] Frontmatter structured property editor
   - In edit mode, parse YAML frontmatter into key/value fields editable
     separately from the body; reserialize on change. Must round-trip safely
     and not corrupt unknown YAML.

3. [DONE] Conflict diff/merge view
   - On 409 (or reload), show what changed on disk vs the draft (line diff)
     so the user can reconcile instead of blind overwrite. Builds on the
     existing Reload-latest flow.

4. [DONE] Attachments — image paste/drag upload
   - NEW backend route `POST /api/attachment` (Rust, multipart → vault,
     path-safety mirroring MCP `import_attachment`). Then editor paste/drag
     → upload → insert `![[...]]` / `![](...)`.

## Decisions / notes (filled in as I go)

## Status log (newest at bottom)
- (start) Plan written.
- [item 1 DONE] Wikilink `[[` autocomplete shipped. Gates green (101 tests).
  - New: `app/noteCandidates.ts` (+test) flatten tree → note list.
  - New: `components/note-page/autocomplete.ts` (+test) pure trigger/insert/match.
  - `NoteEditor.tsx`: dropdown (role=listbox/option), keyboard nav
    (↑/↓/Enter/Tab/Esc), click-select, caret restore via pending ref.
    NOTE: did NOT set role="combobox" on textarea (breaks getByRole textbox in
    existing tests); kept aria-expanded/controls/autocomplete only.
  - Threaded `noteCandidates`: App (flatten tree, useMemo) → NotePage → NoteEditor.
  - CSS: `.note-editor-input/-autocomplete/-autocomplete-item`.
  - Test: App.write-mode "inserts a wikilink from autocomplete suggestions"
    (must set selectionStart on the change event — jsdom keeps caret at 0).
  - SCOPED OUT: `#` tag autocomplete — no /api/tags endpoint (stats top_tags is
    incomplete). Would want a new backend tags endpoint; deferred as sub-item 1b.
  - Not committed.
- [items 2-4 DONE] Remaining heavy write-layer items shipped via TDD.
  - Frontmatter editor: new `components/note-page/frontmatter.ts` (+test) for
    splitting, simple scalar/list parsing, complex-YAML refusal, and safe
    serialization. `NoteEditor` now shows editable frontmatter property fields
    separately from the body for simple frontmatter only.
  - Conflict review: new `components/note-page/conflictDiff.ts` (+test) and
    409 flow that fetches latest disk content, renders a disk/draft line diff,
    and offers "Use disk version" / "Keep my draft" resolution actions.
  - Attachments: new Rust `POST /api/attachment` multipart route, backed by
    vault-safe path/extension/max-size validation and the existing protected API
    router. Editor paste/drop uploads images and inserts `![[relative/path]]`.
  - Security pass: upload route shares the existing web auth boundary, refuses
    overwrite, enforces max size, uses vault-relative path normalization, and
    still inherits the existing risk of unauthenticated write deployments if
    Hatchdoor is exposed without `HATCHDOOR_WEB_BEARER_TOKEN`.
  - Gates green:
    - `npm -C frontend run typecheck`
    - `npm -C frontend run lint`
    - `npm -C frontend test` (112 tests)
    - `cargo check`
    - `cargo clippy --all-targets --all-features -- -D warnings`
    - `cargo test` (207 lib + 7 eval + 18 hatchdoor tests)
