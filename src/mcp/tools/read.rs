//! Vault-scoped MCP read tools, plus the eight Vault collection management
//! tools. These are deliberately thin in-process adapters over the same
//! shared cores used by HTTP: MCP owns JSON-RPC framing, while scope parsing,
//! projections, and error shapes stay in the core.
//!
//! The read tools still proxy their V1 handler and decode its payload back
//! into the declared result type. The management tools do not: since #187
//! `list_vaults`, `create_vault`, `edit_vault`, `enable_vault`,
//! `disable_vault`, `disconnect_vault`, `sync_vault`, and `retry_vault` call
//! `vault_management::VaultCollectionManagement` directly and shape its typed
//! response or structured error themselves, rather than building axum
//! extractors around a handler and decoding an HTTP response body.

use axum::body::to_bytes;
use axum::extract::{Path, Query, State};
use axum::response::Response;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

use std::str::FromStr;

use crate::app_state::AppState;
use crate::handlers::{vault_collection_reads, vault_content};
use crate::vault::allowed_attachment_extensions;
use crate::vault_management::{
    CreateVaultRequest, EditVaultRequest, HttpsCredentialsPatch, VaultCollectionManagement,
};
use crate::vault_read::VaultReadCore;
use crate::vault_registry::VaultId;

use super::super::config::McpConfig;
use super::super::protocol::{JsonRpcFailure, tool_structured_error, tool_success};
use super::read_only_tool_annotations;

const MAX_TOOL_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

/// Decodes a proxied V1 handler's success body into its typed MCP result and
/// serializes the result back out. The round-trip is the point: tool
/// responses are produced from exactly the structures whose schemas
/// `tools/list` advertises (single source of truth), and a handler payload
/// that no longer fits its declared result type fails loudly here instead of
/// silently drifting from the advertised contract.
pub(super) async fn handler_payload<T>(response: Response) -> Result<Value, JsonRpcFailure>
where
    T: DeserializeOwned + Serialize,
{
    let status = response.status();
    let bytes = to_bytes(response.into_body(), MAX_TOOL_RESPONSE_BYTES)
        .await
        .map_err(|error| JsonRpcFailure::internal(format!("read Vault response body: {error}")))?;
    if !status.is_success() {
        // Error bodies stay untyped passthroughs of the shared Vault API's
        // stable error object so agents can branch on `code`.
        let payload = serde_json::from_slice::<Value>(&bytes).map_err(|error| {
            JsonRpcFailure::internal(format!("decode Vault response body: {error}"))
        })?;
        return Ok(tool_structured_error(payload));
    }
    let result = serde_json::from_slice::<T>(&bytes).map_err(|error| {
        JsonRpcFailure::internal(format!(
            "tool result does not match its advertised schema: {error}"
        ))
    })?;
    Ok(tool_success(crate::mcp::results::result_to_value(&result)))
}

fn parse<T: for<'de> Deserialize<'de>>(tool: &str, arguments: Value) -> Result<T, JsonRpcFailure> {
    serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid {tool} arguments: {error}"))
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScopeArgs {
    scope: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchArgs {
    scope: String,
    query: String,
    #[serde(default)]
    mode: Option<crate::search::SearchMode>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    per_note_cap: Option<usize>,
    #[serde(default)]
    layers: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecentArgs {
    scope: String,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExactSlugArgs {
    vault_id: String,
    slug: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolveArgs {
    vault_id: String,
    target: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct VaultControlArgs {
    vault_id: String,
    expected_registry_revision: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct VaultIdArgs {
    vault_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EditVaultArgs {
    vault_id: String,
    expected_registry_revision: u64,
    name: String,
    source: crate::vault_registry::VaultSource,
    #[serde(default)]
    exclude_patterns: Vec<String>,
    #[serde(default)]
    https_credentials: Option<HttpsCredentialsPatch>,
    #[serde(default)]
    confirm_identity_change: bool,
    #[serde(default)]
    archive_folder: Option<String>,
    #[serde(default)]
    commit_identity: Option<crate::vault_registry::VaultCommitIdentity>,
}

pub(super) async fn search_notes_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: SearchArgs = parse("search_notes", arguments)?;
    let query = vault_collection_reads::VaultScopeSearchQuery {
        q: args.query,
        mode: args.mode,
        limit: args.limit,
        per_note_cap: args.per_note_cap,
        layers: (!args.layers.is_empty()).then(|| args.layers.join(",")),
    };
    handler_payload::<crate::mcp::results::SearchNotesResult>(
        vault_collection_reads::vault_scope_search_handler(
            State(state),
            Path(args.scope),
            Ok(Query(query)),
        )
        .await,
    )
    .await
}

pub(super) async fn get_note_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: ExactSlugArgs = parse("get_note", arguments)?;
    handler_payload::<crate::mcp::results::GetNoteResult>(
        vault_content::vault_scoped_note_handler(State(state), Path((args.vault_id, args.slug)))
            .await,
    )
    .await
}

pub(super) async fn get_note_links_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: ExactSlugArgs = parse("get_note_links", arguments)?;
    handler_payload::<crate::mcp::results::GetNoteLinksResult>(
        vault_content::vault_scoped_note_links_handler(
            State(state),
            Path((args.vault_id, args.slug)),
        )
        .await,
    )
    .await
}

pub(super) async fn resolve_wikilink_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: ResolveArgs = parse("resolve_wikilink", arguments)?;
    handler_payload::<crate::mcp::results::ResolveWikilinkResult>(
        vault_content::vault_scoped_resolve_handler(
            State(state),
            Path(args.vault_id),
            Ok(Query(crate::api_types::ResolveQuery {
                target: args.target,
            })),
        )
        .await,
    )
    .await
}

pub(super) async fn get_tree_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: ScopeArgs = parse("get_tree", arguments)?;
    handler_payload::<crate::mcp::results::GetTreeResult>(
        vault_collection_reads::vault_scope_tree_handler(State(state), Path(args.scope)).await,
    )
    .await
}

pub(super) async fn get_stats_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: ScopeArgs = parse("get_stats", arguments)?;
    handler_payload::<crate::mcp::results::GetStatsResult>(
        vault_collection_reads::vault_scope_stats_handler(State(state), Path(args.scope)).await,
    )
    .await
}

pub(super) async fn get_graph_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: ScopeArgs = parse("get_graph", arguments)?;
    handler_payload::<crate::mcp::results::GetGraphResult>(
        vault_collection_reads::vault_scope_graph_handler(State(state), Path(args.scope)).await,
    )
    .await
}

pub(super) async fn recently_modified_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: RecentArgs = parse("recently_modified", arguments)?;
    handler_payload::<crate::mcp::results::RecentlyModifiedResult>(
        vault_collection_reads::vault_scope_recent_handler(
            State(state),
            Path(args.scope),
            Ok(Query(crate::api_types::RecentlyModifiedQuery {
                limit: args.limit,
            })),
        )
        .await,
    )
    .await
}

/// Report how an agent may upload an attachment into one Vault.  Advertised
/// and answerable whatever the write posture is: the answer an agent needs
/// when uploads are unavailable ("not here, and why") is as useful as the
/// methods themselves, so this reports capability rather than refusing.
///
/// Two independent gates decide `enabled`, and they fail for different
/// reasons an agent should not conflate: `HATCHDOOR_MCP_WRITE_ENABLED` is
/// instance-wide and operator-owned, while `capabilities.mutate` belongs to
/// this Vault's own source mode and lifecycle phase (a pull-only or
/// not-yet-Ready Vault refuses writes on an instance where write mode is on).
pub(super) fn attachment_import_config_tool(
    state: &AppState,
    config: &McpConfig,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: VaultIdArgs = parse("get_attachment_import_config", arguments)?;
    let vault_id = VaultId::from_str(&args.vault_id)
        .map_err(|_| JsonRpcFailure::invalid_params("vault_id must be a canonical Vault ID"))?;
    let control = VaultReadCore::new(&state.startup_sqlite, &state.vaults)
        .control_block(vault_id)
        .map_err(|error| {
            JsonRpcFailure::not_found(serde_json::to_string(&error).unwrap_or(error.message))
        })?;
    let vault_mutable = control.snapshot().capabilities.mutate;
    let enabled = config.write_enabled && vault_mutable;

    let methods: Vec<crate::mcp::results::AttachmentImportMethod> = if enabled {
        vec![
            crate::mcp::results::AttachmentImportMethod::HttpMultipart {
                role: "default",
                method: "POST",
                path: format!("/api/v1/vaults/{vault_id}/attachments"),
                path_note: "Relative path — resolve it against the same scheme, host, and port as this MCP endpoint.",
                max_bytes: config.max_attachment_bytes,
                recommended_for: "the default for any file size; use unless the client cannot make an out-of-band HTTP request",
                auth: "Send `Authorization: Bearer <token>` with either the web bearer token (HATCHDOOR_WEB_BEARER_TOKEN) or this session's MCP token. The MCP token is accepted only while MCP and MCP write mode are both currently enabled, checked per request: if an operator disables either one, this credential loses upload access immediately even though the same token still reads. No token is required when neither is configured.",
                requires: "ability to make an HTTP request outside MCP (e.g. shell/curl)",
                usage: "POST multipart/form-data with fields `target_relative_path` and `file`.",
            },
            crate::mcp::results::AttachmentImportMethod::McpBase64 {
                tool: "import_attachment",
                role: "fallback",
                max_bytes: config.max_base64_bytes,
                recommended_for: "fallback when an out-of-band HTTP request is not possible; universal, works with any MCP client, but size-limited",
                usage: "Call import_attachment with this vault_id, base64-encoded `content`, and a Vault-relative `target_relative_path`.",
            },
        ]
    } else {
        Vec::new()
    };

    let usage = if enabled {
        "Two upload methods are available for this Vault. Prefer the HTTP endpoint by default; fall back to import_attachment (base64) only when an out-of-band HTTP request is not possible."
    } else if !config.write_enabled {
        "Attachment upload is disabled for this instance. An operator must set HATCHDOOR_MCP_WRITE_ENABLED; no other Vault will accept uploads either until they do."
    } else {
        "Attachment upload is unavailable for this Vault's current source mode and lifecycle phase, though MCP write mode is enabled. Read this Vault's status and capabilities from list_vaults; another Vault may still accept uploads."
    };

    Ok(tool_success(crate::mcp::results::result_to_value(
        &crate::mcp::results::AttachmentImportConfigResult {
            vault_id: vault_id.to_string(),
            enabled,
            write_mode_enabled: config.write_enabled,
            vault_accepts_mutation: vault_mutable,
            allowed_extensions: allowed_attachment_extensions()
                .iter()
                .map(|extension| extension.to_string())
                .collect(),
            methods,
            usage: usage.to_string(),
        },
    )))
}

/// The MCP half of the mapping for Vault collection management: one
/// structured core error becomes a tool error carrying that same
/// `{code, message, vault_id?, retryable}` payload, byte-identical to the
/// body these tools used to decode back out of a proxied HTTP response.
/// Instance-side detail is already sanitized by the core, so both surfaces
/// report the same message.
fn management_error(error: crate::vault_error::VaultOperationError) -> Value {
    tool_structured_error(
        serde_json::to_value(&error).unwrap_or_else(|_| json!({ "code": error.code })),
    )
}

/// The success half: the core's typed response, serialized through exactly
/// the structure whose schema `tools/list` advertises.
fn management_result<T: Serialize>(
    result: Result<T, crate::vault_error::VaultOperationError>,
) -> Result<Value, JsonRpcFailure> {
    Ok(match result {
        Ok(response) => tool_success(crate::mcp::results::result_to_value(&response)),
        Err(error) => management_error(error),
    })
}

/// The Vault ID every control of an existing Vault carries, parsed by the
/// same core function the HTTP adapter uses so a malformed ID is refused
/// identically on both surfaces. Its refusal is an ordinary structured
/// management error, so it flows through `management_result` with every other
/// outcome rather than returning early on its own path.
fn management_vault_id(raw: &str) -> Result<VaultId, crate::vault_error::VaultOperationError> {
    crate::vault_management::parse_vault_id(raw)
}

pub(super) async fn list_vaults_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let _: EmptyArgs = parse("list_vaults", arguments)?;
    management_result::<crate::mcp::results::ListVaultsResult>(
        VaultCollectionManagement::new(&state).list(),
    )
}

/// Registry writes go straight to the Vault collection management core, the
/// same one the HTTP routes call. The create operation is the sole control
/// without a `vault_id`: the shared registry generates the immutable ID
/// atomically when the expected registry revision commits; every control of
/// an existing Vault takes exactly one `vault_id`.
pub(super) async fn create_vault_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let request: CreateVaultRequest = parse("create_vault", arguments)?;
    management_result::<crate::mcp::results::CreateVaultResult>(
        VaultCollectionManagement::new(&state).create(request).await,
    )
}

pub(super) async fn edit_vault_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: EditVaultArgs = parse("edit_vault", arguments)?;
    let request = EditVaultRequest {
        expected_registry_revision: args.expected_registry_revision,
        name: args.name,
        source: args.source,
        exclude_patterns: args.exclude_patterns,
        https_credentials: args
            .https_credentials
            .unwrap_or(HttpsCredentialsPatch::Keep),
        confirm_identity_change: args.confirm_identity_change,
        archive_folder: args.archive_folder,
        commit_identity: args.commit_identity,
    };
    let core = VaultCollectionManagement::new(&state);
    let result = match management_vault_id(&args.vault_id) {
        Ok(vault_id) => core.edit(vault_id, request).await,
        Err(error) => Err(error),
    };
    management_result::<crate::mcp::results::EditVaultResult>(result)
}

pub(super) async fn enable_vault_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: VaultControlArgs = parse("enable_vault", arguments)?;
    let core = VaultCollectionManagement::new(&state);
    let result = match management_vault_id(&args.vault_id) {
        Ok(vault_id) => {
            core.set_enabled(vault_id, args.expected_registry_revision, true)
                .await
        }
        Err(error) => Err(error),
    };
    management_result::<crate::mcp::results::EnableVaultResult>(result)
}

pub(super) async fn disable_vault_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: VaultControlArgs = parse("disable_vault", arguments)?;
    let core = VaultCollectionManagement::new(&state);
    let result = match management_vault_id(&args.vault_id) {
        Ok(vault_id) => {
            core.set_enabled(vault_id, args.expected_registry_revision, false)
                .await
        }
        Err(error) => Err(error),
    };
    management_result::<crate::mcp::results::DisableVaultResult>(result)
}

pub(super) async fn disconnect_vault_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: VaultControlArgs = parse("disconnect_vault", arguments)?;
    let core = VaultCollectionManagement::new(&state);
    let result = match management_vault_id(&args.vault_id) {
        Ok(vault_id) => {
            core.disconnect(vault_id, args.expected_registry_revision)
                .await
        }
        Err(error) => Err(error),
    };
    management_result::<crate::mcp::results::DisconnectVaultResult>(result)
}

pub(super) async fn sync_vault_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: VaultIdArgs = parse("sync_vault", arguments)?;
    let core = VaultCollectionManagement::new(&state);
    management_result::<crate::mcp::results::SyncVaultResult>(
        management_vault_id(&args.vault_id).and_then(|vault_id| core.sync(vault_id)),
    )
}

pub(super) async fn retry_vault_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: VaultIdArgs = parse("retry_vault", arguments)?;
    let core = VaultCollectionManagement::new(&state);
    management_result::<crate::mcp::results::RetryVaultResult>(
        management_vault_id(&args.vault_id).and_then(|vault_id| core.retry(vault_id)),
    )
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyArgs {}

fn scope_schema() -> Value {
    json!({"type":"string", "minLength":1, "description":"A canonical Vault ID or the literal all."})
}

fn vault_id_schema() -> Value {
    json!({"type":"string", "minLength":1, "description":"A canonical Vault ID. The literal all is invalid."})
}

/// The persisted `VaultSource` contract, spelled out rather than described in
/// prose.  `vault_registry::VaultSource` is internally tagged on `type` and
/// carries `deny_unknown_fields`, so an agent that guesses a field name gets a
/// hard rejection with nothing to correct against.  `edit_vault` callers can
/// copy the `source` object straight back out of `list_vaults`; a first
/// `create_vault` has no such prior to copy, which is why the per-variant
/// constraints enforced by `vault_registry`'s normalization (which `mode` each
/// source accepts, the poll-interval floor, HTTPS-only URLs) are stated here
/// rather than discovered by rejection.
fn vault_source_schema() -> Value {
    let branch = json!({
        "type": ["string", "null"],
        "description": "Branch to track. Null or absent tracks the remote's default branch."
    });
    let vault_subdirectory = json!({
        "type": ["string", "null"],
        "description": "Vault root relative to the repository root. Null or absent uses the repository root itself."
    });
    let poll_interval_secs = json!({
        "type": "integer",
        "minimum": 60,
        "default": 86400,
        "description": "How often Hatchdoor polls the remote, in seconds, absent a manual sync_vault or retry_vault. Minimum 60. Ignored for local_history, which has no remote."
    });
    json!({
        "description": "Where this Vault's Markdown lives and how Hatchdoor versions it. Exactly one of the three shapes below, chosen by the type tag. Unknown fields are rejected.",
        "oneOf": [
            {
                "title": "local",
                "description": "A plain directory on this machine. Hatchdoor never runs Git for it.",
                "type": "object",
                "properties": {
                    "type": {"const": "local"},
                    "path": {"type": "string", "minLength": 1, "description": "Absolute path to the Vault directory."}
                },
                "required": ["type", "path"],
                "additionalProperties": false
            },
            {
                "title": "existing_git",
                "description": "A Git working copy that already exists on this machine. Hatchdoor uses it in place and never clones it.",
                "type": "object",
                "properties": {
                    "type": {"const": "existing_git"},
                    "repository_path": {"type": "string", "minLength": 1, "description": "Absolute path to the existing repository on this machine."},
                    "repository_url": {
                        "type": ["string", "null"],
                        "description": "Credential-free HTTPS remote URL. Required for pull_only and two_way; may be null only for local_history."
                    },
                    "branch": branch,
                    "vault_subdirectory": vault_subdirectory,
                    "mode": {
                        "type": "string",
                        "enum": ["local_history", "pull_only", "two_way"],
                        "description": "local_history commits locally and never contacts a remote; pull_only also fetches; two_way also pushes."
                    },
                    "poll_interval_secs": poll_interval_secs
                },
                "required": ["type", "repository_path", "mode"],
                "additionalProperties": false
            },
            {
                "title": "managed_git",
                "description": "A remote repository Hatchdoor clones and owns the checkout of. There is no local_history mode: a managed Vault exists to track a remote.",
                "type": "object",
                "properties": {
                    "type": {"const": "managed_git"},
                    "repository_url": {"type": "string", "minLength": 1, "description": "Credential-free HTTPS remote URL. Embedded credentials are rejected; supply a token through https_credentials instead."},
                    "branch": branch,
                    "vault_subdirectory": vault_subdirectory,
                    "mode": {
                        "type": "string",
                        "enum": ["pull_only", "two_way"],
                        "description": "pull_only fetches only; two_way also pushes Hatchdoor's commits."
                    },
                    "poll_interval_secs": poll_interval_secs
                },
                "required": ["type", "repository_url", "mode"],
                "additionalProperties": false
            }
        ]
    })
}

/// The credential a Vault presents to an HTTPS remote, on create.  `username`
/// is optional because token providers accept any non-empty username; the
/// registry substitutes a fixed placeholder when it is omitted.
fn https_credentials_input_schema() -> Value {
    json!({
        "type": "object",
        "description": "Optional HTTPS token for the remote. Write-only: it is redacted from every response, and list_vaults reports only whether a credential is configured.",
        "properties": {
            "username": {
                "type": ["string", "null"],
                "description": format!("Optional. Omitted or null uses the fixed placeholder {}, which token providers accept.", crate::vault_registry::HTTPS_CREDENTIALS_USERNAME_PLACEHOLDER)
            },
            "token": {"type": "string", "minLength": 1}
        },
        "required": ["token"],
        "additionalProperties": false
    })
}

/// The three-state credential update, on edit.  The tag exists so "leave the
/// stored credential alone" stays distinguishable from "clear it" without
/// relying on JSON null-versus-absent.
fn https_credentials_patch_schema() -> Value {
    json!({
        "description": "What to do with this Vault's stored HTTPS credential. Absent means keep. The replacement token is never echoed back.",
        "oneOf": [
            {
                "title": "keep",
                "description": "Leave the stored credential exactly as it is.",
                "type": "object",
                "properties": {"action": {"const": "keep"}},
                "required": ["action"],
                "additionalProperties": false
            },
            {
                "title": "remove",
                "description": "Delete the stored credential. A remote needing authentication will then fail to sync.",
                "type": "object",
                "properties": {"action": {"const": "remove"}},
                "required": ["action"],
                "additionalProperties": false
            },
            {
                "title": "replace",
                "type": "object",
                "properties": {
                    "action": {"const": "replace"},
                    "username": {
                        "type": ["string", "null"],
                        "description": format!("Optional. Omitted or null uses the fixed placeholder {}.", crate::vault_registry::HTTPS_CREDENTIALS_USERNAME_PLACEHOLDER)
                    },
                    "token": {"type": "string", "minLength": 1}
                },
                "required": ["action", "token"],
                "additionalProperties": false
            }
        ]
    })
}

/// Per-Vault overrides of instance-wide defaults.  Absent means "inherit",
/// which is a different statement from any particular value, so both tools
/// describe them the same way.
fn archive_folder_schema() -> Value {
    json!({
        "type": "string",
        "description": "Optional per-Vault archive folder for archive_note. Absent means the instance-wide default applies."
    })
}

fn commit_identity_schema() -> Value {
    json!({
        "type": "object",
        "description": "Optional per-Vault commit author identity. Absent means the instance-wide default applies.",
        "properties": {
            "name": {"type": "string", "minLength": 1},
            "email": {"type": "string", "minLength": 1}
        },
        "required": ["name", "email"],
        "additionalProperties": false
    })
}

fn exclude_patterns_schema() -> Value {
    json!({
        "type": "array",
        "items": {"type": "string"},
        "default": [],
        "description": "Glob patterns for paths this Vault's index ignores. Replaces the stored list wholesale; an empty array clears it."
    })
}

pub(super) fn read_tools_list() -> Vec<Value> {
    vec![
        json!({"name":"list_vaults", "description":"Discover the Vault collection, its registry and collection revisions, redacted credentials, status, and capabilities before calling any Vault-dependent tool.", "inputSchema":{"type":"object","properties":{},"additionalProperties":false},"annotations":read_only_tool_annotations()}),
        json!({"name":"search_notes", "description":"Search one Vault or all enabled Vaults. Results use the shared partial-participant envelope and every hit is Vault-qualified.", "inputSchema":{"type":"object","properties":{"scope":scope_schema(),"query":{"type":"string","minLength":1},"mode":{"type":"string","enum":["semantic","keyword"],"default":"semantic"},"limit":{"type":"integer","minimum":1,"maximum":50,"default":10},"per_note_cap":{"type":"integer","minimum":1,"maximum":10,"default":2},"layers":{"type":"array","items":{"type":"string"},"default":[]}},"required":["scope","query"],"additionalProperties":false},"annotations":read_only_tool_annotations()}),
        json!({"name":"get_note", "description":"Read one exact Note from its authoritative Vault Markdown directory.", "inputSchema":{"type":"object","properties":{"vault_id":vault_id_schema(),"slug":{"type":"string","minLength":1}},"required":["vault_id","slug"],"additionalProperties":false},"annotations":read_only_tool_annotations()}),
        json!({"name":"get_note_links", "description":"Read outgoing links and backlinks for one exact Vault Note.", "inputSchema":{"type":"object","properties":{"vault_id":vault_id_schema(),"slug":{"type":"string","minLength":1}},"required":["vault_id","slug"],"additionalProperties":false},"annotations":read_only_tool_annotations()}),
        json!({"name":"resolve_wikilink", "description":"Resolve a wikilink target within exactly one Vault.", "inputSchema":{"type":"object","properties":{"vault_id":vault_id_schema(),"target":{"type":"string","minLength":1}},"required":["vault_id","target"],"additionalProperties":false},"annotations":read_only_tool_annotations()}),
        collection_tool(
            "get_tree",
            "Return grouped explorer trees for one Vault or all enabled Vaults.",
        ),
        collection_tool(
            "get_stats",
            "Return grouped statistics for one Vault or all enabled Vaults.",
        ),
        collection_tool(
            "get_graph",
            "Return grouped graphs for one Vault or all enabled Vaults.",
        ),
        json!({"name":"get_frontmatter", "description":"Read one exact Note's frontmatter metadata — tags, aliases, and properties — from its authoritative Vault Markdown directory, without returning the Markdown body. A note without a frontmatter block returns an empty/default projection rather than an error.", "inputSchema":{"type":"object","properties":{"vault_id":vault_id_schema(),"slug":{"type":"string","minLength":1}},"required":["vault_id","slug"],"additionalProperties":false},"annotations":read_only_tool_annotations()}),
        json!({"name":"list_note_attachments", "description":"List the existing attachments one Note references, without returning the Note's full content.", "inputSchema":{"type":"object","properties":{"vault_id":vault_id_schema(),"slug":{"type":"string","minLength":1}},"required":["vault_id","slug"],"additionalProperties":false},"annotations":read_only_tool_annotations()}),
        json!({"name":"get_attachment", "description":"Fetch one attachment's bytes, addressed by relative_path exactly as list_note_attachments reports it. encoding \"url\" (the default) returns an HTTP download_url resolved against this MCP endpoint's scheme, host, and port; encoding \"base64\" returns inline base64 content instead, for a client that cannot make an out-of-band HTTP request or cannot obtain the download URL's own credential, bounded by the same size limit as import_attachment's base64 path.", "inputSchema":{"type":"object","properties":{"vault_id":vault_id_schema(),"relative_path":{"type":"string","minLength":1},"encoding":{"type":"string","enum":["url","base64"],"default":"url"}},"required":["vault_id","relative_path"],"additionalProperties":false},"annotations":read_only_tool_annotations()}),
        json!({"name":"get_attachment_import_config", "description":"Report how to upload an attachment into one Vault: the available methods (the HTTP endpoint and the base64 import_attachment tool), their size limits in bytes, the allowed file extensions, and whether uploads are currently possible at all. Call before uploading an attachment to that Vault.", "inputSchema":{"type":"object","properties":{"vault_id":vault_id_schema()},"required":["vault_id"],"additionalProperties":false},"annotations":read_only_tool_annotations()}),
        json!({"name":"recently_modified", "description":"List recently modified Notes for one Vault or all enabled Vaults.", "inputSchema":{"type":"object","properties":{"scope":scope_schema(),"limit":{"type":"integer","minimum":1,"maximum":25,"default":5}},"required":["scope"],"additionalProperties":false},"annotations":read_only_tool_annotations()}),
    ]
}

pub(super) fn management_tools_list() -> Vec<Value> {
    let vault_control = |name: &str, description: &str| {
        json!({
            "name": name,
            "description": description,
            "inputSchema": {"type":"object","properties":{
                "vault_id": vault_id_schema(),
                "expected_registry_revision":{"type":"integer","minimum":0}
            },"required":["vault_id","expected_registry_revision"],"additionalProperties":false},
            "annotations": super::write_tool_annotations(true, false)
        })
    };
    vec![
        json!({
            "name": "create_vault",
            "description": "Create a Vault definition with the shared revisioned collection contract. Read expected_registry_revision from list_vaults first; the create is rejected if the registry moved since. The registry assigns the immutable Vault ID after a successful create; use list_vaults to discover it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "expected_registry_revision": {"type":"integer","minimum":0,"description":"The registry_revision most recently read from list_vaults. A mismatch rejects the create rather than racing another writer."},
                    "name": {"type":"string","minLength":1},
                    "enabled": {"type":"boolean","default":true},
                    "source": vault_source_schema(),
                    "exclude_patterns": exclude_patterns_schema(),
                    "https_credentials": https_credentials_input_schema(),
                    "archive_folder": archive_folder_schema(),
                    "commit_identity": commit_identity_schema()
                },
                "required": ["expected_registry_revision","name","source"],
                "additionalProperties": false
            },
            "annotations": super::write_tool_annotations(true, false)
        }),
        json!({
            "name": "edit_vault",
            "description": "Edit exactly one Vault definition with optimistic registry revision control. This replaces the definition wholesale rather than patching named fields: read the Vault from list_vaults, change what you mean to change, and send the rest back unchanged. name and source are required for that reason, and an omitted exclude_patterns, archive_folder, or commit_identity clears the stored value rather than preserving it. The one exception is https_credentials, whose explicit keep|remove|replace action exists so a secret never has to be resent to survive an edit.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "vault_id": vault_id_schema(),
                    "expected_registry_revision": {"type":"integer","minimum":0,"description":"The registry_revision most recently read from list_vaults. A mismatch rejects the edit rather than overwriting a concurrent change."},
                    "name": {"type":"string","minLength":1},
                    "source": vault_source_schema(),
                    "exclude_patterns": exclude_patterns_schema(),
                    "https_credentials": https_credentials_patch_schema(),
                    "confirm_identity_change": {
                        "type": "boolean",
                        "default": false,
                        "description": "Consent to an edit that repoints this Vault at different content: its path, repository URL, branch, or subdirectory. Changing any of those makes the indexed notes a different set, so the edit is refused until this is true, and the Vault must be disabled first (disable_vault) — an identity change on an enabled Vault is refused whatever this says. Changing mode, poll_interval_secs, name, credentials, exclusions, archive folder, or commit identity is not an identity change and needs none of this. Leave it false for ordinary edits so an accidental repoint is caught rather than applied."
                    },
                    "archive_folder": archive_folder_schema(),
                    "commit_identity": commit_identity_schema()
                },
                "required": ["vault_id","expected_registry_revision","name","source"],
                "additionalProperties": false
            },
            "annotations": super::write_tool_annotations(true, false)
        }),
        vault_control("enable_vault", "Enable exactly one Vault definition."),
        vault_control(
            "disable_vault",
            "Disable exactly one Vault definition without deleting its files.",
        ),
        vault_control(
            "disconnect_vault",
            "Disconnect exactly one Vault definition without deleting local files, checkouts, Git history, or credentials outside its registry record.",
        ),
        json!({"name":"sync_vault","description":"Request immediate managed-Git synchronization for exactly one eligible Vault.","inputSchema":{"type":"object","properties":{"vault_id":vault_id_schema()},"required":["vault_id"],"additionalProperties":false},"annotations":super::write_tool_annotations(false, true)}),
        json!({"name":"retry_vault","description":"Retry an admitted managed-Git operation for exactly one eligible Vault.","inputSchema":{"type":"object","properties":{"vault_id":vault_id_schema()},"required":["vault_id"],"additionalProperties":false},"annotations":super::write_tool_annotations(false, true)}),
    ]
}

fn collection_tool(name: &str, description: &str) -> Value {
    json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":{"scope":scope_schema()},"required":["scope"],"additionalProperties":false},"annotations":read_only_tool_annotations()})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_catalogue_requires_explicit_scope_and_retires_legacy_tools() {
        let tools = read_tools_list();
        let named = |name: &str| {
            tools
                .iter()
                .find(|tool| tool["name"] == name)
                .unwrap_or_else(|| panic!("{name} is advertised"))
        };
        for name in [
            "search_notes",
            "get_tree",
            "get_stats",
            "get_graph",
            "recently_modified",
        ] {
            let tool = named(name);
            assert!(
                tool["inputSchema"]["required"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("scope"))
            );
        }
        for name in [
            "get_note",
            "get_note_links",
            "resolve_wikilink",
            "get_attachment",
            "get_attachment_import_config",
        ] {
            let tool = named(name);
            assert!(
                tool["inputSchema"]["required"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("vault_id"))
            );
        }
        for retired in ["refresh_index", "layer_diagnostics", "get_git_sync_status"] {
            assert!(!tools.iter().any(|tool| tool["name"] == retired));
        }
    }

    #[test]
    fn management_catalogue_uses_revisioned_shared_contracts() {
        let tools = management_tools_list();
        for name in [
            "edit_vault",
            "enable_vault",
            "disable_vault",
            "disconnect_vault",
            "sync_vault",
            "retry_vault",
        ] {
            let tool = tools
                .iter()
                .find(|tool| tool["name"] == name)
                .expect("tool");
            assert!(
                tool["inputSchema"]["required"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("vault_id")),
                "{name}"
            );
        }
        let create = tools
            .iter()
            .find(|tool| tool["name"] == "create_vault")
            .expect("create");
        assert!(
            create["inputSchema"]["required"]
                .as_array()
                .unwrap()
                .contains(&json!("expected_registry_revision"))
        );
        assert!(
            !create["inputSchema"]["required"]
                .as_array()
                .unwrap()
                .contains(&json!("vault_id")),
            "the shared registry assigns IDs only after a successful create"
        );
    }

    /// Issue #132's last acceptance criterion, `edit_vault` half: `source`'s
    /// `inputSchema` carries no nested `additionalProperties: false` (only
    /// the outer args object does — see `management_tools_list`'s literal
    /// JSON above), and `EditVaultArgs.source` deserializes straight into
    /// the real `vault_registry::VaultSource`, which already accepts
    /// `poll_interval_secs` on an `existing_git` source. This proves the
    /// Rust-level parse actually succeeds, not just that the schema doesn't
    /// forbid it.
    #[test]
    fn edit_vault_args_accept_the_schedule_on_an_existing_git_source() {
        let value = json!({
            "vault_id": "018f47a0-7768-4d0c-8da3-5aa28d1c31c7",
            "expected_registry_revision": 0,
            "name": "Existing checkout",
            "source": {
                "type": "existing_git",
                "repository_path": "/tmp/hatchdoor-mcp-test-repo",
                "repository_url": "https://example.test/notes.git",
                "branch": null,
                "vault_subdirectory": null,
                "mode": "pull_only",
                "poll_interval_secs": 60
            },
            "exclude_patterns": []
        });
        let args: EditVaultArgs = serde_json::from_value(value)
            .expect("edit_vault must accept poll_interval_secs on an existing_git source");
        assert!(matches!(
            args.source,
            crate::vault_registry::VaultSource::ExistingGit {
                poll_interval_secs: 60,
                ..
            }
        ));
    }

    /// Same acceptance criterion, `create_vault` half: `create_vault_tool`
    /// parses straight into the collection core's `CreateVaultRequest`, which embeds the
    /// same `VaultSource`.
    #[test]
    fn create_vault_request_accepts_the_schedule_on_an_existing_git_source() {
        let value = json!({
            "expected_registry_revision": 0,
            "name": "Existing checkout",
            "source": {
                "type": "existing_git",
                "repository_path": "/tmp/hatchdoor-mcp-test-repo",
                "repository_url": "https://example.test/notes.git",
                "branch": null,
                "vault_subdirectory": null,
                "mode": "pull_only",
                "poll_interval_secs": 60
            }
        });
        let request: CreateVaultRequest = serde_json::from_value(value)
            .expect("create_vault must accept poll_interval_secs on an existing_git source");
        assert!(matches!(
            request.source,
            crate::vault_registry::VaultSource::ExistingGit {
                poll_interval_secs: 60,
                ..
            }
        ));
    }

    /// The advertised `source` schema is the only thing a first `create_vault`
    /// has to go on: there is no stored definition to copy from yet, and
    /// `VaultSource` carries `deny_unknown_fields`, so a schema that overstates
    /// or understates what is required sends an agent into rejections it
    /// cannot correct. Each variant's declared `required` must therefore be
    /// exactly what the deserializer accepts as a minimal source.
    #[test]
    fn advertised_vault_source_variants_match_the_deserializer() {
        use crate::vault_registry::VaultSource;

        let schema = vault_source_schema();
        let variants = schema["oneOf"].as_array().expect("oneOf variants");
        let minimal = [
            (
                "local",
                json!({"type":"local","path":"/tmp/hatchdoor-source-schema"}),
            ),
            (
                "existing_git",
                json!({"type":"existing_git","repository_path":"/tmp/hatchdoor-source-schema","mode":"local_history"}),
            ),
            (
                "managed_git",
                json!({"type":"managed_git","repository_url":"https://example.test/notes.git","mode":"pull_only"}),
            ),
        ];
        assert_eq!(variants.len(), minimal.len());

        for (title, value) in minimal {
            let variant = variants
                .iter()
                .find(|variant| variant["title"] == title)
                .unwrap_or_else(|| panic!("{title} is an advertised source variant"));

            let mut required: Vec<&str> = variant["required"]
                .as_array()
                .expect("required list")
                .iter()
                .map(|field| field.as_str().expect("required field name"))
                .collect();
            let mut supplied: Vec<&str> = value
                .as_object()
                .expect("minimal source object")
                .keys()
                .map(String::as_str)
                .collect();
            required.sort_unstable();
            supplied.sort_unstable();
            assert_eq!(required, supplied, "{title} required fields");

            // Every required field is also described, so an agent reading the
            // schema learns what to put in each one.
            for field in &required {
                assert!(
                    variant["properties"][field].is_object(),
                    "{title}.{field} is described"
                );
            }

            serde_json::from_value::<VaultSource>(value.clone())
                .unwrap_or_else(|error| panic!("{title} minimal source must deserialize: {error}"));

            // `additionalProperties: false` is not decoration: the
            // deserializer really does reject an invented field, so an agent
            // must not be led to guess one.
            let mut invented = value.clone();
            invented["definitely_not_a_field"] = json!("x");
            assert_eq!(variant["additionalProperties"], false);
            assert!(
                serde_json::from_value::<VaultSource>(invented).is_err(),
                "{title} must reject unknown fields"
            );
        }
    }

    /// The credential patch's whole reason to exist is that "keep" is
    /// distinguishable from "clear", so the advertised actions must be the
    /// ones the deserializer answers to.
    #[test]
    fn advertised_credential_actions_match_the_deserializer() {
        let schema = https_credentials_patch_schema();
        let actions: Vec<&str> = schema["oneOf"]
            .as_array()
            .expect("oneOf variants")
            .iter()
            .map(|variant| {
                variant["properties"]["action"]["const"]
                    .as_str()
                    .expect("action")
            })
            .collect();
        assert_eq!(actions, vec!["keep", "remove", "replace"]);

        for value in [
            json!({"action":"keep"}),
            json!({"action":"remove"}),
            json!({"action":"replace","token":"secret"}),
        ] {
            serde_json::from_value::<HttpsCredentialsPatch>(value.clone())
                .unwrap_or_else(|error| panic!("{value} must deserialize: {error}"));
        }
    }
}
