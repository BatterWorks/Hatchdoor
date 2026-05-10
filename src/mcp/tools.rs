use serde::Deserialize;
use serde_json::{Value, json};

use crate::api_types::RefreshResponse;
use crate::app_state::{AppState, refresh_if_needed, sqlite_cache};

use super::protocol::{JsonRpcFailure, tool_error, tool_success};

pub(crate) async fn handle_tools_call(
    state: AppState,
    params: Option<Value>,
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
        other => Err(JsonRpcFailure::invalid_params(format!(
            "Unknown MCP tool: {other}"
        ))),
    }
}

pub(crate) fn tools_list() -> Vec<Value> {
    vec![
        json!({
            "name": "search_notes",
            "description": "Search notes and return compact results. Use this first for most questions. Start with include_content=false, then set include_content=true only when title/path search is not enough. Use get_note for selected slugs.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Search query."
                    },
                    "include_content": {
                        "type": "boolean",
                        "default": false,
                        "description": "Also search note content when title/path search is not enough."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 50,
                        "default": 10
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
            "description": "Refresh Hatchdoor's SQLite view of the vault. Use only when the user says files changed or results appear stale; do not call before every search.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            },
            "annotations": refresh_tool_annotations()
        }),
    ]
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
    let include_content = args.include_content.unwrap_or(false);
    let cache = sqlite_cache(&state)
        .await
        .map_err(|(_status, body)| JsonRpcFailure::internal(body.0.error))?;
    let results = cache
        .search(&query, include_content, limit)
        .map_err(JsonRpcFailure::internal)?;

    Ok(tool_success(json!({ "results": results })))
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
    refresh_if_needed(&state, true)
        .await
        .map_err(|(_status, body)| JsonRpcFailure::internal(body.0.error))?;

    Ok(tool_success(json!(RefreshResponse { refreshed: true })))
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchNotesArgs {
    query: String,
    #[serde(default)]
    include_content: Option<bool>,
    #[serde(default)]
    limit: Option<usize>,
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
