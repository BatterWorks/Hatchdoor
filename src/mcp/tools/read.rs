//! Vault-scoped MCP read tools, plus the eight Vault collection management
//! tools. These are deliberately thin in-process adapters over the same
//! shared cores used by HTTP: MCP owns JSON-RPC framing, while scope parsing,
//! projections, and error shapes stay in the core.
//!
//! Since #188 no tool here proxies an HTTP handler. Each read parses its
//! arguments, calls `VaultReadCore`, `VaultSearchCore`, or (for the
//! collection tools) `vault_management::VaultCollectionManagement` through the
//! read core's own off-runtime offload, and serialises the resulting
//! projection once, through exactly the structure whose schema `tools/list`
//! advertises. There is no axum extractor, no HTTP response body, and no byte
//! cap between a tool and its answer — that was ADR-19's last piece of
//! MCP-to-handler proxying.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use std::str::FromStr;

use crate::app_state::AppState;
use crate::mcp::results;
use crate::search::vault_scoped::{VaultSearchCore, VaultSearchRequest};
use crate::vault::allowed_attachment_extensions;
use crate::vault_error::VaultOperationError;
use crate::vault_management::{
    CreateVaultRequest, EditVaultRequest, HttpsCredentialsPatch, VaultCollectionManagement,
};
use crate::vault_read::{
    AssetPathError, AssetReadError, OffloadedReadError, ResolvedAsset, TreeScope, VaultReadError,
    VaultReads, VaultResolveResponse, VaultScope, asset_download_path, clamp_recent_limit,
    clamp_search_limit, clamp_search_per_note_cap, clamp_tree_max_depth, note_not_found,
};
use crate::vault_registry::VaultId;

use super::super::config::McpConfig;
use super::super::protocol::{JsonRpcFailure, tool_structured_error, tool_success};
use super::read_only_tool_annotations;

/// One tool's answer: the structure whose schema `tools/list` advertises,
/// serialised once.
fn tool_result<T: Serialize>(result: &T) -> Value {
    tool_success(results::result_to_value(result))
}

/// The MCP half of ADR-19's mapping for a Vault read: one structured core
/// failure becomes a tool error carrying that same
/// `{code, message, vault_id?, retryable}` payload — byte-identical to the
/// body these tools used to decode back out of a proxied HTTP response — while
/// a blocking task that never completed is an instance-side fault, reported as
/// a JSON-RPC internal error whose detail the dispatcher masks.
fn read_failure(error: OffloadedReadError) -> Result<Value, JsonRpcFailure> {
    match error {
        OffloadedReadError::Read(error) => Ok(structured_error(error.into_operation_error())),
        OffloadedReadError::Failed(message) => Err(JsonRpcFailure::internal(message)),
    }
}

/// A domain failure as a tool error carrying the shared
/// `{code, message, vault_id?, retryable}` object, so an agent branches on
/// `code` rather than on human text. Byte-identical to the body these tools
/// used to decode back out of a proxied HTTP response.
fn structured_error(error: VaultOperationError) -> Value {
    tool_structured_error(
        serde_json::to_value(&error).unwrap_or_else(|_| json!({ "code": error.code })),
    )
}

/// The `vault_id` argument of a Vault-qualified *read*, refused with the same
/// structured `invalid_vault_id` error the HTTP adapter reports for the same
/// malformed path segment — the shape these tools have always returned, back
/// when they decoded that route's `400` body.
fn read_vault_id(raw: &str) -> Result<VaultId, Value> {
    crate::vault_management::parse_vault_id(raw).map_err(structured_error)
}

/// The `vault_id` argument of the attachment and frontmatter tools, which have
/// always refused a malformed ID at the protocol level instead: they resolved
/// their Vault before parsing any other argument, rather than through a route
/// that had already shaped one. Kept distinct from [`read_vault_id`] so neither
/// group's existing refusal changes.
fn scoped_vault_id(raw: &str) -> Result<VaultId, JsonRpcFailure> {
    VaultId::from_str(raw)
        .map_err(|_| JsonRpcFailure::invalid_params("vault_id must be a canonical Vault ID"))
}

/// The `scope` argument of a collection read, refused as the core's structured
/// `invalid_scope` error.
fn tool_scope(raw: &str) -> Result<VaultScope, Value> {
    VaultScope::parse(raw).map_err(|error| structured_error(error.into_operation_error()))
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

/// `get_tree`'s arguments. The three narrowing ones are optional and default to
/// the whole Vault, so a caller that passes only `scope` reads the tree it
/// always did.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TreeArgs {
    scope: String,
    #[serde(default)]
    folder: Option<String>,
    #[serde(default)]
    max_depth: Option<u32>,
    #[serde(default = "notes_included_by_default")]
    include_notes: bool,
}

fn notes_included_by_default() -> bool {
    true
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
    let scope = match tool_scope(&args.scope) {
        Ok(scope) => scope,
        Err(refusal) => return Ok(refusal),
    };
    let reads = VaultReads::new(&state);
    let surface = reads.surface();
    // The same comma-separated token grammar the `layers=` query uses, so a
    // selector means the same thing on both surfaces.
    let layers = surface.layer_selection(
        (!args.layers.is_empty())
            .then(|| args.layers.join(","))
            .as_deref(),
    );
    let request = VaultSearchRequest {
        scope,
        query: args.query,
        mode: args.mode.unwrap_or_default(),
        limit: clamp_search_limit(args.limit),
        per_note_cap: clamp_search_per_note_cap(args.per_note_cap),
        layers,
    };
    let cache = state.startup_sqlite.clone();
    let vaults = state.vaults.clone();
    let embedder = state.embedder.clone();
    // Query embedding (semantic mode) and SQLite work both run off the async
    // runtime, exactly as the HTTP search route runs them.
    let result = tokio::task::spawn_blocking(move || {
        VaultSearchCore::new(&cache, &vaults, embedder.as_ref())
            .on_surface(surface)
            .search(request)
    })
    .await;
    match result {
        Ok(Ok(projection)) => Ok(tool_result::<results::SearchNotesResult>(&projection)),
        Ok(Err(error)) => Ok(structured_error(error.into_operation_error())),
        Err(join_error) => Err(JsonRpcFailure::internal(format!(
            "background task panicked: {join_error}"
        ))),
    }
}

pub(super) async fn get_note_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: ExactSlugArgs = parse("get_note", arguments)?;
    let vault_id = match read_vault_id(&args.vault_id) {
        Ok(vault_id) => vault_id,
        Err(refusal) => return Ok(refusal),
    };
    let slug = args.slug.clone();
    match VaultReads::new(&state)
        .read(move |core| core.exact_note(vault_id, &slug))
        .await
    {
        Ok(Some(note)) => Ok(tool_result::<results::GetNoteResult>(&note)),
        Ok(None) => Ok(structured_error(note_not_found(vault_id, &args.slug))),
        Err(error) => read_failure(error),
    }
}

pub(super) async fn get_note_links_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: ExactSlugArgs = parse("get_note_links", arguments)?;
    let vault_id = match read_vault_id(&args.vault_id) {
        Ok(vault_id) => vault_id,
        Err(refusal) => return Ok(refusal),
    };
    let slug = args.slug.clone();
    match VaultReads::new(&state)
        .read(move |core| core.exact_note_links(vault_id, &slug))
        .await
    {
        Ok(Some(links)) => Ok(tool_result::<results::GetNoteLinksResult>(&links)),
        Ok(None) => Ok(structured_error(note_not_found(vault_id, &args.slug))),
        Err(error) => read_failure(error),
    }
}

pub(super) async fn resolve_wikilink_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: ResolveArgs = parse("resolve_wikilink", arguments)?;
    let vault_id = match read_vault_id(&args.vault_id) {
        Ok(vault_id) => vault_id,
        Err(refusal) => return Ok(refusal),
    };
    match VaultReads::new(&state)
        .read(move |core| core.resolve_wikilink(vault_id, &args.target))
        .await
    {
        Ok(resolved) => Ok(tool_result::<results::ResolveWikilinkResult>(
            &VaultResolveResponse {
                vault_id,
                slug: resolved.map(|resolved| resolved.slug),
            },
        )),
        Err(error) => read_failure(error),
    }
}

pub(super) async fn get_tree_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: TreeArgs = parse("get_tree", arguments)?;
    let scope = match tool_scope(&args.scope) {
        Ok(scope) => scope,
        Err(refusal) => return Ok(refusal),
    };
    let tree_scope = TreeScope {
        folder: args.folder,
        max_depth: clamp_tree_max_depth(args.max_depth),
        include_notes: args.include_notes,
    };
    match VaultReads::new(&state)
        .read(move |core| core.trees(scope, tree_scope))
        .await
    {
        Ok(projection) => Ok(tool_result::<results::GetTreeResult>(&projection)),
        Err(error) => read_failure(error),
    }
}

pub(super) async fn get_stats_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: ScopeArgs = parse("get_stats", arguments)?;
    let scope = match tool_scope(&args.scope) {
        Ok(scope) => scope,
        Err(refusal) => return Ok(refusal),
    };
    match VaultReads::new(&state)
        .read(move |core| core.statistics(scope))
        .await
    {
        Ok(projection) => Ok(tool_result(&results::StampedStatsResult {
            hatchdoor_version: crate::config::version_string(),
            projection,
        })),
        Err(error) => read_failure(error),
    }
}

pub(super) async fn get_graph_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    collection_read(state, "get_graph", arguments, |core, scope| {
        core.graphs(scope)
    })
    .await
}

/// The three scope-only collection reads differ by nothing but the projection
/// they ask the core for.
async fn collection_read<T, F>(
    state: AppState,
    tool: &str,
    arguments: Value,
    project: F,
) -> Result<Value, JsonRpcFailure>
where
    T: Serialize + Send + 'static,
    F: FnOnce(&crate::vault_read::VaultReadCore<'_>, VaultScope) -> Result<T, VaultReadError>
        + Send
        + 'static,
{
    let args: ScopeArgs = parse(tool, arguments)?;
    let scope = match tool_scope(&args.scope) {
        Ok(scope) => scope,
        Err(refusal) => return Ok(refusal),
    };
    match VaultReads::new(&state)
        .read(move |core| project(core, scope))
        .await
    {
        Ok(projection) => Ok(tool_result(&projection)),
        Err(error) => read_failure(error),
    }
}

pub(super) async fn recently_modified_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: RecentArgs = parse("recently_modified", arguments)?;
    let scope = match tool_scope(&args.scope) {
        Ok(scope) => scope,
        Err(refusal) => return Ok(refusal),
    };
    let limit = clamp_recent_limit(args.limit);
    match VaultReads::new(&state)
        .read(move |core| core.recently_modified(scope, limit))
        .await
    {
        Ok(projection) => Ok(tool_result::<results::RecentlyModifiedResult>(&projection)),
        Err(error) => read_failure(error),
    }
}

pub(super) async fn get_frontmatter_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: ExactSlugArgs = parse("get_frontmatter", arguments)?;
    let vault_id = scoped_vault_id(&args.vault_id)?;
    // The empty-slug refusal this tool has always applied before looking a
    // Note up, kept at the protocol level rather than folded into not-found.
    let slug = super::non_empty_argument("slug", args.slug)?;
    let lookup_slug = slug.clone();
    match VaultReads::new(&state)
        .read(move |core| core.exact_note_frontmatter(vault_id, &lookup_slug))
        .await
    {
        Ok(Some(frontmatter)) => Ok(tool_result(&results::GetFrontmatterResult {
            vault_id: vault_id.to_string(),
            slug: frontmatter.slug,
            relative_path: frontmatter.relative_path,
            content_hash: frontmatter.content_hash,
            has_frontmatter: frontmatter.has_frontmatter,
            tags: frontmatter.metadata.tags,
            aliases: frontmatter.metadata.aliases,
            properties: frontmatter.metadata.properties,
        })),
        Ok(None) => Ok(structured_error(note_not_found(vault_id, &slug))),
        Err(error) => read_failure(error),
    }
}

pub(super) async fn list_note_attachments_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: ExactSlugArgs = parse("list_note_attachments", arguments)?;
    let vault_id = scoped_vault_id(&args.vault_id)?;
    let slug = super::non_empty_argument("slug", args.slug)?;
    let lookup_slug = slug.clone();
    match VaultReads::new(&state)
        .read(move |core| core.note_attachments(vault_id, &lookup_slug))
        .await
    {
        Ok(Some(attachments)) => Ok(tool_result(&results::NoteAttachmentsResult {
            vault_id: vault_id.to_string(),
            attachments,
        })),
        Ok(None) => Ok(structured_error(note_not_found(vault_id, &slug))),
        Err(error) => read_failure(error),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GetAttachmentArgs {
    vault_id: String,
    relative_path: String,
    #[serde(default)]
    encoding: Option<AttachmentEncoding>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum AttachmentEncoding {
    Url,
    Base64,
}

/// One attachment resolved for this tool: what to report about it, and its
/// bytes when the caller asked for base64 and they fit under the cap. The
/// attachment keeps being named by the `relative_path` the caller asked with,
/// which is also what `download_url` addresses.
struct FetchedAttachment {
    size_bytes: u64,
    content_type: &'static str,
    bytes: Option<Vec<u8>>,
}

/// Fetch one attachment's bytes: an HTTP download URL by default, or
/// base64-inline content as the fallback when an out-of-band HTTP request
/// isn't possible or the URL's own credential isn't available to this client.
/// Addressed by `relative_path`, resolved through the read core's
/// contained-resource seam — the same Vault gate, containment check, extension
/// allow-list, and browse surface the `/assets/{*path}` route answers on, so a
/// path one surface refuses is refused identically by the other.
pub(super) async fn get_attachment_tool(
    state: AppState,
    arguments: Value,
    config: &McpConfig,
) -> Result<Value, JsonRpcFailure> {
    let args: GetAttachmentArgs = parse("get_attachment", arguments)?;
    let vault_id = scoped_vault_id(&args.vault_id)?;
    let relative_path = super::non_empty_argument("relative_path", args.relative_path)?;
    let encoding = args.encoding.unwrap_or(AttachmentEncoding::Url);
    let max_base64_bytes = config.max_base64_bytes;

    // Resolution, the size check, and the read are all blocking filesystem
    // work; they happen in one offloaded trip rather than on a tokio worker.
    let lookup_path = relative_path.clone();
    let fetched = VaultReads::new(&state)
        .read(move |core| {
            let asset: ResolvedAsset = match core.contained_asset(vault_id, &lookup_path)? {
                Ok(asset) => asset,
                Err(error) => return Ok(Err(AttachmentFailure::Path(error))),
            };
            let bytes = match encoding {
                AttachmentEncoding::Url => None,
                AttachmentEncoding::Base64 => {
                    if asset.size_bytes > max_base64_bytes {
                        return Ok(Err(AttachmentFailure::TooLargeForBase64(asset.size_bytes)));
                    }
                    match asset.read_bytes() {
                        Ok(bytes) => Some(bytes),
                        Err(AssetReadError::TooLarge) => {
                            return Ok(Err(AttachmentFailure::Path(AssetPathError::TooLarge)));
                        }
                        Err(AssetReadError::Io(_)) => {
                            return Ok(Err(AttachmentFailure::Path(AssetPathError::Internal)));
                        }
                    }
                }
            };
            Ok(Ok(FetchedAttachment {
                size_bytes: asset.size_bytes,
                content_type: asset.content_type,
                bytes,
            }))
        })
        .await;

    let fetched = match fetched {
        Ok(Ok(fetched)) => fetched,
        Ok(Err(AttachmentFailure::Path(error))) => {
            return Ok(structured_error(VaultOperationError::new(
                error.code(),
                error.message(&relative_path),
                Some(vault_id),
                false,
            )));
        }
        Ok(Err(AttachmentFailure::TooLargeForBase64(size_bytes))) => {
            return Err(JsonRpcFailure::invalid_params(format!(
                "attachment exceeds max size for base64 encoding: {size_bytes} > {max_base64_bytes}; call get_attachment again with encoding \"url\" instead"
            )));
        }
        Err(error) => return read_failure(error),
    };

    let content = match fetched.bytes {
        None => results::AttachmentContent::Url {
            download_url: asset_download_path(&vault_id.to_string(), &relative_path),
            path_note: "Relative path — resolve it against the same scheme, host, and port as this MCP endpoint.",
            auth: "Send this MCP session's own bearer token as an Authorization: Bearer header; the route accepts it for as long as MCP stays enabled. This deployment's web bearer token (HATCHDOOR_WEB_BEARER_TOKEN) also works, as a header or an access_token query parameter. When neither token is configured, or demo mode is enabled, the URL needs no credential. If this client cannot make an out-of-band HTTP request at all, call get_attachment again with encoding \"base64\".",
        },
        Some(bytes) => {
            use base64::Engine as _;
            results::AttachmentContent::Base64 {
                content: base64::engine::general_purpose::STANDARD.encode(&bytes),
            }
        }
    };

    Ok(tool_result(&results::GetAttachmentResult {
        vault_id: vault_id.to_string(),
        relative_path,
        size_bytes: fetched.size_bytes,
        content_type: fetched.content_type.to_string(),
        content,
    }))
}

enum AttachmentFailure {
    Path(AssetPathError),
    TooLargeForBase64(u64),
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
pub(super) async fn attachment_import_config_tool(
    state: AppState,
    config: &McpConfig,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: VaultIdArgs = parse("get_attachment_import_config", arguments)?;
    let vault_id = scoped_vault_id(&args.vault_id)?;
    let capabilities = match VaultReads::new(&state)
        .read(move |core| core.vault_capabilities(vault_id))
        .await
    {
        Ok(capabilities) => capabilities,
        Err(error) => return read_failure(error),
    };
    let vault_mutable = capabilities.mutate;
    let enabled = config.write_enabled && vault_mutable;

    let methods: Vec<results::AttachmentImportMethod> = if enabled {
        vec![
            results::AttachmentImportMethod::HttpMultipart {
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
            results::AttachmentImportMethod::McpBase64 {
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

    Ok(tool_result(&results::AttachmentImportConfigResult {
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
    }))
}

/// One Vault collection management outcome: the core's typed response
/// serialized through exactly the structure whose schema `tools/list`
/// advertises, or its structured failure as a tool error.
fn management_result<T: Serialize>(
    result: Result<T, VaultOperationError>,
) -> Result<Value, JsonRpcFailure> {
    Ok(match result {
        Ok(response) => tool_result(&response),
        Err(error) => structured_error(error),
    })
}

/// The Vault ID every control of an existing Vault carries, parsed by the
/// same core function the HTTP adapter uses so a malformed ID is refused
/// identically on both surfaces. Its refusal is an ordinary structured
/// management error, so it flows through `management_result` with every other
/// outcome rather than returning early on its own path.
fn management_vault_id(raw: &str) -> Result<VaultId, VaultOperationError> {
    crate::vault_management::parse_vault_id(raw)
}

pub(super) async fn list_vaults_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let _: EmptyArgs = parse("list_vaults", arguments)?;
    management_result::<results::ListVaultsResult>(VaultCollectionManagement::new(&state).list())
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
    management_result::<results::CreateVaultResult>(
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
    management_result::<results::EditVaultResult>(result)
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
    management_result::<results::EnableVaultResult>(result)
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
    management_result::<results::DisableVaultResult>(result)
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
    management_result::<results::DisconnectVaultResult>(result)
}

pub(super) async fn sync_vault_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: VaultIdArgs = parse("sync_vault", arguments)?;
    let core = VaultCollectionManagement::new(&state);
    management_result::<results::SyncVaultResult>(
        management_vault_id(&args.vault_id).and_then(|vault_id| core.sync(vault_id)),
    )
}

pub(super) async fn retry_vault_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: VaultIdArgs = parse("retry_vault", arguments)?;
    let core = VaultCollectionManagement::new(&state);
    management_result::<results::RetryVaultResult>(
        management_vault_id(&args.vault_id).and_then(|vault_id| core.retry(vault_id)),
    )
}

/// Unlike `sync_vault`/`retry_vault` above, this one talks to no Git remote:
/// it admits the Vault's next Index turn, which is what republishes the
/// snapshot every collection read projects from. It is the only MCP path to
/// that turn, and the reason a client can now act on a collection read that
/// reports itself stale rather than only observe it (#228).
pub(super) async fn refresh_vault_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: VaultIdArgs = parse("refresh_vault", arguments)?;
    let core = VaultCollectionManagement::new(&state);
    management_result::<results::RefreshVaultResult>(
        management_vault_id(&args.vault_id).and_then(|vault_id| core.refresh(vault_id)),
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
        tree_tool(),
        collection_tool(
            "get_stats",
            "Return grouped statistics for one Vault or all enabled Vaults.",
        ),
        collection_tool(
            "get_graph",
            "Return grouped graphs for one Vault or all enabled Vaults.",
        ),
        json!({"name":"get_frontmatter", "description":"Read one exact Note's frontmatter metadata — tags, aliases, and properties — from its authoritative Vault Markdown directory, without returning the Markdown body. A note without a frontmatter block returns an empty/default projection rather than an error. Also returns the note's content_hash — the same string get_note reports — so a hash-protected write can be prepared without reading the body.", "inputSchema":{"type":"object","properties":{"vault_id":vault_id_schema(),"slug":{"type":"string","minLength":1}},"required":["vault_id","slug"],"additionalProperties":false},"annotations":read_only_tool_annotations()}),
        json!({"name":"list_note_attachments", "description":"List the existing attachments one Note references, without returning the Note's full content. Every non-Markdown file the Note points at counts, not only the types get_attachment can fetch back.", "inputSchema":{"type":"object","properties":{"vault_id":vault_id_schema(),"slug":{"type":"string","minLength":1}},"required":["vault_id","slug"],"additionalProperties":false},"annotations":read_only_tool_annotations()}),
        json!({"name":"get_attachment", "description":"Fetch one attachment's bytes, addressed by relative_path exactly as list_note_attachments reports it. Fetchable types are narrower than the set Hatchdoor manages: png, jpg, jpeg, gif, webp, svg, avif, bmp and pdf. A file of any other type - video, audio, data, an archive - is listed, moved, renamed and deleted like any other attachment but is refused here, so do not assume every path list_note_attachments returns can be fetched. encoding \"url\" (the default) returns an HTTP download_url resolved against this MCP endpoint's scheme, host, and port; encoding \"base64\" returns inline base64 content instead, for a client that cannot make an out-of-band HTTP request or cannot obtain the download URL's own credential, bounded by the same size limit as import_attachment's base64 path.", "inputSchema":{"type":"object","properties":{"vault_id":vault_id_schema(),"relative_path":{"type":"string","minLength":1},"encoding":{"type":"string","enum":["url","base64"],"default":"url"}},"required":["vault_id","relative_path"],"additionalProperties":false},"annotations":read_only_tool_annotations()}),
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
        json!({"name":"refresh_vault","description":"Request one Vault's next index turn: Hatchdoor re-scans that Vault's Markdown and republishes the snapshot get_tree, get_graph, get_stats, recently_modified and search_notes project from. Call this when one of those reads comes back with partial: true and a stale participant for the Vault. This is not sync_vault: it contacts no Git remote and works on any enabled Vault with usable local Markdown. It returns as soon as the turn is admitted, not when the turn finishes — schedule is queued, or coalesced when a turn for that Vault is already pending — so observe the outcome by re-reading a collection read's freshness fields rather than by this response.","inputSchema":{"type":"object","properties":{"vault_id":vault_id_schema()},"required":["vault_id"],"additionalProperties":false},"annotations":super::write_tool_annotations(false, true)}),
    ]
}

fn collection_tool(name: &str, description: &str) -> Value {
    json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":{"scope":scope_schema()},"required":["scope"],"additionalProperties":false},"annotations":read_only_tool_annotations()})
}

/// `get_tree` no longer shares [`collection_tool`]'s scope-only schema: it takes
/// three optional narrowing arguments (#192) that `get_stats` and `get_graph`
/// deliberately do not.
///
/// The description carries its own weight here. An agent choosing between a
/// whole-Vault dump and a one-kilobyte orientation call has nothing else to go
/// on, so the cheap call is named in it rather than left to be discovered.
fn tree_tool() -> Value {
    json!({
        "name": "get_tree",
        "description": "Return grouped explorer trees for one Vault or all enabled Vaults. With no folder, max_depth or include_notes the entire Vault is returned, which is large. To see a Vault's shape cheaply, call with include_notes false: that returns every folder at every level with its note count and no notes. Use folder to read one subtree, and max_depth to stop descending. Every folder reports note_count, the notes directly inside it; a folder held back by max_depth is marked truncated.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "scope": scope_schema(),
                "folder": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Vault-relative folder to return as the tree root, such as \"40-reference/Parenting\". Matched case-insensitively. A folder that does not exist is an error, not an empty tree. Defaults to the whole Vault."
                },
                "max_depth": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "How far below the starting folder to descend, the starting folder being depth 0. A folder at this depth is listed with its note count but not expanded. Defaults to unlimited."
                },
                "include_notes": {
                    "type": "boolean",
                    "default": true,
                    "description": "Whether notes appear at all. Folders and their note counts are returned either way."
                }
            },
            "required": ["scope"],
            "additionalProperties": false
        },
        "annotations": read_only_tool_annotations()
    })
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
    fn only_get_tree_advertises_the_tree_narrowing_arguments() {
        let tools = read_tools_list();
        let named = |name: &str| {
            tools
                .iter()
                .find(|tool| tool["name"] == name)
                .unwrap_or_else(|| panic!("{name} is advertised"))
        };

        let tree = named("get_tree");
        let properties = &tree["inputSchema"]["properties"];
        for argument in ["folder", "max_depth", "include_notes"] {
            assert!(
                properties[argument].is_object(),
                "get_tree advertises {argument}"
            );
        }
        // Optional, so a caller passing only `scope` still reads the whole
        // tree, exactly as it did before #192.
        assert_eq!(
            tree["inputSchema"]["required"],
            json!(["scope"]),
            "the narrowing arguments must stay optional"
        );
        assert_eq!(properties["include_notes"]["default"], json!(true));
        assert_eq!(properties["max_depth"]["minimum"], json!(1));
        // The cheap orientation call has only the description to announce it.
        let description = tree["description"].as_str().expect("get_tree description");
        assert!(description.contains("include_notes"));

        // `get_stats` and `get_graph` shared `get_tree`'s schema builder until
        // this split. They take scope and nothing else, and still do.
        for name in ["get_stats", "get_graph"] {
            assert_eq!(
                named(name)["inputSchema"]["properties"]
                    .as_object()
                    .unwrap_or_else(|| panic!("{name} has properties"))
                    .keys()
                    .collect::<Vec<_>>(),
                vec!["scope"],
                "{name} takes scope alone"
            );
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
            "refresh_vault",
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
