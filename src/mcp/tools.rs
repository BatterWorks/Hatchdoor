use serde::Deserialize;
use serde_json::{Value, json};

use crate::api_types::RefreshResponse;
use crate::app_state::{AppState, refresh_now, sqlite_cache};
use crate::search::SearchRequest;
use crate::vault::VaultIndex;
use crate::vault::{
    AttachmentOutcome, SectionMode, WriteError, WriteOutcome, allowed_attachment_extensions,
    append_note, create_note, delete_attachment, delete_note, edit_note, import_attachment,
    list_note_attachments, move_attachment, move_or_rename_note, rename_attachment,
    replace_section, update_note,
};

use super::config::McpConfig;
use super::protocol::{JsonRpcFailure, tool_error, tool_success};

pub async fn handle_tools_call(
    state: AppState,
    params: Option<Value>,
    config: &McpConfig,
) -> Result<Value, JsonRpcFailure> {
    let params =
        params.ok_or_else(|| JsonRpcFailure::invalid_params("Missing tool call params"))?;
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| JsonRpcFailure::invalid_params("Missing tool name"))?;
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    match name {
        "search_notes" => search_notes_tool(state, arguments).await,
        "get_note" => get_note_tool(state, arguments).await,
        "get_note_links" => get_note_links_tool(state, arguments).await,
        "resolve_wikilink" => resolve_wikilink_tool(state, arguments).await,
        "get_tree" => get_tree_tool(state, arguments).await,
        "refresh_index" => refresh_index_tool(state, arguments).await,
        "get_git_sync_status" => get_git_sync_status_tool(state).await,
        "get_attachment_import_config" if config.write_enabled => {
            get_attachment_import_config_tool(config)
        }
        "get_attachment_import_config" => Ok(tool_success(json!({
            "enabled": false,
            "staging_path": null,
            "staging_path_kind": "hidden",
            "host_staging_path": null,
            "host_staging_path_kind": "hidden",
            "allowed_extensions": allowed_attachment_extensions(),
            "max_bytes": config.max_attachment_bytes,
            "usage": "Enable HATCHDOOR_MCP_WRITE_ENABLED to use staged attachment imports."
        }))),
        "create_note" | "update_note" | "append_to_note" | "edit_note" | "replace_section"
        | "rename_note" | "move_note" | "move_rename_note" | "delete_note"
        | "import_attachment" | "move_attachment" | "rename_attachment" | "delete_attachment"
            if config.write_enabled =>
        {
            // Hold the vault write lock for the whole tool call so a concurrent
            // git-sync merge/reset cannot race a filesystem write.
            let _guard = state.vault_write_lock.clone().lock_owned().await;
            match name {
                "create_note" => create_note_tool(state, arguments).await,
                "update_note" => update_note_tool(state, arguments).await,
                "append_to_note" => append_to_note_tool(state, arguments).await,
                "edit_note" => edit_note_tool(state, arguments).await,
                "replace_section" => replace_section_tool(state, arguments).await,
                "rename_note" => rename_note_tool(state, arguments).await,
                "move_note" => move_note_tool(state, arguments).await,
                "move_rename_note" => move_rename_note_tool(state, arguments).await,
                "delete_note" => delete_note_tool(state, arguments).await,
                "import_attachment" => import_attachment_tool(state, arguments, config).await,
                "move_attachment" => move_attachment_tool(state, arguments).await,
                "rename_attachment" => rename_attachment_tool(state, arguments).await,
                "delete_attachment" => delete_attachment_tool(state, arguments).await,
                _ => unreachable!(),
            }
        }
        "list_note_attachments" if config.write_enabled => {
            list_note_attachments_tool(state, arguments).await
        }
        "create_note"
        | "update_note"
        | "append_to_note"
        | "edit_note"
        | "replace_section"
        | "rename_note"
        | "move_note"
        | "move_rename_note"
        | "delete_note"
        | "import_attachment"
        | "move_attachment"
        | "rename_attachment"
        | "delete_attachment"
        | "list_note_attachments" => Err(JsonRpcFailure::invalid_params(
            "MCP write tools are disabled by HATCHDOOR_MCP_WRITE_ENABLED",
        )),
        other => Err(JsonRpcFailure::invalid_params(format!(
            "Unknown MCP tool: {other}"
        ))),
    }
}

pub fn tools_list(config: &McpConfig) -> Vec<Value> {
    let mut tools = vec![
        json!({
            "name": "search_notes",
            "description": "Semantic-first chunk search across the vault. Returns ranked chunks with parent note metadata and the parent note's outbound wikilinks. The default semantic mode uses vector similarity — phrase queries as natural language descriptions of what you're looking for, not keyword lists (e.g. \"how should I structure my backup strategy\" beats \"backup strategy\"). Use mode=\"keyword\" when the exact term or phrasing matters (tags, proper names, code symbols). Use get_note for full note content of a returned slug.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Search query."
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["semantic", "keyword"],
                        "default": "semantic",
                        "description": "Retrieval mode. semantic = vector similarity (default). keyword = FTS5 BM25 over chunk content."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 50,
                        "default": 10
                    },
                    "per_note_cap": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 10,
                        "default": 2,
                        "description": "Maximum number of chunks returned from any single note."
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            },
            "annotations": read_only_tool_annotations()
        }),
        json!({
            "name": "get_note",
            "description": "Fetch full Markdown content for one known slug. Use only after search_notes or resolve_wikilink identifies the slug; avoid fetching many full notes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "slug": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Hatchdoor note slug."
                    }
                },
                "required": ["slug"],
                "additionalProperties": false
            },
            "annotations": read_only_tool_annotations()
        }),
        json!({
            "name": "get_note_links",
            "description": "Fetch outgoing links and backlinks for one known slug. Use when note relationships help answer the user.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "slug": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Hatchdoor note slug."
                    }
                },
                "required": ["slug"],
                "additionalProperties": false
            },
            "annotations": read_only_tool_annotations()
        }),
        json!({
            "name": "resolve_wikilink",
            "description": "Resolve an Obsidian wikilink target to a Hatchdoor slug before fetching a note.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "target": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Wikilink target without surrounding [[ ]]."
                    }
                },
                "required": ["target"],
                "additionalProperties": false
            },
            "annotations": read_only_tool_annotations()
        }),
        json!({
            "name": "get_tree",
            "description": "Return the full explorer tree. Use only for vault structure, folders, or navigation questions; do not use for normal search or Q&A.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            },
            "annotations": read_only_tool_annotations()
        }),
        json!({
            "name": "refresh_index",
            "description": "Refresh Hatchdoor's SQLite view of the vault. Only needed for changes made outside this MCP session (e.g. the user edited a note directly). All write tools already trigger a synchronous reindex before returning, so do not call this after create_note, update_note, append_to_note, or any other write tool.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            },
            "annotations": refresh_tool_annotations()
        }),
        json!({
            "name": "get_attachment_import_config",
            "description": "Return MCP attachment staging configuration, allowed extensions, max size, and usage guidance. Use before importing attachments.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            },
            "annotations": read_only_tool_annotations()
        }),
        json!({
            "name": "get_git_sync_status",
            "description": "Report the status of automatic git sync: whether it is enabled, the last sync time, whether the last attempt succeeded, the last error (if any), and how many writes are pending. Use to check whether your changes have been committed and pushed.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            },
            "annotations": read_only_tool_annotations()
        }),
    ];
    if config.write_enabled {
        tools.extend(write_tools_list());
    }
    tools
}

async fn search_notes_tool(state: AppState, arguments: Value) -> Result<Value, JsonRpcFailure> {
    let args: SearchNotesArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid search_notes arguments: {error}"))
    })?;
    let query = args.query.trim().to_string();
    if query.is_empty() {
        return Err(JsonRpcFailure::invalid_params(
            "search_notes query cannot be empty",
        ));
    }

    let limit = args.limit.unwrap_or(10).clamp(1, 50);
    let per_note_cap = args.per_note_cap.unwrap_or(2).clamp(1, 10);
    let mode = args.mode.unwrap_or_default();

    let cache = sqlite_cache(&state)
        .await
        .map_err(|(_status, body)| JsonRpcFailure::internal(body.0.error))?;
    let embedder = state.embedder.as_ref();

    let req = SearchRequest {
        query,
        mode,
        limit,
        per_note_cap,
    };
    let response =
        crate::search::run(cache.as_ref(), embedder, req).map_err(JsonRpcFailure::internal)?;

    Ok(tool_success(serde_json::to_value(&response).map_err(
        |e| JsonRpcFailure::internal(format!("serialize search response: {e}")),
    )?))
}

async fn get_note_tool(state: AppState, arguments: Value) -> Result<Value, JsonRpcFailure> {
    let args: SlugArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid get_note arguments: {error}"))
    })?;
    let slug = non_empty_argument("slug", args.slug)?;
    let cache = sqlite_cache(&state)
        .await
        .map_err(|(_status, body)| JsonRpcFailure::internal(body.0.error))?;
    let note = cache
        .read_note_by_slug(&slug)
        .map_err(JsonRpcFailure::internal)?;

    match note {
        Some(note) => Ok(tool_success(json!({ "note": note }))),
        None => Ok(tool_error(format!("Note not found: {slug}"))),
    }
}

async fn get_note_links_tool(state: AppState, arguments: Value) -> Result<Value, JsonRpcFailure> {
    let args: SlugArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid get_note_links arguments: {error}"))
    })?;
    let slug = non_empty_argument("slug", args.slug)?;
    let cache = sqlite_cache(&state)
        .await
        .map_err(|(_status, body)| JsonRpcFailure::internal(body.0.error))?;

    match cache.note_links(&slug).map_err(JsonRpcFailure::internal)? {
        Some(links) => Ok(tool_success(json!({ "links": links }))),
        None => Ok(tool_error(format!("Note not found: {slug}"))),
    }
}

async fn resolve_wikilink_tool(state: AppState, arguments: Value) -> Result<Value, JsonRpcFailure> {
    let args: ResolveWikilinkArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid resolve_wikilink arguments: {error}"))
    })?;
    let target = non_empty_argument("target", args.target)?;
    let cache = sqlite_cache(&state)
        .await
        .map_err(|(_status, body)| JsonRpcFailure::internal(body.0.error))?;

    let slug = cache
        .resolve_wikilink(&target)
        .map_err(JsonRpcFailure::internal)?;
    Ok(tool_success(json!({ "slug": slug })))
}

async fn get_tree_tool(state: AppState, arguments: Value) -> Result<Value, JsonRpcFailure> {
    reject_non_empty_arguments("get_tree", &arguments)?;
    let cache = sqlite_cache(&state)
        .await
        .map_err(|(_status, body)| JsonRpcFailure::internal(body.0.error))?;
    let tree = cache.explorer_tree().map_err(JsonRpcFailure::internal)?;

    Ok(tool_success(json!({ "tree": tree })))
}

async fn refresh_index_tool(state: AppState, arguments: Value) -> Result<Value, JsonRpcFailure> {
    reject_non_empty_arguments("refresh_index", &arguments)?;
    refresh_now(&state)
        .await
        .map_err(|(_status, body)| JsonRpcFailure::internal(body.0.error))?;

    Ok(tool_success(json!(RefreshResponse { refreshed: true })))
}

async fn get_git_sync_status_tool(state: AppState) -> Result<Value, JsonRpcFailure> {
    let status = match &state.git_sync {
        Some(handle) => {
            let guard = handle.status();
            let snapshot = guard.read().await;
            serde_json::to_value(&*snapshot)
                .map_err(|e| JsonRpcFailure::internal(format!("serialize git status: {e}")))?
        }
        None => json!({
            "enabled": false,
            "last_sync_at": null,
            "last_ok": false,
            "last_error": null,
            "last_error_kind": null,
            "pending": 0,
            "unpushed": 0
        }),
    };
    Ok(tool_success(status))
}

/// Returns the last sync error message when the most recent sync failed.
async fn git_sync_warning(state: &AppState) -> Option<String> {
    let handle = state.git_sync.as_ref()?;
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

fn get_attachment_import_config_tool(config: &McpConfig) -> Result<Value, JsonRpcFailure> {
    let host_staging_path = if config.advertise_host_paths {
        config.host_attachment_staging_path.clone()
    } else {
        None
    };
    Ok(tool_success(json!({
        "enabled": config.write_enabled && config.attachment_staging_path.is_some(),
        "staging_path": config
            .attachment_staging_path
            .as_ref()
            .map(|path| path.display().to_string()),
        "staging_path_kind": "container",
        "host_staging_path": host_staging_path,
        "host_staging_path_kind": if config.advertise_host_paths { "host_hint" } else { "hidden" },
        "allowed_extensions": allowed_attachment_extensions(),
        "max_bytes": config.max_attachment_bytes,
        "usage": "Place files in the advertised staging folder, then call import_attachment with staged_filename and target_relative_path."
    })))
}

async fn create_note_tool(state: AppState, arguments: Value) -> Result<Value, JsonRpcFailure> {
    let args: CreateNoteArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid create_note arguments: {error}"))
    })?;
    let relative_path = non_empty_argument("relative_path", args.relative_path)?;
    let overwrite = args.overwrite.unwrap_or(false);
    let outcome = create_note(&state.vault_path, &relative_path, &args.content, overwrite)
        .map_err(write_error_to_jsonrpc)?;
    refresh_after_write(&state).await?;
    record_note_write(&state, "create", &outcome, args.commit_summary);
    let warning = git_sync_warning(&state).await;
    Ok(write_success(outcome, warning))
}

async fn update_note_tool(state: AppState, arguments: Value) -> Result<Value, JsonRpcFailure> {
    let args: UpdateNoteArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid update_note arguments: {error}"))
    })?;
    let index = current_index(&state).await?;
    let entry = note_entry(&index, &args.slug)?;
    let outcome = update_note(&entry, &args.content, &args.expected_content_hash)
        .map_err(write_error_to_jsonrpc)?;
    refresh_after_write(&state).await?;
    record_note_write(&state, "update", &outcome, args.commit_summary);
    let warning = git_sync_warning(&state).await;
    Ok(write_success(outcome, warning))
}

async fn append_to_note_tool(state: AppState, arguments: Value) -> Result<Value, JsonRpcFailure> {
    let args: AppendNoteArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid append_to_note arguments: {error}"))
    })?;
    let content = non_empty_argument("content", args.content)?;
    let index = current_index(&state).await?;
    let entry = note_entry(&index, &args.slug)?;
    let outcome = append_note(&entry, &content, &args.expected_content_hash)
        .map_err(write_error_to_jsonrpc)?;
    refresh_after_write(&state).await?;
    record_note_write(&state, "append", &outcome, args.commit_summary);
    let warning = git_sync_warning(&state).await;
    Ok(write_success(outcome, warning))
}

async fn edit_note_tool(state: AppState, arguments: Value) -> Result<Value, JsonRpcFailure> {
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
    refresh_after_write(&state).await?;
    record_note_write(&state, "edit", &outcome, args.commit_summary);
    let warning = git_sync_warning(&state).await;
    Ok(write_success(outcome, warning))
}

async fn replace_section_tool(state: AppState, arguments: Value) -> Result<Value, JsonRpcFailure> {
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
    refresh_after_write(&state).await?;
    record_note_write(&state, "replace_section", &outcome, args.commit_summary);
    let warning = git_sync_warning(&state).await;
    Ok(write_success(outcome, warning))
}

async fn rename_note_tool(state: AppState, arguments: Value) -> Result<Value, JsonRpcFailure> {
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
    let outcome = move_or_rename_note(
        &state.vault_path,
        &index,
        &entry,
        &target,
        &args.expected_content_hash,
    )
    .map_err(write_error_to_jsonrpc)?;
    refresh_after_write(&state).await?;
    record_note_write(&state, "rename", &outcome, args.commit_summary);
    let warning = git_sync_warning(&state).await;
    Ok(write_success(outcome, warning))
}

async fn move_note_tool(state: AppState, arguments: Value) -> Result<Value, JsonRpcFailure> {
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
    let outcome = move_or_rename_note(
        &state.vault_path,
        &index,
        &entry,
        &target,
        &args.expected_content_hash,
    )
    .map_err(write_error_to_jsonrpc)?;
    refresh_after_write(&state).await?;
    record_note_write(&state, "move", &outcome, args.commit_summary);
    let warning = git_sync_warning(&state).await;
    Ok(write_success(outcome, warning))
}

async fn move_rename_note_tool(state: AppState, arguments: Value) -> Result<Value, JsonRpcFailure> {
    let args: MoveRenameNoteArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid move_rename_note arguments: {error}"))
    })?;
    let target_relative_path =
        non_empty_argument("target_relative_path", args.target_relative_path)?;
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
    refresh_after_write(&state).await?;
    record_note_write(&state, "move_rename", &outcome, args.commit_summary);
    let warning = git_sync_warning(&state).await;
    Ok(write_success(outcome, warning))
}

async fn delete_note_tool(state: AppState, arguments: Value) -> Result<Value, JsonRpcFailure> {
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
    refresh_after_write(&state).await?;
    record_note_write(&state, "delete", &outcome, args.commit_summary);
    let warning = git_sync_warning(&state).await;
    Ok(write_success(outcome, warning))
}

async fn import_attachment_tool(
    state: AppState,
    arguments: Value,
    config: &McpConfig,
) -> Result<Value, JsonRpcFailure> {
    let args: ImportAttachmentArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid import_attachment arguments: {error}"))
    })?;
    let staging_path = config.attachment_staging_path.as_ref().ok_or_else(|| {
        JsonRpcFailure::invalid_params("HATCHDOOR_MCP_ATTACHMENT_STAGING_PATH is not configured")
    })?;
    let staged_filename = non_empty_argument("staged_filename", args.staged_filename)?;
    let target_relative_path =
        non_empty_argument("target_relative_path", args.target_relative_path)?;
    let overwrite = args.overwrite.unwrap_or(false);
    let outcome = import_attachment(
        &state.vault_path,
        staging_path,
        &staged_filename,
        &target_relative_path,
        config.max_attachment_bytes,
        overwrite,
    )
    .map_err(write_error_to_jsonrpc)?;
    record_attachment_write(&state, "import_attachment", &outcome, args.commit_summary);
    let warning = git_sync_warning(&state).await;
    Ok(attachment_success(outcome, warning))
}

async fn move_attachment_tool(state: AppState, arguments: Value) -> Result<Value, JsonRpcFailure> {
    let args: MoveAttachmentArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid move_attachment arguments: {error}"))
    })?;
    let source_relative_path =
        non_empty_argument("source_relative_path", args.source_relative_path)?;
    let target_relative_path =
        non_empty_argument("target_relative_path", args.target_relative_path)?;
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

async fn rename_attachment_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: RenameAttachmentArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid rename_attachment arguments: {error}"))
    })?;
    let source_relative_path =
        non_empty_argument("source_relative_path", args.source_relative_path)?;
    let new_filename = non_empty_argument("new_filename", args.new_filename)?;
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

async fn delete_attachment_tool(
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

async fn list_note_attachments_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: SlugArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid list_note_attachments arguments: {error}"))
    })?;
    let index = current_index(&state).await?;
    let entry = note_entry(&index, &args.slug)?;
    let attachments =
        list_note_attachments(&state.vault_path, &entry).map_err(write_error_to_jsonrpc)?;
    Ok(tool_success(json!({ "attachments": attachments })))
}

/// Build the vault index off the async runtime. Write tools need the full
/// index to rewrite backlinks/assets, but the O(vault) walk must not block a
/// tokio worker.
async fn current_index(state: &AppState) -> Result<VaultIndex, JsonRpcFailure> {
    let vault_path = state.vault_path.clone();
    match tokio::task::spawn_blocking(move || VaultIndex::build(&vault_path)).await {
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
        .ok_or_else(|| JsonRpcFailure::invalid_params(format!("Note not found: {slug}")))
}

async fn refresh_after_write(state: &AppState) -> Result<(), JsonRpcFailure> {
    refresh_now(state)
        .await
        .map_err(|(_status, body)| JsonRpcFailure::internal(body.0.error))
}

fn write_error_to_jsonrpc(error: WriteError) -> JsonRpcFailure {
    match error {
        WriteError::InvalidInput(message) => JsonRpcFailure::invalid_params(message),
        WriteError::Conflict(message) => JsonRpcFailure::invalid_params(message),
        WriteError::Io(message) => JsonRpcFailure::internal(message),
    }
}

fn write_success(outcome: WriteOutcome, git_sync_warning: Option<String>) -> Value {
    tool_success(json!({
        "ok": true,
        "slug": outcome.slug,
        "relative_path": outcome.relative_path,
        "content_hash": outcome.content_hash,
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

fn reject_non_empty_arguments(tool_name: &str, arguments: &Value) -> Result<(), JsonRpcFailure> {
    if arguments
        .as_object()
        .map(|object| object.is_empty())
        .unwrap_or(false)
    {
        return Ok(());
    }

    Err(JsonRpcFailure::invalid_params(format!(
        "{tool_name} does not accept arguments"
    )))
}

fn non_empty_argument(name: &str, value: String) -> Result<String, JsonRpcFailure> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(JsonRpcFailure::invalid_params(format!(
            "{name} cannot be empty"
        )));
    }
    Ok(value)
}

fn read_only_tool_annotations() -> Value {
    json!({
        "readOnlyHint": true,
        "destructiveHint": false,
        "idempotentHint": true,
        "openWorldHint": false,
    })
}

fn refresh_tool_annotations() -> Value {
    json!({
        "readOnlyHint": false,
        "destructiveHint": false,
        "idempotentHint": true,
        "openWorldHint": false,
    })
}

fn write_tool_annotations(destructive: bool, idempotent: bool) -> Value {
    json!({
        "readOnlyHint": false,
        "destructiveHint": destructive,
        "idempotentHint": idempotent,
        "openWorldHint": false,
    })
}

fn write_tools_list() -> Vec<Value> {
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
            "description": "Import an attachment from the configured staging folder into the vault. staged_filename must be a filename only. Returns compact metadata for the imported file.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "staged_filename": {"type": "string", "minLength": 1},
                    "target_relative_path": {"type": "string", "minLength": 1},
                    "overwrite": {"type": "boolean", "default": false},
                    "commit_summary": {"type": "string", "description": "Optional one-line summary of this change for the git commit body."}
                },
                "required": ["staged_filename", "target_relative_path"],
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
struct SearchNotesArgs {
    query: String,
    #[serde(default)]
    mode: Option<crate::search::SearchMode>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    per_note_cap: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SlugArgs {
    slug: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolveWikilinkArgs {
    target: String,
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
struct DeleteNoteArgs {
    slug: String,
    expected_content_hash: String,
    #[serde(default)]
    commit_summary: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportAttachmentArgs {
    staged_filename: String,
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
