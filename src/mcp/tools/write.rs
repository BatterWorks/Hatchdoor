//! Mutating MCP tools: note and attachment writes, plus the write-side
//! helpers (index building, git-sync bookkeeping, result shaping). Gated by
//! `HATCHDOOR_MCP_WRITE_ENABLED` at the dispatch layer in `mod.rs`.

// The deserialization-only scope and retired legacy commit-summary fields are
// intentionally retained until every old client gets a structured invalid
// parameter response instead of silently accepting an ambiguous payload.
#![allow(dead_code)]

use serde::Deserialize;
use serde_json::{Value, json};

use std::str::FromStr;

use crate::app_state::AppState;
use crate::vault::VaultIndex;
use crate::vault::{
    AttachmentOutcome, LayerMap, SectionMode, WriteError, WriteOutcome, append_note, archive_note,
    create_note, delete_attachment, delete_note, edit_note, import_attachment_bytes,
    list_note_attachments, move_attachment, move_or_rename_note, rename_attachment,
    replace_section, update_note,
};
use crate::vault_read::{VaultReadCore, VaultReadError};
use crate::vault_registry::VaultId;
use crate::vault_runtime::VaultControlBlock;

use super::super::config::McpConfig;
use super::super::protocol::{JsonRpcFailure, tool_success};
use super::{non_empty_argument, read_only_tool_annotations, write_tool_annotations};

/// A target resolved from the explicit MCP `vault_id`.  It is intentionally
/// not recoverable from `AppState`'s legacy single-Vault fields.
pub(super) struct McpVault {
    pub(super) vault_id: VaultId,
    control: VaultControlBlock,
}

impl McpVault {
    fn exclude(&self) -> Result<crate::vault::ExcludeMatcher, JsonRpcFailure> {
        crate::vault::ExcludeMatcher::new(self.control.definition().exclude_patterns())
            .map_err(JsonRpcFailure::internal)
    }

    fn definition(&self) -> &crate::vault_registry::VaultDefinition {
        self.control.definition()
    }
}

pub(super) fn scoped_vault(
    state: &AppState,
    arguments: &Value,
) -> Result<McpVault, JsonRpcFailure> {
    let raw = arguments
        .get("vault_id")
        .and_then(Value::as_str)
        .ok_or_else(|| JsonRpcFailure::invalid_params("vault_id is required"))?;
    let vault_id = VaultId::from_str(raw)
        .map_err(|_| JsonRpcFailure::invalid_params("vault_id must be a canonical Vault ID"))?;
    let core = VaultReadCore::new(&state.startup_sqlite, &state.vaults);
    let control = core.control_block(vault_id).map_err(vault_error)?;
    if !control.snapshot().capabilities.mutate {
        return Err(JsonRpcFailure::not_found(
            serde_json::json!({
                "code": "capability_unavailable",
                "message": "This Vault's current source and lifecycle do not allow mutation",
                "vault_id": vault_id,
                "retryable": false,
            })
            .to_string(),
        ));
    }
    Ok(McpVault { vault_id, control })
}

pub(super) async fn acquire_mutation(
    vault: &McpVault,
) -> Result<tokio::sync::OwnedMutexGuard<()>, JsonRpcFailure> {
    vault
        .control
        .acquire_mutation()
        .await
        .map_err(|error| vault_error(crate::vault_read::runtime_error(vault.vault_id, error)))
}

fn vault_error(error: VaultReadError) -> JsonRpcFailure {
    JsonRpcFailure::not_found(serde_json::to_string(&error).unwrap_or(error.message))
}

pub(super) async fn create_note_tool(
    _state: AppState,
    vault: &McpVault,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: CreateNoteArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid create_note arguments: {error}"))
    })?;
    let relative_path = non_empty_argument("relative_path", args.relative_path)?;
    refuse_marker_write(&relative_path)?;
    refuse_noise_write(&vault.exclude()?, &relative_path)?;
    let overwrite = args.overwrite.unwrap_or(false);
    // A metadata-only catalog, not the full index: create_note has no
    // pre-write entry to read a slug from, so it needs this to compute one,
    // but never touches the (expensive, content-reading) wikilink graph.
    let catalog = current_catalog(vault).await?;
    let outcome = create_note(
        &vault_path(vault),
        &relative_path,
        &args.content,
        overwrite,
        &catalog,
    )
    .map_err(|error| write_error_to_jsonrpc(vault.vault_id, error))?;
    Ok(finalize_note_write(
        vault.vault_id,
        &catalog.layers,
        outcome,
    ))
}

pub(super) async fn update_note_tool(
    _state: AppState,
    vault: &McpVault,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: UpdateNoteArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid update_note arguments: {error}"))
    })?;
    let index = current_index(vault).await?;
    let entry = note_entry(&index, &args.slug)?;
    let outcome = update_note(&entry, &args.content, &args.expected_content_hash)
        .map_err(|error| write_error_to_jsonrpc(vault.vault_id, error))?;
    Ok(finalize_note_write(vault.vault_id, &index.layers, outcome))
}

pub(super) async fn append_to_note_tool(
    _state: AppState,
    vault: &McpVault,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: AppendNoteArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid append_to_note arguments: {error}"))
    })?;
    let content = non_empty_argument("content", args.content)?;
    let index = current_index(vault).await?;
    let entry = note_entry(&index, &args.slug)?;
    let outcome = append_note(&entry, &content, &args.expected_content_hash)
        .map_err(|error| write_error_to_jsonrpc(vault.vault_id, error))?;
    Ok(finalize_note_write(vault.vault_id, &index.layers, outcome))
}

pub(super) async fn edit_note_tool(
    _state: AppState,
    vault: &McpVault,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: EditNoteArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid edit_note arguments: {error}"))
    })?;
    let index = current_index(vault).await?;
    let entry = note_entry(&index, &args.slug)?;
    let outcome = edit_note(
        &entry,
        &args.old_string,
        &args.new_string,
        &args.expected_content_hash,
        args.replace_all.unwrap_or(false),
    )
    .map_err(|error| write_error_to_jsonrpc(vault.vault_id, error))?;
    Ok(finalize_note_write(vault.vault_id, &index.layers, outcome))
}

pub(super) async fn replace_section_tool(
    _state: AppState,
    vault: &McpVault,
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
    let index = current_index(vault).await?;
    let entry = note_entry(&index, &args.slug)?;
    let outcome = replace_section(
        &entry,
        &heading,
        mode,
        &args.content,
        &args.expected_content_hash,
    )
    .map_err(|error| write_error_to_jsonrpc(vault.vault_id, error))?;
    Ok(finalize_note_write(vault.vault_id, &index.layers, outcome))
}

pub(super) async fn rename_note_tool(
    _state: AppState,
    vault: &McpVault,
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
    let index = current_index(vault).await?;
    let entry = note_entry(&index, &args.slug)?;
    let target = replace_filename(&entry.relative_path, &new_title);
    refuse_noise_write(&vault.exclude()?, &target)?;
    let outcome = move_or_rename_note(
        &vault_path(vault),
        &index,
        &entry,
        &target,
        &args.expected_content_hash,
    )
    .map_err(|error| write_error_to_jsonrpc(vault.vault_id, error))?;
    Ok(finalize_note_write(vault.vault_id, &index.layers, outcome))
}

pub(super) async fn move_note_tool(
    _state: AppState,
    vault: &McpVault,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: MoveNoteArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid move_note arguments: {error}"))
    })?;
    let index = current_index(vault).await?;
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
    refuse_noise_write(&vault.exclude()?, &target)?;
    let outcome = move_or_rename_note(
        &vault_path(vault),
        &index,
        &entry,
        &target,
        &args.expected_content_hash,
    )
    .map_err(|error| write_error_to_jsonrpc(vault.vault_id, error))?;
    Ok(finalize_note_write(vault.vault_id, &index.layers, outcome))
}

pub(super) async fn move_rename_note_tool(
    _state: AppState,
    vault: &McpVault,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: MoveRenameNoteArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid move_rename_note arguments: {error}"))
    })?;
    let target_relative_path =
        non_empty_argument("target_relative_path", args.target_relative_path)?;
    refuse_noise_write(&vault.exclude()?, &target_relative_path)?;
    let index = current_index(vault).await?;
    let entry = note_entry(&index, &args.slug)?;
    let outcome = move_or_rename_note(
        &vault_path(vault),
        &index,
        &entry,
        &target_relative_path,
        &args.expected_content_hash,
    )
    .map_err(|error| write_error_to_jsonrpc(vault.vault_id, error))?;
    Ok(finalize_note_write(vault.vault_id, &index.layers, outcome))
}

pub(super) async fn archive_note_tool(
    state: AppState,
    vault: &McpVault,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: ArchiveNoteArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid archive_note arguments: {error}"))
    })?;
    let index = current_index(vault).await?;
    let entry = note_entry(&index, &args.slug)?;
    let snapshot = state.runtime_snapshot();
    let archive_prefix = AppState::vault_archive_prefix(Some(vault.definition()), &snapshot)
        .map_err(JsonRpcFailure::internal)?;
    let archive_folder = archive_prefix.trim().trim_matches('/');
    let file_name = entry
        .relative_path
        .rsplit('/')
        .next()
        .unwrap_or(&entry.relative_path);
    let target = format!("{archive_folder}/{file_name}");
    refuse_noise_write(&vault.exclude()?, &target)?;
    let outcome = archive_note(
        &vault_path(vault),
        &index,
        &entry,
        &archive_prefix,
        &args.expected_content_hash,
    )
    .map_err(|error| write_error_to_jsonrpc(vault.vault_id, error))?;
    Ok(finalize_note_write(vault.vault_id, &index.layers, outcome))
}

pub(super) async fn delete_note_tool(
    _state: AppState,
    vault: &McpVault,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: DeleteNoteArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid delete_note arguments: {error}"))
    })?;
    let index = current_index(vault).await?;
    let entry = note_entry(&index, &args.slug)?;
    let outcome = delete_note(
        &vault_path(vault),
        &index,
        &entry,
        &args.expected_content_hash,
    )
    .map_err(|error| write_error_to_jsonrpc(vault.vault_id, error))?;
    Ok(finalize_note_write(vault.vault_id, &index.layers, outcome))
}

pub(super) async fn import_attachment_tool(
    _state: AppState,
    vault: &McpVault,
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
    refuse_noise_write(&vault.exclude()?, &target_relative_path)?;
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
        &vault_path(vault),
        &target_relative_path,
        &bytes,
        config.max_base64_bytes,
        overwrite,
    )
    .map_err(|error| write_error_to_jsonrpc(vault.vault_id, error))?;
    Ok(attachment_success(vault.vault_id, outcome))
}

pub(super) async fn move_attachment_tool(
    _state: AppState,
    vault: &McpVault,
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
    refuse_noise_write(&vault.exclude()?, &target_relative_path)?;
    let index = current_index(vault).await?;
    let outcome = move_attachment(
        &vault_path(vault),
        &index,
        &source_relative_path,
        &target_relative_path,
    )
    .map_err(|error| write_error_to_jsonrpc(vault.vault_id, error))?;
    Ok(attachment_success(vault.vault_id, outcome))
}

pub(super) async fn rename_attachment_tool(
    _state: AppState,
    vault: &McpVault,
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
    refuse_noise_write(&vault.exclude()?, &target_relative_path)?;
    let index = current_index(vault).await?;
    let outcome = rename_attachment(
        &vault_path(vault),
        &index,
        &source_relative_path,
        &new_filename,
    )
    .map_err(|error| write_error_to_jsonrpc(vault.vault_id, error))?;
    Ok(attachment_success(vault.vault_id, outcome))
}

pub(super) async fn delete_attachment_tool(
    _state: AppState,
    vault: &McpVault,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: DeleteAttachmentArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid delete_attachment arguments: {error}"))
    })?;
    let source_relative_path =
        non_empty_argument("source_relative_path", args.source_relative_path)?;
    let index = current_index(vault).await?;
    let outcome = delete_attachment(&vault_path(vault), &index, &source_relative_path)
        .map_err(|error| write_error_to_jsonrpc(vault.vault_id, error))?;
    Ok(attachment_success(vault.vault_id, outcome))
}

pub(super) async fn list_note_attachments_tool(
    _state: AppState,
    vault: &McpVault,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: VaultSlugArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid list_note_attachments arguments: {error}"))
    })?;
    let index = current_index(vault).await?;
    let entry = note_entry(&index, &args.slug)?;
    let attachments = list_note_attachments(&vault_path(vault), &index.layers, &entry)
        .map_err(|error| write_error_to_jsonrpc(vault.vault_id, error))?;
    Ok(tool_success(
        json!({ "vault_id": vault.vault_id, "attachments": attachments }),
    ))
}

/// Build the vault index off the async runtime. Write tools need the full
/// index to rewrite backlinks/assets, but the O(vault) walk must not block a
/// tokio worker.
async fn current_index(vault: &McpVault) -> Result<VaultIndex, JsonRpcFailure> {
    let control = vault.control.clone();
    match tokio::task::spawn_blocking(move || control.authoritative_index()).await {
        Ok(Ok(index)) => Ok(index),
        Ok(Err(error)) => Err(vault_error(crate::vault_read::runtime_error(
            vault.vault_id,
            error,
        ))),
        Err(join_error) => Err(JsonRpcFailure::internal(format!(
            "vault index build panicked: {join_error}"
        ))),
    }
}

/// Build the vault's metadata-only catalog off the async runtime, like
/// `current_index` but skipping the content-reading wikilink-graph pass:
/// `create_note` has no pre-write entry to fetch a slug from, so it needs
/// this before the write to compute one, but never needs the link graph
/// itself.
async fn current_catalog(vault: &McpVault) -> Result<VaultIndex, JsonRpcFailure> {
    let control = vault.control.clone();
    match tokio::task::spawn_blocking(move || control.authoritative_catalog()).await {
        Ok(Ok(catalog)) => Ok(catalog),
        Ok(Err(error)) => Err(vault_error(crate::vault_read::runtime_error(
            vault.vault_id,
            error,
        ))),
        Err(join_error) => Err(JsonRpcFailure::internal(format!(
            "vault catalog build panicked: {join_error}"
        ))),
    }
}

fn vault_path(vault: &McpVault) -> std::path::PathBuf {
    vault.control.vault_path().to_path_buf()
}

fn note_entry(index: &VaultIndex, slug: &str) -> Result<crate::vault::NoteEntry, JsonRpcFailure> {
    let slug = slug.trim();
    if slug.is_empty() {
        return Err(JsonRpcFailure::invalid_params("slug cannot be empty"));
    }
    index.find_by_slug(slug).cloned().ok_or_else(|| {
        JsonRpcFailure::not_found(
            json!({
                "code": "note_not_found",
                "message": format!("Note not found: {slug}"),
                "retryable": false,
            })
            .to_string(),
        )
    })
}

/// Builds the write tool's result value straight from `layers` — the
/// `LayerMap` already sitting in memory from the write's own pre-write
/// index/catalog fetch — instead of rebuilding a fresh authoritative index
/// after the write has already committed to disk (issue #101: a rescan here
/// would delay an otherwise-completed mutation response, and could turn a
/// rescan failure into a JSON-RPC error after the write already succeeded).
/// Every write op (`vault/write/notes.rs`) now sets `outcome.slug` itself, so
/// there is no lookup left to do; `layer` reports the note's resulting layer
/// (`None` = default surface) via the same path-based `layer_for` a real
/// index build uses, except a delete, which leaves no note behind and always
/// reports `None`. `trashed_path.is_some()` is currently only ever set by
/// `delete_note`, so it stands in for "this is a delete"; a future write op
/// that starts setting `trashed_path` for a non-delete reason would need to
/// update this.
fn finalize_note_write(vault_id: VaultId, layers: &LayerMap, outcome: WriteOutcome) -> Value {
    let layer = if outcome.trashed_path.is_some() {
        None
    } else {
        outcome
            .relative_path
            .as_deref()
            .and_then(|relative_path| layers.layer_for(relative_path))
            .map(str::to_string)
    };
    write_success(vault_id, outcome, layer)
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

fn write_error_to_jsonrpc(vault_id: VaultId, error: WriteError) -> JsonRpcFailure {
    // Keep a partially-applied multi-phase mutation distinguishable from an
    // ordinary write failure: it needs operator action, and the HTTP surface
    // reports it under the same code.
    if let Some(message) = error.recovery_message() {
        let error = crate::handlers::vaults::VaultApiError::new(
            "write_recovery_required",
            message.to_string(),
            Some(vault_id),
            false,
        );
        return JsonRpcFailure::not_found(serde_json::to_string(&error).unwrap_or(error.message));
    }
    let (code, message, retryable) = match error {
        WriteError::InvalidInput(message) => ("invalid_write_input", message, false),
        WriteError::Conflict(message) => ("write_conflict", message, true),
        WriteError::Io(message) => ("write_failed", message, false),
    };
    let error =
        crate::handlers::vaults::VaultApiError::new(code, message, Some(vault_id), retryable);
    JsonRpcFailure::not_found(serde_json::to_string(&error).unwrap_or(error.message))
}

fn write_success(vault_id: VaultId, outcome: WriteOutcome, layer: Option<String>) -> Value {
    tool_success(json!({
        "vault_id": vault_id,
        "ok": true,
        "slug": outcome.slug,
        "relative_path": outcome.relative_path,
        "content_hash": outcome.content_hash,
        "layer": layer,
        "quality_warnings": outcome.quality_warnings,
        "rewritten_notes": outcome.rewritten_notes,
        "moved_assets": outcome.moved_assets,
        "trashed_path": outcome.trashed_path,
    }))
}

fn attachment_success(vault_id: VaultId, outcome: AttachmentOutcome) -> Value {
    tool_success(json!({
        "vault_id": vault_id,
        "ok": true,
        "attachment": outcome.attachment,
        "rewritten_notes": outcome.rewritten_notes,
        "trashed_path": outcome.trashed_path,
        "cleanup_warning": outcome.cleanup_warning,
    }))
}

fn replace_filename(relative_path: &str, new_title: &str) -> String {
    let directory = relative_path.rsplit_once('/').map(|(dir, _)| dir);
    match directory {
        Some(dir) if !dir.is_empty() => format!("{dir}/{new_title}.md"),
        _ => format!("{new_title}.md"),
    }
}

pub(super) fn write_tools_list() -> Vec<Value> {
    let mut tools = vec![
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
            "description": "Upload an attachment into one Vault by sending its bytes base64-encoded. This is the fallback for clients that cannot make an out-of-band HTTP request; it is size-limited by HATCHDOOR_MCP_MAX_BASE64_BYTES. Prefer the Vault-scoped HTTP upload endpoint (POST /api/v1/vaults/{vault_id}/attachments) when possible. Returns compact metadata for the imported file.",
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
    ];
    for tool in &mut tools {
        let schema = tool
            .get_mut("inputSchema")
            .expect("MCP write tool has an input schema");
        let properties = schema
            .get_mut("properties")
            .and_then(Value::as_object_mut)
            .expect("MCP write tool schema has properties");
        properties.insert(
            "vault_id".to_string(),
            json!({
                "type": "string",
                "minLength": 1,
                "description": "Canonical target Vault ID. The literal all is invalid."
            }),
        );
        schema
            .get_mut("required")
            .and_then(Value::as_array_mut)
            .expect("MCP write tool schema has required arguments")
            .push(json!("vault_id"));
    }
    tools
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateNoteArgs {
    vault_id: VaultId,
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
    vault_id: VaultId,
    slug: String,
    content: String,
    expected_content_hash: String,
    #[serde(default)]
    commit_summary: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AppendNoteArgs {
    vault_id: VaultId,
    slug: String,
    content: String,
    expected_content_hash: String,
    #[serde(default)]
    commit_summary: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EditNoteArgs {
    vault_id: VaultId,
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
    vault_id: VaultId,
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
    vault_id: VaultId,
    slug: String,
    new_title: String,
    expected_content_hash: String,
    #[serde(default)]
    commit_summary: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveNoteArgs {
    vault_id: VaultId,
    slug: String,
    target_folder: String,
    expected_content_hash: String,
    #[serde(default)]
    commit_summary: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveRenameNoteArgs {
    vault_id: VaultId,
    slug: String,
    target_relative_path: String,
    expected_content_hash: String,
    #[serde(default)]
    commit_summary: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArchiveNoteArgs {
    vault_id: VaultId,
    slug: String,
    expected_content_hash: String,
    #[serde(default)]
    commit_summary: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteNoteArgs {
    vault_id: VaultId,
    slug: String,
    expected_content_hash: String,
    #[serde(default)]
    commit_summary: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportAttachmentArgs {
    vault_id: VaultId,
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
    vault_id: VaultId,
    source_relative_path: String,
    target_relative_path: String,
    #[serde(default)]
    commit_summary: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RenameAttachmentArgs {
    vault_id: VaultId,
    source_relative_path: String,
    new_filename: String,
    #[serde(default)]
    commit_summary: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteAttachmentArgs {
    vault_id: VaultId,
    source_relative_path: String,
    #[serde(default)]
    commit_summary: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct VaultSlugArgs {
    vault_id: VaultId,
    slug: String,
}

#[cfg(test)]
mod scoped_tests {
    use super::*;

    #[test]
    fn every_advertised_write_tool_requires_one_vault_id() {
        for tool in write_tools_list() {
            assert!(tool["inputSchema"]["properties"].get("vault_id").is_some());
            assert!(
                tool["inputSchema"]["required"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("vault_id"))
            );
        }
    }
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

    #[test]
    fn recovery_required_write_errors_expose_bounded_guidance() {
        let vault_id = crate::vault_registry::VaultId::generate().expect("generate Vault id");
        let failure = write_error_to_jsonrpc(
            vault_id,
            WriteError::recovery_required(
                "vault mutation rollback was incomplete: restore rewritten note [Backlink.md]"
                    .to_string(),
            ),
        );

        assert!(failure.message.contains("write_recovery_required"));
        assert!(failure.message.contains("recovery required"));
        assert!(failure.message.contains("Backlink.md"));
        assert!(!failure.message.contains("/home/"));
    }
}

#[cfg(test)]
mod finalize_tests {
    use super::*;

    #[test]
    fn finalize_note_write_derives_layer_from_the_layer_map_alone() {
        // issue #101: `finalize_note_write` takes a `&LayerMap`, not a
        // `&VaultIndex` — it structurally cannot rebuild a full authoritative
        // index (there is no index for it to rebuild), so the second
        // post-write rescan the issue flags is not just avoided but
        // impossible to reintroduce here by accident.
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let root = tmp.path();
        std::fs::create_dir_all(root.join("sources")).expect("mkdir");
        std::fs::write(root.join("sources/.hatchdoor-layer"), "name: sources\n").expect("marker");
        let layers = LayerMap::collect(root).expect("collect layers");
        let vault_id = VaultId::generate().expect("generate Vault id");

        let updated = WriteOutcome {
            slug: Some("clip".to_string()),
            relative_path: Some("sources/Clip".to_string()),
            content_hash: Some("h".to_string()),
            quality_warnings: Vec::new(),
            rewritten_notes: 0,
            moved_assets: 0,
            trashed_path: None,
            affected_paths: Vec::new(),
        };
        let value = finalize_note_write(vault_id, &layers, updated);
        assert_eq!(value["structuredContent"]["slug"], "clip");
        assert_eq!(value["structuredContent"]["layer"], "sources");

        let deleted = WriteOutcome {
            slug: Some("clip".to_string()),
            relative_path: Some("sources/Clip".to_string()),
            content_hash: None,
            quality_warnings: Vec::new(),
            rewritten_notes: 0,
            moved_assets: 0,
            trashed_path: Some(".hatchdoor-trash/sources/Clip".to_string()),
            affected_paths: Vec::new(),
        };
        let value = finalize_note_write(vault_id, &layers, deleted);
        assert!(
            value["structuredContent"]["layer"].is_null(),
            "a delete leaves no note behind, so layer stays null even though the path matches a named layer"
        );
    }
}
