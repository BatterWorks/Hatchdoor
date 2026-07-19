//! Read-only MCP tools: search, note/link/tree lookups, and status. Always
//! available whenever MCP is enabled.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::api_types::RefreshResponse;
use crate::app_state::{AppState, refresh_now, sqlite_cache};
use crate::search::SearchRequest;
use crate::vault::allowed_attachment_extensions;

use super::super::config::McpConfig;
use super::super::protocol::{JsonRpcFailure, tool_error, tool_success};
use super::{SlugArgs, non_empty_argument, read_only_tool_annotations, refresh_tool_annotations};

pub(super) async fn search_notes_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
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

pub(super) async fn get_note_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
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

pub(super) async fn get_note_links_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
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

pub(super) async fn resolve_wikilink_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
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

pub(super) async fn get_tree_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    reject_non_empty_arguments("get_tree", &arguments)?;
    let cache = sqlite_cache(&state)
        .await
        .map_err(|(_status, body)| JsonRpcFailure::internal(body.0.error))?;
    let tree = cache.explorer_tree().map_err(JsonRpcFailure::internal)?;

    Ok(tool_success(json!({ "tree": tree })))
}

pub(super) async fn refresh_index_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    reject_non_empty_arguments("refresh_index", &arguments)?;
    refresh_now(&state)
        .await
        .map_err(|(_status, body)| JsonRpcFailure::internal(body.0.error))?;

    Ok(tool_success(json!(RefreshResponse { refreshed: true })))
}

pub(super) async fn get_git_sync_status_tool(state: AppState) -> Result<Value, JsonRpcFailure> {
    let status = match state.git_sync.get() {
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

pub(super) fn get_attachment_import_config_tool(
    config: &McpConfig,
) -> Result<Value, JsonRpcFailure> {
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

pub(super) fn read_tools_list() -> Vec<Value> {
    vec![
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
struct ResolveWikilinkArgs {
    target: String,
}
