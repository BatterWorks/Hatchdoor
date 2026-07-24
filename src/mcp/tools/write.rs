//! Mutating MCP tools: note and attachment writes, plus the write-side
//! helpers (index building, git-sync bookkeeping, result shaping). Gated by
//! `HATCHDOOR_MCP_WRITE_ENABLED` at the dispatch layer in `mod.rs`.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::app_state::{AppState, refresh_now};
use crate::vault::VaultIndex;
use crate::vault::{
    AttachmentOutcome, SectionMode, WriteError, WriteOutcome, append_note, archive_note,
    create_note, delete_attachment, delete_note, edit_note, import_attachment_bytes,
    list_note_attachments, move_attachment, move_or_rename_note, rename_attachment,
    replace_section, update_note,
};

use super::super::config::McpConfig;
use super::super::protocol::{JsonRpcFailure, tool_success};
use super::{SlugArgs, non_empty_argument, read_only_tool_annotations, write_tool_annotations};

pub(super) async fn create_note_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: CreateNoteArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid create_note arguments: {error}"))
    })?;
    let relative_path = non_empty_argument("relative_path", args.relative_path)?;
    refuse_marker_write(&relative_path)?;
    refuse_noise_write(&state.scan_config.exclude, &relative_path)?;
    let overwrite = args.overwrite.unwrap_or(false);
    let outcome = create_note(&state.vault_path, &relative_path, &args.content, overwrite)
        .map_err(write_error_to_jsonrpc)?;
    finalize_note_write(&state, "create", outcome, args.commit_summary).await
}

pub(super) async fn update_note_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: UpdateNoteArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid update_note arguments: {error}"))
    })?;
    let index = current_index(&state).await?;
    let entry = note_entry(&index, &args.slug)?;
    let outcome = update_note(&entry, &args.content, &args.expected_content_hash)
        .map_err(write_error_to_jsonrpc)?;
    finalize_note_write(&state, "update", outcome, args.commit_summary).await
}

pub(super) async fn append_to_note_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: AppendNoteArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid append_to_note arguments: {error}"))
    })?;
    let content = non_empty_argument("content", args.content)?;
    let index = current_index(&state).await?;
    let entry = note_entry(&index, &args.slug)?;
    let outcome = append_note(&entry, &content, &args.expected_content_hash)
        .map_err(write_error_to_jsonrpc)?;
    finalize_note_write(&state, "append", outcome, args.commit_summary).await
}

pub(super) async fn edit_note_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: EditNoteArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid edit_note arguments: {error}"))
    })?;
    let index = current_index(&state).await?;
    let entry = note_entry(&index, &args.slug)?;
    let outcome = edit_note(
        &entry,
        &args.old_string,
        &args.new_string,
        &args.expected_content_hash,
        args.replace_all.unwrap_or(false),
    )
    .map_err(write_error_to_jsonrpc)?;
    finalize_note_write(&state, "edit", outcome, args.commit_summary).await
}

pub(super) async fn replace_section_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: ReplaceSectionArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid replace_section arguments: {error}"))
    })?;
    let heading = non_empty_argument("heading", args.heading)?;
    let mode = match args.mode.as_str() {
        "replace" => SectionMode::Replace,
        "before" => SectionMode::Before,
        "after" => SectionMode::After,
        other => {
            return Err(JsonRpcFailure::invalid_params(format!(
                "mode must be one of replace, before, after (got '{other}')"
            )));
        }
    };
    let index = current_index(&state).await?;
    let entry = note_entry(&index, &args.slug)?;
    let outcome = replace_section(
        &entry,
        &heading,
        mode,
        &args.content,
        &args.expected_content_hash,
    )
    .map_err(write_error_to_jsonrpc)?;
    finalize_note_write(&state, "replace_section", outcome, args.commit_summary).await
}

pub(super) async fn rename_note_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: RenameNoteArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid rename_note arguments: {error}"))
    })?;
    let new_title = non_empty_argument("new_title", args.new_title)?;
    if new_title.contains('/') || new_title.contains('\\') {
        return Err(JsonRpcFailure::invalid_params(
            "new_title cannot contain path separators",
        ));
    }
    let index = current_index(&state).await?;
    let entry = note_entry(&index, &args.slug)?;
    let target = replace_filename(&entry.relative_path, &new_title);
    refuse_noise_write(&state.scan_config.exclude, &target)?;
    let outcome = move_or_rename_note(
        &state.vault_path,
        &index,
        &entry,
        &target,
        &args.expected_content_hash,
    )
    .map_err(write_error_to_jsonrpc)?;
    finalize_note_write(&state, "rename", outcome, args.commit_summary).await
}

pub(super) async fn move_note_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: MoveNoteArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid move_note arguments: {error}"))
    })?;
    let index = current_index(&state).await?;
    let entry = note_entry(&index, &args.slug)?;
    let target_folder = args.target_folder.trim().trim_matches('/');
    let file_name = entry
        .relative_path
        .rsplit('/')
        .next()
        .unwrap_or(&entry.relative_path);
    let target = if target_folder.is_empty() {
        file_name.to_string()
    } else {
        format!("{target_folder}/{file_name}")
    };
    refuse_noise_write(&state.scan_config.exclude, &target)?;
    let outcome = move_or_rename_note(
        &state.vault_path,
        &index,
        &entry,
        &target,
        &args.expected_content_hash,
    )
    .map_err(write_error_to_jsonrpc)?;
    finalize_note_write(&state, "move", outcome, args.commit_summary).await
}

pub(super) async fn move_rename_note_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: MoveRenameNoteArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid move_rename_note arguments: {error}"))
    })?;
    let target_relative_path =
        non_empty_argument("target_relative_path", args.target_relative_path)?;
    refuse_noise_write(&state.scan_config.exclude, &target_relative_path)?;
    let index = current_index(&state).await?;
    let entry = note_entry(&index, &args.slug)?;
    let outcome = move_or_rename_note(
        &state.vault_path,
        &index,
        &entry,
        &target_relative_path,
        &args.expected_content_hash,
    )
    .map_err(write_error_to_jsonrpc)?;
    finalize_note_write(&state, "move_rename", outcome, args.commit_summary).await
}

pub(super) async fn archive_note_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: ArchiveNoteArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid archive_note arguments: {error}"))
    })?;
    let index = current_index(&state).await?;
    let entry = note_entry(&index, &args.slug)?;
    let archive_folder = state.archive_prefix.trim().trim_matches('/');
    let file_name = entry
        .relative_path
        .rsplit('/')
        .next()
        .unwrap_or(&entry.relative_path);
    let target = format!("{archive_folder}/{file_name}");
    refuse_noise_write(&state.scan_config.exclude, &target)?;
    let outcome = archive_note(
        &state.vault_path,
        &index,
        &entry,
        &state.archive_prefix,
        &args.expected_content_hash,
    )
    .map_err(write_error_to_jsonrpc)?;
    finalize_note_write(&state, "archive", outcome, args.commit_summary).await
}

pub(super) async fn delete_note_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: DeleteNoteArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid delete_note arguments: {error}"))
    })?;
    let index = current_index(&state).await?;
    let entry = note_entry(&index, &args.slug)?;
    let outcome = delete_note(
        &state.vault_path,
        &index,
        &entry,
        &args.expected_content_hash,
    )
    .map_err(write_error_to_jsonrpc)?;
    finalize_note_write(&state, "delete", outcome, args.commit_summary).await
}

pub(super) async fn import_attachment_tool(
    state: AppState,
    arguments: Value,
    config: &McpConfig,
) -> Result<Value, JsonRpcFailure> {
    use base64::Engine as _;

    let args: ImportAttachmentArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid import_attachment arguments: {error}"))
    })?;
    let target_relative_path =
        non_empty_argument("target_relative_path", args.target_relative_path)?;
    refuse_marker_write(&target_relative_path)?;
    refuse_noise_write(&state.scan_config.exclude, &target_relative_path)?;
    let overwrite = args.overwrite.unwrap_or(false);

    // Whitespace-tolerant so line-wrapped base64 still decodes.
    let content: String = args
        .content
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();

    // Guard the encoded payload before decoding: base64 inflates bytes by ~4/3,
    // so anything longer than that for the cap cannot decode to an allowed size.
    // Rejecting up front avoids decoding a deliberately oversized payload; the
    // authoritative check on the decoded length runs in import_attachment_bytes.
    let max_encoded = config
        .max_base64_bytes
        .saturating_mul(4)
        .div_ceil(3)
        .saturating_add(4);
    if content.len() as u64 > max_encoded {
        return Err(JsonRpcFailure::invalid_params(format!(
            "attachment exceeds max size: base64 content is larger than the {}-byte base64 limit allows",
            config.max_base64_bytes
        )));
    }

    let bytes = base64::engine::general_purpose::STANDARD
        .decode(content.as_bytes())
        .map_err(|error| {
            JsonRpcFailure::invalid_params(format!("content is not valid base64: {error}"))
        })?;

    let outcome = import_attachment_bytes(
        &state.vault_path,
        &target_relative_path,
        &bytes,
        config.max_base64_bytes,
        overwrite,
    )
    .map_err(write_error_to_jsonrpc)?;
    record_attachment_write(&state, "import_attachment", &outcome, args.commit_summary);
    let warning = git_sync_warning(&state).await;
    Ok(attachment_success(outcome, warning))
}

pub(super) async fn move_attachment_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: MoveAttachmentArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid move_attachment arguments: {error}"))
    })?;
    let source_relative_path =
        non_empty_argument("source_relative_path", args.source_relative_path)?;
    let target_relative_path =
        non_empty_argument("target_relative_path", args.target_relative_path)?;
    refuse_marker_write(&source_relative_path)?;
    refuse_marker_write(&target_relative_path)?;
    refuse_noise_write(&state.scan_config.exclude, &target_relative_path)?;
    let index = current_index(&state).await?;
    let outcome = move_attachment(
        &state.vault_path,
        &index,
        &source_relative_path,
        &target_relative_path,
    )
    .map_err(write_error_to_jsonrpc)?;
    refresh_after_write(&state).await?;
    record_attachment_write(&state, "move_attachment", &outcome, args.commit_summary);
    let warning = git_sync_warning(&state).await;
    Ok(attachment_success(outcome, warning))
}

pub(super) async fn rename_attachment_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: RenameAttachmentArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid rename_attachment arguments: {error}"))
    })?;
    let source_relative_path =
        non_empty_argument("source_relative_path", args.source_relative_path)?;
    let new_filename = non_empty_argument("new_filename", args.new_filename)?;
    refuse_marker_write(&source_relative_path)?;
    refuse_marker_write(&new_filename)?;
    let target_relative_path = replace_filename(&source_relative_path, &new_filename);
    refuse_noise_write(&state.scan_config.exclude, &target_relative_path)?;
    let index = current_index(&state).await?;
    let outcome = rename_attachment(
        &state.vault_path,
        &index,
        &source_relative_path,
        &new_filename,
    )
    .map_err(write_error_to_jsonrpc)?;
    refresh_after_write(&state).await?;
    record_attachment_write(&state, "rename_attachment", &outcome, args.commit_summary);
    let warning = git_sync_warning(&state).await;
    Ok(attachment_success(outcome, warning))
}

pub(super) async fn delete_attachment_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: DeleteAttachmentArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid delete_attachment arguments: {error}"))
    })?;
    let source_relative_path =
        non_empty_argument("source_relative_path", args.source_relative_path)?;
    let index = current_index(&state).await?;
    let outcome = delete_attachment(&state.vault_path, &index, &source_relative_path)
        .map_err(write_error_to_jsonrpc)?;
    refresh_after_write(&state).await?;
    record_attachment_write(&state, "delete_attachment", &outcome, args.commit_summary);
    let warning = git_sync_warning(&state).await;
    Ok(attachment_success(outcome, warning))
}

pub(super) async fn list_note_attachments_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: SlugArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid list_note_attachments arguments: {error}"))
    })?;
    let index = current_index(&state).await?;
    let entry = note_entry(&index, &args.slug)?;
    let attachments = list_note_attachments(&state.vault_path, &index.layers, &entry)
        .map_err(write_error_to_jsonrpc)?;
    Ok(tool_success(json!({ "attachments": attachments })))
}

/// Build the vault index off the async runtime. Write tools need the full
/// index to rewrite backlinks/assets, but the O(vault) walk must not block a
/// tokio worker.
async fn current_index(state: &AppState) -> Result<VaultIndex, JsonRpcFailure> {
    let vault_path = state.vault_path.clone();
    let scan_config = state.scan_config.clone();
    match tokio::task::spawn_blocking(move || {
        VaultIndex::build_with_config(&vault_path, &scan_config)
    })
    .await
    {
        Ok(Ok(index)) => Ok(index),
        Ok(Err(error)) => Err(JsonRpcFailure::internal(format!(
            "failed to index vault at '{}': {error}",
            state.vault_path.display()
        ))),
        Err(join_error) => Err(JsonRpcFailure::internal(format!(
            "vault index build panicked: {join_error}"
        ))),
    }
}

fn note_entry(index: &VaultIndex, slug: &str) -> Result<crate::vault::NoteEntry, JsonRpcFailure> {
    let slug = slug.trim();
    if slug.is_empty() {
        return Err(JsonRpcFailure::invalid_params("slug cannot be empty"));
    }
    index
        .find_by_slug(slug)
        .cloned()
        .ok_or_else(|| JsonRpcFailure::not_found(format!("Note not found: {slug}")))
}

async fn refresh_after_write(state: &AppState) -> Result<(), JsonRpcFailure> {
    refresh_now(state)
        .await
        .map_err(|(_status, body)| JsonRpcFailure::internal(body.0.error))
}

async fn finalize_note_write(
    state: &AppState,
    op: &str,
    mut outcome: WriteOutcome,
    commit_summary: Option<String>,
) -> Result<Value, JsonRpcFailure> {
    refresh_after_write(state).await?;
    if outcome.slug.is_none() && outcome.relative_path.is_some() && outcome.content_hash.is_some() {
        let index = current_index(state).await?;
        let relative_path = outcome
            .relative_path
            .as_deref()
            .expect("relative_path checked above");
        outcome.slug = slug_for_relative_path(&index, relative_path);
        if outcome.slug.is_none() {
            return Err(JsonRpcFailure::internal(
                "note write completed but refreshed index did not contain the note",
            ));
        }
    }
    record_note_write(state, op, &outcome, commit_summary);
    let warning = git_sync_warning(state).await;
    // Report the note's resulting layer (None = default surface) so a caller
    // sees which surface a create/move/rename/archive landed on. Read from the
    // just-refreshed cache; a delete leaves no note, so the layer is None.
    let layer = match &outcome.slug {
        Some(slug) => state
            .cache
            .read()
            .await
            .sqlite
            .read_note_by_slug(slug)
            .ok()
            .flatten()
            .and_then(|note| note.layer),
        None => None,
    };
    Ok(write_success(outcome, warning, layer))
}

fn slug_for_relative_path(index: &VaultIndex, relative_path: &str) -> Option<String> {
    index
        .ordered_entries()
        .into_iter()
        .find(|entry| entry.relative_path == relative_path)
        .map(|entry| entry.slug)
}

/// Returns the last sync error message when the most recent sync failed.
async fn git_sync_warning(state: &AppState) -> Option<String> {
    let handle = state.git_sync.get()?;
    let guard = handle.status();
    let snapshot = guard.read().await;
    if snapshot.last_ok {
        None
    } else {
        snapshot
            .last_error
            .clone()
            .map(|e| format!("git sync has not succeeded since: {e}"))
    }
}

/// Hard-refuse any write whose target basename is the layer marker file. A
/// marker demotes its whole folder, so letting a write tool create or rename
/// one would let an agent silently reclassify a subtree; markers are edited in
/// the vault directly, never through the API.
fn refuse_marker_write(path: &str) -> Result<(), JsonRpcFailure> {
    // Take the last non-empty path segment so trailing separators or a bare `.`
    // component can't hide the marker basename, and compare case-insensitively
    // so a case-folding filesystem can't smuggle one in either.
    let basename = path
        .split(['/', '\\'])
        .rfind(|segment| !segment.is_empty() && *segment != ".")
        .unwrap_or(path);
    if basename.eq_ignore_ascii_case(crate::vault::MARKER_FILE_NAME) {
        return Err(JsonRpcFailure::invalid_params(format!(
            "'{}' is a reserved Hatchdoor layer marker and cannot be written through the API; \
             edit it directly in the vault.",
            crate::vault::MARKER_FILE_NAME
        )));
    }
    Ok(())
}

/// Hard-refuse a write whose target path matches a noise-exclusion pattern. The
/// index applies the same matcher, so such a note or attachment would be written
/// to disk yet silently absent from every read surface — an invisible write. The
/// `.hatchdoor-layer` marker is exempt from exclusion, so this never fires on a
/// marker (which `refuse_marker_write` handles separately).
fn refuse_noise_write(
    exclude: &crate::vault::ExcludeMatcher,
    path: &str,
) -> Result<(), JsonRpcFailure> {
    if exclude.is_excluded(std::path::Path::new(path.trim()), false) {
        return Err(JsonRpcFailure::invalid_params(format!(
            "'{path}' matches a Hatchdoor noise-exclusion pattern and would be ignored by the \
             index; choose a path outside the excluded set."
        )));
    }
    Ok(())
}

fn write_error_to_jsonrpc(error: WriteError) -> JsonRpcFailure {
    match error {
        WriteError::InvalidInput(message) => JsonRpcFailure::invalid_params(message),
        WriteError::Conflict(message) => JsonRpcFailure::invalid_params(message),
        WriteError::Io(message) => JsonRpcFailure::internal(message),
    }
}

fn write_success(
    outcome: WriteOutcome,
    git_sync_warning: Option<String>,
    layer: Option<String>,
) -> Value {
    tool_success(json!({
        "ok": true,
        "slug": outcome.slug,
        "relative_path": outcome.relative_path,
        "content_hash": outcome.content_hash,
        "layer": layer,
        "quality_warnings": outcome.quality_warnings,
        "rewritten_notes": outcome.rewritten_notes,
        "moved_assets": outcome.moved_assets,
        "trashed_path": outcome.trashed_path,
        "git_sync_warning": git_sync_warning,
    }))
}

fn attachment_success(outcome: AttachmentOutcome, git_sync_warning: Option<String>) -> Value {
    tool_success(json!({
        "ok": true,
        "attachment": outcome.attachment,
        "rewritten_notes": outcome.rewritten_notes,
        "trashed_path": outcome.trashed_path,
        "cleanup_warning": outcome.cleanup_warning,
        "git_sync_warning": git_sync_warning,
    }))
}

/// Build a WriteRecord from a note outcome and enqueue it for git sync (no-op when disabled).
fn record_note_write(
    state: &AppState,
    op: &str,
    outcome: &WriteOutcome,
    commit_summary: Option<String>,
) {
    let target = outcome
        .relative_path
        .clone()
        .or_else(|| outcome.slug.clone())
        .unwrap_or_else(|| "note".to_string());
    state.record_vault_write(crate::git::WriteRecord {
        op: op.to_string(),
        target,
        affected_paths: outcome.affected_paths.clone(),
        summary: commit_summary,
    });
}

/// Build a WriteRecord from an attachment outcome and enqueue it for git sync (no-op when disabled).
fn record_attachment_write(
    state: &AppState,
    op: &str,
    outcome: &AttachmentOutcome,
    commit_summary: Option<String>,
) {
    state.record_vault_write(crate::git::WriteRecord {
        op: op.to_string(),
        target: outcome.attachment.relative_path.clone(),
        affected_paths: outcome.affected_paths.clone(),
        summary: commit_summary,
    });
}

fn replace_filename(relative_path: &str, new_title: &str) -> String {
    let directory = relative_path.rsplit_once('/').map(|(dir, _)| dir);
    match directory {
        Some(dir) if !dir.is_empty() => format!("{dir}/{new_title}.md"),
        _ => format!("{new_title}.md"),
    }
}

pub(super) fn write_tools_list() -> Vec<Value> {
    vec![
        json!({
            "name": "create_note",
            "description": "Create a Markdown note at a vault-relative path. Parent folders are created automatically. Fails if the note exists unless overwrite is true.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "relative_path": {"type": "string", "minLength": 1},
                    "content": {"type": "string"},
                    "overwrite": {"type": "boolean", "default": false},
                    "commit_summary": {"type": "string", "description": "Optional one-line summary of this change for the git commit body."}
                },
                "required": ["relative_path", "content"],
                "additionalProperties": false
            },
            "annotations": write_tool_annotations(true, false)
        }),
        json!({
            "name": "update_note",
            "description": "Replace the full Markdown content of an existing note. Requires expected_content_hash from get_note.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "slug": {"type": "string", "minLength": 1},
                    "content": {"type": "string"},
                    "expected_content_hash": {"type": "string", "minLength": 1},
                    "commit_summary": {"type": "string", "description": "Optional one-line summary of this change for the git commit body."}
                },
                "required": ["slug", "content", "expected_content_hash"],
                "additionalProperties": false
            },
            "annotations": write_tool_annotations(true, false)
        }),
        json!({
            "name": "append_to_note",
            "description": "Append Markdown content to an existing note. Requires expected_content_hash from get_note.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "slug": {"type": "string", "minLength": 1},
                    "content": {"type": "string", "minLength": 1},
                    "expected_content_hash": {"type": "string", "minLength": 1},
                    "commit_summary": {"type": "string", "description": "Optional one-line summary of this change for the git commit body."}
                },
                "required": ["slug", "content", "expected_content_hash"],
                "additionalProperties": false
            },
            "annotations": write_tool_annotations(false, false)
        }),
        json!({
            "name": "edit_note",
            "description": "Make a surgical string replacement in an existing note. old_string must match exactly and be unique unless replace_all is true; otherwise the edit is rejected without writing. Prefer this over update_note for small changes. Requires expected_content_hash from get_note.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "slug": {"type": "string", "minLength": 1},
                    "old_string": {"type": "string", "minLength": 1},
                    "new_string": {"type": "string"},
                    "expected_content_hash": {"type": "string", "minLength": 1},
                    "replace_all": {"type": "boolean"},
                    "commit_summary": {"type": "string", "description": "Optional one-line summary of this change for the git commit body."}
                },
                "required": ["slug", "old_string", "new_string", "expected_content_hash"],
                "additionalProperties": false
            },
            "annotations": write_tool_annotations(false, false)
        }),
        json!({
            "name": "replace_section",
            "description": "Replace or insert around a whole Markdown section identified by its heading (e.g. '## Multi-engine support'). The section spans the heading line through the body up to the next same-or-higher heading. mode 'replace' overwrites the section (content should include the heading), 'before' inserts content above the heading, 'after' inserts content below the section. Headings inside fenced code blocks are ignored; the heading must match exactly and be unique. Requires expected_content_hash from get_note.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "slug": {"type": "string", "minLength": 1},
                    "heading": {"type": "string", "minLength": 1},
                    "mode": {"type": "string", "enum": ["replace", "before", "after"]},
                    "content": {"type": "string"},
                    "expected_content_hash": {"type": "string", "minLength": 1},
                    "commit_summary": {"type": "string", "description": "Optional one-line summary of this change for the git commit body."}
                },
                "required": ["slug", "heading", "mode", "content", "expected_content_hash"],
                "additionalProperties": false
            },
            "annotations": write_tool_annotations(false, false)
        }),
        json!({
            "name": "rename_note",
            "description": "Rename a note within its current folder, rewrite wikilink backlinks, move referenced assets with the note, and rewrite other asset references. Requires expected_content_hash from get_note.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "slug": {"type": "string", "minLength": 1},
                    "new_title": {"type": "string", "minLength": 1},
                    "expected_content_hash": {"type": "string", "minLength": 1},
                    "commit_summary": {"type": "string", "description": "Optional one-line summary of this change for the git commit body."}
                },
                "required": ["slug", "new_title", "expected_content_hash"],
                "additionalProperties": false
            },
            "annotations": write_tool_annotations(true, false)
        }),
        json!({
            "name": "move_note",
            "description": "Move a note to a target vault-relative folder, rewrite wikilink backlinks, move referenced assets with the note, and rewrite other asset references. Requires expected_content_hash from get_note.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "slug": {"type": "string", "minLength": 1},
                    "target_folder": {"type": "string"},
                    "expected_content_hash": {"type": "string", "minLength": 1},
                    "commit_summary": {"type": "string", "description": "Optional one-line summary of this change for the git commit body."}
                },
                "required": ["slug", "target_folder", "expected_content_hash"],
                "additionalProperties": false
            },
            "annotations": write_tool_annotations(true, false)
        }),
        json!({
            "name": "move_rename_note",
            "description": "Move and rename a note to a target vault-relative Markdown path in one operation, rewrite wikilink backlinks, move referenced assets with the note, and rewrite other asset references. Requires expected_content_hash from get_note.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "slug": {"type": "string", "minLength": 1},
                    "target_relative_path": {"type": "string", "minLength": 1},
                    "expected_content_hash": {"type": "string", "minLength": 1},
                    "commit_summary": {"type": "string", "description": "Optional one-line summary of this change for the git commit body."}
                },
                "required": ["slug", "target_relative_path", "expected_content_hash"],
                "additionalProperties": false
            },
            "annotations": write_tool_annotations(true, false)
        }),
        json!({
            "name": "archive_note",
            "description": "Archive a note by moving it to Hatchdoor's configured archive folder, rewrite wikilink backlinks, move referenced assets with the note, and rewrite other asset references. Requires expected_content_hash from get_note.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "slug": {"type": "string", "minLength": 1},
                    "expected_content_hash": {"type": "string", "minLength": 1},
                    "commit_summary": {"type": "string", "description": "Optional one-line summary of this change for the git commit body."}
                },
                "required": ["slug", "expected_content_hash"],
                "additionalProperties": false
            },
            "annotations": write_tool_annotations(true, false)
        }),
        json!({
            "name": "delete_note",
            "description": "Trash a note by moving it to .hatchdoor-trash, remove wikilink backlinks to the deleted note, move referenced assets with it, and rewrite other asset references. Requires expected_content_hash from get_note.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "slug": {"type": "string", "minLength": 1},
                    "expected_content_hash": {"type": "string", "minLength": 1},
                    "commit_summary": {"type": "string", "description": "Optional one-line summary of this change for the git commit body."}
                },
                "required": ["slug", "expected_content_hash"],
                "additionalProperties": false
            },
            "annotations": write_tool_annotations(true, false)
        }),
        json!({
            "name": "import_attachment",
            "description": "Upload an attachment into the vault by sending its bytes base64-encoded. This is the fallback for clients that cannot make an out-of-band HTTP request; it is size-limited (see get_attachment_import_config for the limit). Prefer the HTTP upload endpoint (POST /api/attachment) by default. Returns compact metadata for the imported file.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "content": {"type": "string", "minLength": 1, "description": "Base64-encoded file bytes."},
                    "target_relative_path": {"type": "string", "minLength": 1, "description": "Vault-relative destination path, e.g. Assets/diagram.png."},
                    "overwrite": {"type": "boolean", "default": false},
                    "commit_summary": {"type": "string", "description": "Optional one-line summary of this change for the git commit body."}
                },
                "required": ["content", "target_relative_path"],
                "additionalProperties": false
            },
            "annotations": write_tool_annotations(true, false)
        }),
        json!({
            "name": "move_attachment",
            "description": "Move an existing attachment to a new vault-relative path and rewrite all note references to it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source_relative_path": {"type": "string", "minLength": 1},
                    "target_relative_path": {"type": "string", "minLength": 1},
                    "commit_summary": {"type": "string", "description": "Optional one-line summary of this change for the git commit body."}
                },
                "required": ["source_relative_path", "target_relative_path"],
                "additionalProperties": false
            },
            "annotations": write_tool_annotations(true, false)
        }),
        json!({
            "name": "rename_attachment",
            "description": "Rename an existing attachment in its current folder and rewrite all note references to it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source_relative_path": {"type": "string", "minLength": 1},
                    "new_filename": {"type": "string", "minLength": 1},
                    "commit_summary": {"type": "string", "description": "Optional one-line summary of this change for the git commit body."}
                },
                "required": ["source_relative_path", "new_filename"],
                "additionalProperties": false
            },
            "annotations": write_tool_annotations(true, false)
        }),
        json!({
            "name": "delete_attachment",
            "description": "Trash an existing attachment under .hatchdoor-trash and rewrite all note references to the trashed path.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source_relative_path": {"type": "string", "minLength": 1},
                    "commit_summary": {"type": "string", "description": "Optional one-line summary of this change for the git commit body."}
                },
                "required": ["source_relative_path"],
                "additionalProperties": false
            },
            "annotations": write_tool_annotations(true, false)
        }),
        json!({
            "name": "list_note_attachments",
            "description": "List existing attachments referenced by a note without returning full note content.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "slug": {"type": "string", "minLength": 1}
                },
                "required": ["slug"],
                "additionalProperties": false
            },
            "annotations": read_only_tool_annotations()
        }),
    ]
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateNoteArgs {
    relative_path: String,
    content: String,
    #[serde(default)]
    overwrite: Option<bool>,
    #[serde(default)]
    commit_summary: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateNoteArgs {
    slug: String,
    content: String,
    expected_content_hash: String,
    #[serde(default)]
    commit_summary: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AppendNoteArgs {
    slug: String,
    content: String,
    expected_content_hash: String,
    #[serde(default)]
    commit_summary: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EditNoteArgs {
    slug: String,
    old_string: String,
    new_string: String,
    expected_content_hash: String,
    #[serde(default)]
    replace_all: Option<bool>,
    #[serde(default)]
    commit_summary: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplaceSectionArgs {
    slug: String,
    heading: String,
    mode: String,
    content: String,
    expected_content_hash: String,
    #[serde(default)]
    commit_summary: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RenameNoteArgs {
    slug: String,
    new_title: String,
    expected_content_hash: String,
    #[serde(default)]
    commit_summary: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveNoteArgs {
    slug: String,
    target_folder: String,
    expected_content_hash: String,
    #[serde(default)]
    commit_summary: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveRenameNoteArgs {
    slug: String,
    target_relative_path: String,
    expected_content_hash: String,
    #[serde(default)]
    commit_summary: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArchiveNoteArgs {
    slug: String,
    expected_content_hash: String,
    #[serde(default)]
    commit_summary: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteNoteArgs {
    slug: String,
    expected_content_hash: String,
    #[serde(default)]
    commit_summary: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportAttachmentArgs {
    content: String,
    target_relative_path: String,
    #[serde(default)]
    overwrite: Option<bool>,
    #[serde(default)]
    commit_summary: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveAttachmentArgs {
    source_relative_path: String,
    target_relative_path: String,
    #[serde(default)]
    commit_summary: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RenameAttachmentArgs {
    source_relative_path: String,
    new_filename: String,
    #[serde(default)]
    commit_summary: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteAttachmentArgs {
    source_relative_path: String,
    #[serde(default)]
    commit_summary: Option<String>,
}

#[cfg(test)]
mod record_tests {
    use super::*;

    #[test]
    fn record_note_write_prefers_relative_path_target() {
        let outcome = WriteOutcome {
            slug: Some("new".to_string()),
            relative_path: Some("Projects/New".to_string()),
            content_hash: Some("h".to_string()),
            quality_warnings: Vec::new(),
            rewritten_notes: 0,
            moved_assets: 0,
            trashed_path: None,
            affected_paths: vec![std::path::PathBuf::from("/v/Projects/New.md")],
        };
        let record = crate::git::WriteRecord {
            op: "create".to_string(),
            target: outcome
                .relative_path
                .clone()
                .or_else(|| outcome.slug.clone())
                .unwrap_or_default(),
            affected_paths: outcome.affected_paths.clone(),
            summary: Some("added".to_string()),
        };
        assert_eq!(record.target, "Projects/New");
        assert_eq!(record.affected_paths.len(), 1);
    }
}
