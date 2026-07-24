//! MCP tool surface. Dispatch and the shared helpers live here; the tool
//! implementations and their JSON schemas are split by permission boundary
//! into `read` (always available) and `write` (gated by
//! `HATCHDOOR_MCP_WRITE_ENABLED`), mirroring how `McpConfig` gates them.

mod read;
mod write;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::app_state::AppState;

use super::config::McpConfig;
use super::protocol::{JsonRpcFailure, tool_error};

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

    let outcome = match name {
        "search_notes" => read::search_notes_tool(state, arguments).await,
        "query_notes" => read::query_notes_tool(state, arguments).await,
        "get_note" => read::get_note_tool(state, arguments).await,
        "get_note_links" => read::get_note_links_tool(state, arguments).await,
        "resolve_wikilink" => read::resolve_wikilink_tool(state, arguments).await,
        "get_tree" => read::get_tree_tool(state, arguments).await,
        "recently_modified" => read::recently_modified_tool(state, arguments).await,
        "refresh_index" => read::refresh_index_tool(state, arguments).await,
        "get_git_sync_status" => read::get_git_sync_status_tool(state).await,
        "get_attachment_import_config" => read::get_attachment_import_config_tool(config),
        "create_note" | "update_note" | "append_to_note" | "edit_note" | "replace_section"
        | "rename_note" | "move_note" | "move_rename_note" | "archive_note" | "delete_note"
        | "import_attachment" | "move_attachment" | "rename_attachment" | "delete_attachment"
            if config.write_enabled =>
        {
            // Hold the vault write lock for the whole tool call so a concurrent
            // git-sync merge/reset cannot race a filesystem write.
            let _guard = state.vault_write_lock.clone().lock_owned().await;
            match name {
                "create_note" => write::create_note_tool(state, arguments).await,
                "update_note" => write::update_note_tool(state, arguments).await,
                "append_to_note" => write::append_to_note_tool(state, arguments).await,
                "edit_note" => write::edit_note_tool(state, arguments).await,
                "replace_section" => write::replace_section_tool(state, arguments).await,
                "rename_note" => write::rename_note_tool(state, arguments).await,
                "move_note" => write::move_note_tool(state, arguments).await,
                "move_rename_note" => write::move_rename_note_tool(state, arguments).await,
                "archive_note" => write::archive_note_tool(state, arguments).await,
                "delete_note" => write::delete_note_tool(state, arguments).await,
                "import_attachment" => {
                    write::import_attachment_tool(state, arguments, config).await
                }
                "move_attachment" => write::move_attachment_tool(state, arguments).await,
                "rename_attachment" => write::rename_attachment_tool(state, arguments).await,
                "delete_attachment" => write::delete_attachment_tool(state, arguments).await,
                _ => unreachable!(),
            }
        }
        "list_note_attachments" if config.write_enabled => {
            write::list_note_attachments_tool(state, arguments).await
        }
        "create_note"
        | "update_note"
        | "append_to_note"
        | "edit_note"
        | "replace_section"
        | "rename_note"
        | "move_note"
        | "move_rename_note"
        | "archive_note"
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
    };

    // Tool-level failures (e.g. "note not found") are rendered as an isError
    // tool result so read and write tools report the same conditions the same
    // way; genuine protocol errors stay JSON-RPC errors.
    match outcome {
        Err(failure) if failure.tool_level => Ok(tool_error(failure.message)),
        other => other,
    }
}

pub fn tools_list(config: &McpConfig, layers: &[crate::search::LayerInfo]) -> Vec<Value> {
    let mut tools = read::read_tools_list(layers);
    if config.write_enabled {
        tools.extend(write::write_tools_list());
    }
    tools
}

/// Arguments for the several tools keyed only by a note slug (read and write).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SlugArgs {
    pub slug: String,
}

pub(super) fn non_empty_argument(name: &str, value: String) -> Result<String, JsonRpcFailure> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(JsonRpcFailure::invalid_params(format!(
            "{name} cannot be empty"
        )));
    }
    Ok(value)
}

pub(super) fn read_only_tool_annotations() -> Value {
    json!({
        "readOnlyHint": true,
        "destructiveHint": false,
        "idempotentHint": true,
        "openWorldHint": false,
    })
}

pub(super) fn refresh_tool_annotations() -> Value {
    json!({
        "readOnlyHint": false,
        "destructiveHint": false,
        "idempotentHint": true,
        "openWorldHint": false,
    })
}

pub(super) fn write_tool_annotations(destructive: bool, idempotent: bool) -> Value {
    json!({
        "readOnlyHint": false,
        "destructiveHint": destructive,
        "idempotentHint": idempotent,
        "openWorldHint": false,
    })
}
