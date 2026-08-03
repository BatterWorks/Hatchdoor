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
use super::{non_empty_argument, read_only_tool_annotations, refresh_tool_annotations};

pub(super) async fn search_notes_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: SearchNotesArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid search_notes arguments: {error}"))
    })?;
    validate_metadata_query(&args.filters, &args.include_properties)?;
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

    let layers = parse_layer_selection(cache.as_ref(), &args.layers)?;
    check_path_prefix_precedence(cache.as_ref(), &args.filters, &layers)?;
    let req = SearchRequest {
        query,
        mode,
        limit,
        per_note_cap,
        filters: args.filters,
        include_properties: args.include_properties,
        layers,
    };
    let response =
        crate::search::run(cache.as_ref(), embedder, req).map_err(JsonRpcFailure::internal)?;

    Ok(tool_success(serde_json::to_value(&response).map_err(
        |e| JsonRpcFailure::internal(format!("serialize search response: {e}")),
    )?))
}

pub(super) async fn query_notes_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: QueryNotesArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid query_notes arguments: {error}"))
    })?;
    validate_metadata_query(&args.filters, &args.include_properties)?;
    if args.filters.is_empty() {
        return Err(JsonRpcFailure::invalid_params(
            "query_notes requires at least one metadata filter",
        ));
    }
    let cache = sqlite_cache(&state)
        .await
        .map_err(|(_status, body)| JsonRpcFailure::internal(body.0.error))?;
    let layers = parse_layer_selection(cache.as_ref(), &args.layers)?;
    check_path_prefix_precedence(cache.as_ref(), &args.filters, &layers)?;
    let notes = crate::search::query_notes(
        cache.as_ref(),
        &args.filters,
        &args.include_properties,
        args.limit.unwrap_or(50).clamp(1, 200),
        &layers,
    )
    .map_err(JsonRpcFailure::internal)?;
    Ok(tool_success(json!({ "notes": notes })))
}

pub(super) async fn get_note_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: GetNoteArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid get_note arguments: {error}"))
    })?;
    let cache = sqlite_cache(&state)
        .await
        .map_err(|(_status, body)| JsonRpcFailure::internal(body.0.error))?;

    // Exactly one of `slug` or `path` addresses the note. Both reach any layer;
    // the response carries the note's `layer` so the caller knows its surface.
    let (note, address) = match (args.slug, args.path) {
        (Some(slug), None) => {
            let slug = non_empty_argument("slug", slug)?;
            let note = cache
                .read_note_by_slug(&slug)
                .map_err(JsonRpcFailure::internal)?;
            (note, slug)
        }
        (None, Some(path)) => {
            let path = non_empty_argument("path", path)?;
            let note = cache
                .read_note_by_path(&path)
                .map_err(JsonRpcFailure::internal)?;
            (note, path)
        }
        (Some(_), Some(_)) => {
            return Err(JsonRpcFailure::invalid_params(
                "get_note takes exactly one of slug or path, not both",
            ));
        }
        (None, None) => {
            return Err(JsonRpcFailure::invalid_params(
                "get_note requires either slug or path",
            ));
        }
    };

    match note {
        Some(note) => Ok(tool_success(json!({ "note": note }))),
        None => Ok(tool_error(format!("Note not found: {address}"))),
    }
}

pub(super) async fn get_note_links_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: LinksArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid get_note_links arguments: {error}"))
    })?;
    let slug = non_empty_argument("slug", args.slug)?;
    let cache = sqlite_cache(&state)
        .await
        .map_err(|(_status, body)| JsonRpcFailure::internal(body.0.error))?;

    // The selection scopes which backlinks are visible; forward links always
    // resolve across the boundary regardless (a default-surface note may point
    // into a demoted one).
    let layers = parse_layer_selection(cache.as_ref(), &args.layers)?;
    match cache
        .note_links(&slug, &layers)
        .map_err(JsonRpcFailure::internal)?
    {
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
    let args: LayersOnlyArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid get_tree arguments: {error}"))
    })?;
    let cache = sqlite_cache(&state)
        .await
        .map_err(|(_status, body)| JsonRpcFailure::internal(body.0.error))?;
    let layers = parse_layer_selection(cache.as_ref(), &args.layers)?;
    let tree = cache
        .explorer_tree(&layers)
        .map_err(JsonRpcFailure::internal)?;

    Ok(tool_success(json!({ "tree": tree })))
}

pub(super) async fn recently_modified_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: RecentlyModifiedArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid recently_modified arguments: {error}"))
    })?;
    let limit = args.limit.unwrap_or(20).clamp(1, 100);
    let cache = sqlite_cache(&state)
        .await
        .map_err(|(_status, body)| JsonRpcFailure::internal(body.0.error))?;
    let layers = parse_layer_selection(cache.as_ref(), &args.layers)?;
    let notes = cache
        .recently_modified_notes(limit, &layers)
        .map_err(JsonRpcFailure::internal)?;

    Ok(tool_success(json!({ "notes": notes })))
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LayerDiagnosticsArgs {
    #[serde(default)]
    path: Option<String>,
}

pub(super) async fn layer_diagnostics_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    // Disabled under demo mode (it reveals demoted paths). MCP is already refused
    // alongside demo_mode at startup, so this is a defensive belt-and-braces guard.
    if state.demo_mode {
        return Err(JsonRpcFailure::invalid_params(
            "layer_diagnostics is disabled in demo mode",
        ));
    }
    let args: LayerDiagnosticsArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid layer_diagnostics arguments: {error}"))
    })?;
    let cache = sqlite_cache(&state)
        .await
        .map_err(|(_status, body)| JsonRpcFailure::internal(body.0.error))?;
    let vault_path = state.vault_path.clone();
    let scan_config = state.scan_config.clone();
    let diagnostics = tokio::task::spawn_blocking(move || {
        crate::handlers::diagnostics::build_layer_diagnostics(
            &vault_path,
            &scan_config,
            &cache,
            args.path.as_deref(),
        )
    })
    .await
    .map_err(|join_error| {
        JsonRpcFailure::internal(format!("diagnostics task panicked: {join_error}"))
    })?
    .map_err(JsonRpcFailure::internal)?;

    Ok(tool_success(serde_json::to_value(&diagnostics).map_err(
        |e| JsonRpcFailure::internal(format!("serialize diagnostics: {e}")),
    )?))
}

pub(super) async fn get_git_sync_status_tool(state: AppState) -> Result<Value, JsonRpcFailure> {
    let sync = state.git_sync.read().await;
    let status = match sync.as_ref() {
        Some(handle) => {
            let guard = handle.status();
            let snapshot = guard.read().await;
            serde_json::to_value(&*snapshot)
                .map_err(|e| JsonRpcFailure::internal(format!("serialize git status: {e}")))?
        }
        None => json!({
            "enabled": false,
            "state": "disabled",
            "mode": "off",
            "last_sync_at": null,
            "last_ok": false,
            "last_error": null,
            "last_error_kind": null,
            "pending": 0
        }),
    };
    Ok(tool_success(status))
}

/// Describe the available attachment-upload methods so an agent can pick one:
/// the HTTP endpoint (the default — it now accepts the MCP token directly,
/// so no separate credential is needed) and the base64 MCP tool (the
/// fallback, for clients that cannot make an out-of-band HTTP request).
pub(super) fn get_attachment_import_config_tool(
    config: &McpConfig,
) -> Result<Value, JsonRpcFailure> {
    let enabled = config.write_enabled;
    let methods = if enabled {
        json!([
            {
                "id": "http_multipart",
                "role": "default",
                "method": "POST",
                "path": "/api/attachment",
                "path_note": "Relative path — resolve it against the same scheme, host, and port as this MCP endpoint.",
                "max_bytes": config.max_attachment_bytes,
                "recommended_for": "the default for any file size; use unless the client cannot make an out-of-band HTTP request",
                "auth": "Accepts either the web bearer token (HATCHDOOR_WEB_BEARER_TOKEN) or the MCP token as `Authorization: Bearer <token>` — an agent can reuse its existing MCP token here, no separate credential needed. No token is required when neither is configured.",
                "requires": "ability to make an HTTP request outside MCP (e.g. shell/curl)",
                "usage": "POST multipart/form-data with fields `target_relative_path` and `file`."
            },
            {
                "id": "mcp_base64",
                "tool": "import_attachment",
                "role": "fallback",
                "max_bytes": config.max_base64_bytes,
                "recommended_for": "fallback when an out-of-band HTTP request is not possible; universal, works with any MCP client, but size-limited",
                "usage": "Call import_attachment with base64-encoded `content` and a vault-relative `target_relative_path`."
            }
        ])
    } else {
        json!([])
    };
    Ok(tool_success(json!({
        "enabled": enabled,
        "allowed_extensions": allowed_attachment_extensions(),
        "methods": methods,
        "usage": if enabled {
            "Two upload methods are available. Prefer the HTTP endpoint (POST /api/attachment) by default; fall back to import_attachment (base64) only when an out-of-band HTTP request is not possible."
        } else {
            "Attachment upload is disabled. Set HATCHDOOR_MCP_WRITE_ENABLED to enable it."
        }
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

/// Tools whose queries accept a `layers` selector. Kept in one place so the
/// schema injection below and any future per-tool logic agree on the set.
const LAYER_AWARE_READ_TOOLS: [&str; 5] = [
    "search_notes",
    "query_notes",
    "get_note_links",
    "get_tree",
    "recently_modified",
];

/// The JSON-schema fragment for the `layers` array parameter, or `None` for a
/// vault with no layers (in which case the parameter is omitted entirely). The
/// enum is `default`/`all` plus every discovered layer name; each named layer's
/// marker description (already sanitized in phase 1) is folded into the
/// parameter description, since JSON Schema has no per-enum-value docs.
fn layers_param_schema(layers: &[crate::search::LayerInfo]) -> Option<Value> {
    if layers.is_empty() {
        return None;
    }
    let mut enum_values = vec![json!("default"), json!("all")];
    let mut described = Vec::new();
    for layer in layers {
        enum_values.push(json!(layer.name));
        match &layer.description {
            Some(description) => described.push(format!("'{}' — {}", layer.name, description)),
            None => described.push(format!("'{}'", layer.name)),
        }
    }
    Some(json!({
        "type": "array",
        "items": {"type": "string", "enum": enum_values},
        "default": [],
        "description": format!(
            "Which vault layers to include. Omit for the default surface only. \
             'default' adds the default surface; 'all' selects every layer. \
             Named demoted layers: {}.",
            described.join("; ")
        )
    }))
}

pub(super) fn read_tools_list(layers: &[crate::search::LayerInfo]) -> Vec<Value> {
    let mut tools = read_tools_list_base();
    if let Some(schema) = layers_param_schema(layers) {
        for tool in tools.iter_mut() {
            let is_layer_aware = tool
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| LAYER_AWARE_READ_TOOLS.contains(&name));
            if !is_layer_aware {
                continue;
            }
            if let Some(properties) = tool
                .get_mut("inputSchema")
                .and_then(|schema| schema.get_mut("properties"))
                .and_then(Value::as_object_mut)
            {
                properties.insert("layers".to_string(), schema.clone());
            }
        }
    }
    tools
}

fn read_tools_list_base() -> Vec<Value> {
    vec![
        json!({
            "name": "search_notes",
            "description": "Semantic-first chunk search across the vault. Optional metadata filters restrict eligible notes before results are returned. Results always include normalized tags and aliases; include_properties selects frontmatter fields to return. Use query_notes instead when metadata alone defines the request.",
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
                    },
                    "filters": note_filters_schema(),
                    "include_properties": {
                        "type": "array",
                        "items": {"type":"string"},
                        "maxItems": 50,
                        "default": [],
                        "description": "Frontmatter property names to include in each result."
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            },
            "annotations": read_only_tool_annotations()
        }),
        json!({
            "name": "query_notes",
            "description": "List notes using exact metadata filters without semantic or keyword retrieval. Use for requests such as all notes with a tag, property, status, or path prefix.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "filters": note_filters_schema(),
                    "include_properties": {
                        "type": "array",
                        "items": {"type":"string"},
                        "maxItems": 50,
                        "default": []
                    },
                    "limit": {
                        "type":"integer",
                        "minimum":1,
                        "maximum":200,
                        "default":50
                    }
                },
                "required": ["filters"],
                "additionalProperties": false
            },
            "annotations": read_only_tool_annotations()
        }),
        json!({
            "name": "get_note",
            "description": "Fetch full Markdown content for one note, addressed by exactly one of `slug` or `path` (a vault-relative path, with or without .md). Both reach any layer; the response carries the note's `layer`. Use only after search_notes or resolve_wikilink identifies the note; avoid fetching many full notes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "slug": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Hatchdoor note slug. Provide slug or path, not both."
                    },
                    "path": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Vault-relative path (e.g. 'sources/Clip.md'). Reaches demoted notes by a stable address. Provide slug or path, not both."
                    }
                },
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
            "name": "recently_modified",
            "description": "List the most recently modified notes, newest first. The agent's ingest-discovery path: use it to find notes changed since a checkpoint. Returns title, slug, relative_path, mtime_ns and layer.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 100,
                        "default": 20
                    }
                },
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
            "description": "Return the available attachment upload methods (base64 MCP tool and HTTP endpoint), their size limits, allowed extensions, and which to use. Call before uploading attachments.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            },
            "annotations": read_only_tool_annotations()
        }),
        json!({
            "name": "get_git_sync_status",
            "description": "Report local or remote versioning lifecycle state, the last attempt, failures, and pending writes. In remote mode it also reports commits not yet pushed; that field is absent for local history.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            },
            "annotations": read_only_tool_annotations()
        }),
        json!({
            "name": "layer_diagnostics",
            "description": "Explain the vault's layer and noise classification. Dumps the active noise-exclusion ruleset with provenance (built-in vs HATCHDOOR_EXCLUDE), the discovered layer markers, per-layer note counts, and any conflicts (a marker directory that is itself noise-excluded, a vanished marker whose notes are retained, disagreeing marker descriptions). Pass an optional `path` to classify an arbitrary path string by re-running the matchers, whether or not it is indexed.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "A vault-relative path to classify (noise? which layer?). Re-runs the matchers on the raw string; does not require the path to exist or be indexed."
                    }
                },
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
    #[serde(default)]
    filters: crate::search::NoteFilters,
    #[serde(default)]
    include_properties: Vec<String>,
    #[serde(default)]
    layers: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryNotesArgs {
    filters: crate::search::NoteFilters,
    #[serde(default)]
    include_properties: Vec<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    layers: Vec<String>,
}

/// get_note addresses a note by exactly one of `slug` or `path`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GetNoteArgs {
    #[serde(default)]
    slug: Option<String>,
    #[serde(default)]
    path: Option<String>,
}

/// A note slug plus an optional `layers` selector (get_note_links).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LinksArgs {
    slug: String,
    #[serde(default)]
    layers: Vec<String>,
}

/// recently_modified: a result cap plus an optional `layers` selector.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecentlyModifiedArgs {
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    layers: Vec<String>,
}

/// Only a `layers` selector (get_tree, recently_modified).
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct LayersOnlyArgs {
    #[serde(default)]
    layers: Vec<String>,
}

/// Reject a `path_prefix` that points wholly inside a demoted layer the current
/// selection does not include, with an error naming the layer and the parameter
/// to pass — never a silently empty result (spec "Addressing"/precedence).
fn check_path_prefix_precedence(
    cache: &crate::cache::SqliteCache,
    filters: &crate::search::NoteFilters,
    selection: &crate::search::LayerSelection,
) -> Result<(), JsonRpcFailure> {
    let Some(prefix) = filters.path_prefix.as_deref() else {
        return Ok(());
    };
    if prefix.trim().is_empty() || selection.is_all() {
        return Ok(());
    }
    let Some(layers) = cache
        .demoted_layers_under_prefix(prefix)
        .map_err(JsonRpcFailure::internal)?
    else {
        return Ok(());
    };
    // The prefix is wholly inside demoted space. If the selection already covers
    // one of those layers the query returns something, so let it through; only
    // when it covers none of them would the result be silently empty — error
    // instead, naming the layer(s) and how to include them.
    let selected = selection.named_layers();
    if layers.iter().any(|layer| selected.contains(layer)) {
        return Ok(());
    }
    let names = layers
        .iter()
        .map(|layer| format!("\"{layer}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let plural = if layers.len() == 1 { "layer" } else { "layers" };
    Err(JsonRpcFailure::invalid_params(format!(
        "path_prefix '{prefix}' is inside the demoted {plural} {names}, which is not selected. \
         Pass layers: [{names}] (or [\"all\"]) to include it."
    )))
}

/// Parse the MCP `layers` tokens against the vault's persisted layer catalog,
/// logging any degrade warnings. An unknown layer name is not a hard error: it
/// degrades to the default surface (see [`crate::search::LayerSelection::parse`]),
/// so a stale client holding a since-removed layer name keeps working.
fn parse_layer_selection(
    cache: &crate::cache::SqliteCache,
    tokens: &[String],
) -> Result<crate::search::LayerSelection, JsonRpcFailure> {
    let known: Vec<String> = cache
        .layer_catalog()
        .map_err(JsonRpcFailure::internal)?
        .into_iter()
        .map(|layer| layer.name)
        .collect();
    let (selection, warnings) = crate::search::LayerSelection::parse(tokens, &known);
    for warning in warnings {
        tracing::warn!(%warning, "MCP layers selector degraded to the default surface");
    }
    Ok(selection)
}

fn note_filters_schema() -> Value {
    json!({
        "type":"object",
        "properties": {
            "tags": {
                "type":"array",
                "items":{"type":"string"},
                "maxItems":50,
                "default":[],
                "description":"All listed tags must be present; matching is case-insensitive and ignores a leading #."
            },
            "tag_prefixes": {
                "type":"array",
                "items":{"type":"string"},
                "maxItems":50,
                "default":[],
                "description":"All listed tag paths must match exactly or as ancestors of a note tag; matching is case-insensitive and ignores a leading #."
            },
            "path_prefix": {
                "type":"string",
                "description":"Case-insensitive vault-relative path prefix."
            },
            "property_exists": {
                "type":"array",
                "items":{"type":"string"},
                "maxItems":50,
                "default":[]
            },
            "property_equals": {
                "type":"object",
                "additionalProperties": true,
                "description":"Exact typed frontmatter property matches."
            }
        },
        "additionalProperties":false
    })
}

fn validate_metadata_query(
    filters: &crate::search::NoteFilters,
    include_properties: &[String],
) -> Result<(), JsonRpcFailure> {
    const MAX_METADATA_TERMS: usize = 50;
    for (name, count) in [
        ("tags", filters.tags.len()),
        ("tag_prefixes", filters.tag_prefixes.len()),
        ("property_exists", filters.property_exists.len()),
        ("property_equals", filters.property_equals.len()),
        ("include_properties", include_properties.len()),
    ] {
        if count > MAX_METADATA_TERMS {
            return Err(JsonRpcFailure::invalid_params(format!(
                "{name} accepts at most {MAX_METADATA_TERMS} entries"
            )));
        }
    }
    let names = filters
        .tags
        .iter()
        .chain(filters.tag_prefixes.iter())
        .chain(filters.property_exists.iter())
        .chain(filters.property_equals.keys())
        .chain(include_properties.iter());
    if names.into_iter().any(|value| value.trim().is_empty()) {
        return Err(JsonRpcFailure::invalid_params(
            "metadata filter names cannot be empty",
        ));
    }
    if filters
        .path_prefix
        .as_deref()
        .is_some_and(|prefix| prefix.len() > 4_096)
    {
        return Err(JsonRpcFailure::invalid_params(
            "path_prefix cannot exceed 4096 bytes",
        ));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolveWikilinkArgs {
    target: String,
}
