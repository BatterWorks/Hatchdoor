# Write layer — functional improvements (progress log)

Branch: `development`. Frontend dir: `frontend/`.
Gates: `npm run typecheck`, `npm run lint`, `npx vitest run`.
(Repo-wide `format:check` already fails on untouched files — not a gate.)

## Scope agreed
- Defer attachments (image paste/drag) — needs a NEW backend route
  `POST /api/attachment` (Rust multipart → vault, path-safety like MCP
  `import_attachment`). None exists today. TODO only.
- Build the high-impact six (below).

## Backend facts confirmed
- `normalize_note_relative_path` (src/vault/write/paths.rs) auto-appends `.md`
  and rejects `..`/absolute/empty segments. So create can send `folder/name`
  without `.md`.
- Move `target_folder`: trimmed, slash-trimmed, empty = vault root.
- HTTP routes: note CRUD + rename/move/move-rename, download, links, resolve,
  search, stats, graph, refresh, write-capabilities, read-only
  `/vault-assets/{*path}`. NO attachment upload route.

## Features — status
1. [DONE] Preview toggle. New `components/note-page/NotePreview.tsx` (reuses
   parseFrontmatter/stripBlockIds/wikilinks/renderers). `NoteEditor.tsx` gained
   Write/Preview tabs + `renderPreview` prop. NotePage passes it.
2. [DONE] Folder pickers. `app/folderPaths.ts` (`collectFolderPaths`, +test).
   `NoteActionsDialog` Create split into Folder(datalist)+Note name; Move folder
   gets datalist. Props `folderPaths`/`initialFolder`. App computes `folderPaths`.
3. [DONE] Visible Edit button (NotePage heading) + Cmd/Ctrl+S save (NoteEditor)
   + Cmd/Ctrl+N new note (App global keydown, guarded, PWA-friendly).
4. [DONE] Patch-in-place save (NotePage handleSave): set note from draft +
   outcome hash, `loadNote(false)` reconcile — no skeleton flash.
5. [DONE] New-note-in-folder. Explorer FolderNode "+" button (hover),
   ExplorerPane "New" button (root), threaded writeEnabled +
   onCreateNoteInFolder. App `openCreateDialog(folder)` prefills.
6. [DONE] Draft GC. `pruneNoteDrafts(maxAgeMs)` in writeDrafts.ts (+test),
   called on App mount (7-day TTL).

## Status: ALL SIX DONE + GREEN
- typecheck: clean. lint: clean. vitest: 90 passed (17 files).
- CSS added (App.css): note-page-heading, note-edit-button, note-editor-header,
  note-editor-modes, note-editor-hint, note-editor-preview, folder-new-note.
- Tests updated/added in App.write-mode.test.tsx: create form uses Folder+Note
  name (POST relative_path `Projects/New Note`); traversal via Folder `..`;
  preview toggle renders markdown; new-note-in-folder prefills "Projects";
  + folderPaths.test.ts, writeDrafts prune test.
- Attachments TODO comment left in NoteEditor.tsx.

## NOT committed yet
All changes (this session + prior A–F security work) are uncommitted on
`development`. Ask user before committing.

## Deferred (next pass, per user)
- Attachments (backend + frontend).
- Frontmatter structured property editor.
- Wikilink/tag autocomplete.
- Conflict diff/merge view.

## Prior session (already merged-in-tree, uncommitted)
- Security/correctness fixes A–F on the write layer (hash freeze, stale-draft
  detection, conflict reload, outcome/warning surfacing, path validation, modal
  a11y). See writeApi/writePaths/NotePage/NoteEditor/NoteActionsDialog/App.
