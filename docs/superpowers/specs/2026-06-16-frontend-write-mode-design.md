# Frontend Write Mode Design

Date: 2026-06-16
Status: Approved for implementation planning
Role: Coordinator

## Goal

Add full note management to the Hatchdoor frontend. Users should be able to edit the current note inline, create new notes, rename notes, move notes, and delete notes from the web app.

The first version intentionally excludes attachment management and automatic merge resolution. It should reuse the existing vault-safe write primitives that already power MCP write tools.

## Decisions

- Build full note management, not edit-only mode.
- Use an inline editor on the existing note page.
- Use local draft autosave with explicit Save to the vault.
- Reject stale saves with a conflict state instead of overwriting or auto-merging.
- Open create flow as a blank inline editor.
- Use small dialogs for rename, move, and delete.
- Add dedicated REST write endpoints instead of making the frontend speak MCP JSON-RPC.
- Allow frontend writes whenever the web API is reachable, including unauthenticated deployments.

## Backend API

Add browser-facing write endpoints that call the same vault write functions used by MCP:

- `GET /api/write-capabilities`
  - Returns whether write operations are available and any deployment warnings.
  - Includes a warning when no `HATCHDOOR_WEB_BEARER_TOKEN` is configured.
- `POST /api/note`
  - Creates a note from `relative_path` and initial `content`.
- `PUT /api/note/:slug`
  - Updates note Markdown from `content` and `expected_content_hash`.
- `PATCH /api/note/:slug/rename`
  - Renames a note within its current folder.
- `PATCH /api/note/:slug/move`
  - Moves a note to another folder.
- `PATCH /api/note/:slug/move-rename`
  - Supports full target relative path changes when the frontend needs one request for both.
- `DELETE /api/note/:slug`
  - Deletes a note using `expected_content_hash`.

All write endpoints must hold the existing vault write lock for the full write operation, refresh the index after note writes, emit the existing vault revision event, and record git sync work when automatic git sync is configured.

## Frontend UX

The note page gets a read/edit toggle. Read mode keeps the current behavior. Edit mode replaces the rendered Markdown body with a Markdown editor and actions for Save and Cancel.

Drafts are saved locally while typing:

- Existing-note drafts are keyed by slug.
- New-note drafts use a temporary create-note key until the note is saved.
- Reopening a note with a saved draft should offer to restore that draft.
- Cancelling edit mode should ask before discarding a dirty draft.

Create starts a blank inline editor. On successful save, the app navigates to the new note slug.

Rename, move, and delete are dialogs launched from the existing actions menu. Rename and move navigate to the resulting slug when successful. Delete requires an explicit confirmation and returns the user to a stable non-note route after success.

## Save Flow

On note load, the frontend stores the current note content and `content_hash`. While editing, only draft state changes.

On Save:

1. Send `content` and `expected_content_hash` to the backend.
2. Backend rejects the write if the hash no longer matches.
3. Backend writes the file, refreshes the index, emits a vault revision event, records git sync, and returns updated note metadata plus any git-sync warning.
4. Frontend clears the local draft only after a successful vault write.
5. Frontend returns to read mode and displays any sync warning.

## Error Handling

- `409 Conflict`: the note changed since editing started. Keep the draft, do not overwrite, and show options to reload latest or keep editing the draft.
- `404 Not Found`: the note was deleted or moved. Keep the draft and offer navigation back to the explorer.
- `400 Bad Request`: the title, path, folder, or content request is invalid. Keep the dialog or editor open and show an inline error.
- `500` or refresh failure: keep the draft and allow retry.
- Offline or fetch failure: keep the draft and allow retry.

Delete must include `expected_content_hash` so a stale page cannot delete a changed note without noticing.

## Security Review Scope

Implementation must include a Security Reviewer pass because this feature touches external input, file handling, public web routes, and write permissions.

The security review should explicitly check:

- Path traversal prevention for create, rename, move, and move-rename.
- CSRF risk for write endpoints, especially unauthenticated deployments.
- Public write exposure when Hatchdoor is bound to `0.0.0.0` or exposed through a proxy.
- Content hash enforcement for update and delete.
- Stale delete behavior.
- Whether unauthenticated write warnings are visible enough at startup and in `GET /api/write-capabilities`.

## Tests

Backend tests should cover:

- Create note success.
- Update note success.
- Update conflict rejection.
- Rename success.
- Move success.
- Delete success.
- Delete conflict rejection.
- Invalid path and traversal rejection.
- Refresh and vault revision behavior after successful writes.

Frontend tests should cover:

- Entering and leaving edit mode.
- Local draft persistence and restore.
- Successful save clears the draft and returns to read mode.
- Conflict keeps the draft and shows conflict UI.
- Create flow navigates to the new note.
- Rename, move, and delete dialogs call the expected API paths and handle errors.

The implementation plan should confirm exact selective commands from repo scripts before code changes.
