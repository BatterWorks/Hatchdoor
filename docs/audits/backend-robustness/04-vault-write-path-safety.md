# Vault write safety & path traversal

4 confirmed (1 high, 2 medium, 1 low), 0 refuted.

## Confirmed findings

### HIGH: delete_note moves assets to trash BEFORE renaming the note, with no rollback

- **Trigger conditions**
  - fs::rename of the note fails after assets already moved (e.g. note file concurrently gone, cross-device trash path, EIO)
  - process crash between move_assets (l.467) and fs::rename (l.468)
- **Location**: src/vault/write/notes.rs:467-475
- **What happens**: In delete_note the order is: move_assets(&asset_moves) (l.467) → fs::rename(entry.path → trash) (l.468) → apply_rewrites (l.475). The referenced attachments are physically relocated into .hatchdoor-trash BEFORE the note itself is trashed. If the note rename returns Err, the function bails with WriteError::Io but the already-moved assets are NOT moved back. This leaves the original note in place now pointing at attachments that no longer exist at their old location. Note the sibling move_or_rename_note does the safe order (rename note first at l.376, then move_assets at l.383), and move_attachment/delete_attachment install rollback_rewrites on rename failure — delete_note has neither the safe order nor any rollback.
- **Why**: Data-integrity: a partial failure (no crash even required) leaves a live note with dangling image/pdf references while the bytes sit in trash; the caller sees only an error and cannot tell the vault was half-mutated.
- **Fix sketch**: Rename the note into trash FIRST, then move_assets, mirroring move_or_rename_note; on any post-rename failure, roll the note (and any already-moved assets) back, or move assets last and reverse them on error.

### MEDIUM: Multi-file backlink/asset rewrites are applied non-atomically with no rollback across move/rename/delete/archive

- **Trigger conditions**
  - apply_rewrites hits an IO error (disk full / EIO) on the k-th of N notes
  - process crash partway through the rewrite loop
  - the outer op fails after the note was already renamed (rewrites applied after fs::rename in move_or_rename_note l.384 and delete_note l.475)
- **Location**: src/vault/write/rewrites.rs:168-177
- **What happens**: apply_rewrites iterates rewrites and calls atomic_write per file, appending to `written` as it goes. Each individual file write is atomic, but the SET is not: if write #k fails, files 1..k-1 are already committed with updated wikilinks/asset paths while k..N still hold the old references, and the function returns Err with no attempt to restore the earlier files. move_or_rename_note and delete_note additionally sequence this AFTER the note's own fs::rename (notes.rs l.384 / l.475), so a failure or crash there yields a moved/trashed note plus a mix of rewritten and stale backlinks — dangling or duplicated links that the returned error hides. Unlike move_attachment (which calls rollback_rewrites), these note paths have no compensating action.
- **Why**: Crash/IO between the file mutation and its link follow-up (explicitly in scope) leaves the vault link graph internally inconsistent, and the cache refresh is skipped because the caller received an error.
- **Fix sketch**: Make apply_rewrites capture prior contents and reverse committed writes on failure (best-effort transaction), or stage all rewrites to temp files and rename them only after every stage succeeds; order the note rename to happen last so a rewrite failure needs no note rollback.

### MEDIUM: atomic_write does not fsync the temp file or the parent directory

- **Trigger conditions**
  - process/container killed immediately after fs::rename returns but before the OS flushes data/metadata
  - host power loss
- **Location**: src/vault/write/fs_ops.rs:31-46
- **What happens**: atomic_write writes content to `<path>.hatchdoor-tmp` with fs::write and immediately fs::rename()s it over the target, never calling File::sync_all on the temp file nor fsync on the containing directory. rename() only guarantees atomic metadata replacement, not that the freshly-written bytes or the rename itself are durable. On ext4/xfs with a crash in the window before the journal commit, the target can reappear as zero-length or with stale contents (the classic write-then-rename-without-fsync data-loss pattern). Given the stated assumption that the process can be killed at any instant, every note write (create/update/edit/append/replace_section and all backlink rewrites) is exposed.
- **Why**: A note that reports success can be silently truncated to empty or reverted after a crash, i.e. real data loss on the source of truth.
- **Fix sketch**: Open the temp file, write, then sync_all() before rename; after rename, fsync the parent directory. Wrap in a small helper so all callers get durability.

### LOW: asset_move_plan: allow_trash_collision branch is dead code, so deleting a note whose asset name already exists in trash fails

- **Trigger conditions**
  - deleting/trashing a second note that references an attachment whose relative path already exists under .hatchdoor-trash
- **Location**: src/vault/write/assets.rs:47-58
- **What happens**: asset_move_plan first does `if destination_asset.exists() { return Err(Conflict...) }` (l.47-52) unconditionally, and only then `if allow_trash_collision && destination_asset.exists() { return Err(...) }` (l.53-58) — which is unreachable dead code because the earlier check already returned. For delete_note the destination dir is `.hatchdoor-trash/...` and asset names are NOT uniquified (only the note filename is, via unique_trash_relative_path). So trashing a note whose referenced asset (e.g. img.png) collides with a previously-trashed img.png returns WriteError::Conflict and blocks the delete entirely, even though allow_trash_collision=true was clearly intended to tolerate/uniquify that case.
- **Why**: A benign, common operation (deleting notes that share attachment filenames) becomes un-performable, and the dead second branch shows the collision handling was intended but never takes effect.
- **Fix sketch**: Gate the collision error on `!allow_trash_collision`, and for the trash case uniquify the asset destination (like unique_trash_attachment_relative_path) instead of erroring; remove the unreachable duplicate check.

## Refuted (not real / already handled)

No findings were refuted.
