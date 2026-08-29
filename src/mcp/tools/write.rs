//! Vault-scoped MCP tools: note and attachment writes. The mutations —
//! [`WRITE_OPS`], dispatched by [`dispatch_write_tool`] — are gated by
//! `HATCHDOOR_MCP_WRITE_ENABLED` at the dispatch layer in `mod.rs`.
//!
//! Since #186 every mutation here is JSON-RPC shaping and nothing else: parse
//! this tool's arguments, hand them to the Vault mutation core
//! ([`crate::vault_mutation`]), and turn the typed outcome or the structured
//! error into a tool result or a JSON-RPC failure. The Vault gate, the
//! mutation lock, the index build, the slug lookup, the marker and noise
//! refusals, the archive prefix, the off-runtime write, and the write-error
//! translation all live there, shared with the HTTP routes. What stays here
//! is what only this transport knows: its own argument names and wording, the
//! `replace_section` mode spelling, and the base64 envelope `import_attachment`
//! arrives in.
//!
//! Only mutations live here. The three read tools that used to sit alongside
//! them for this module's Vault-scoping helpers (`list_note_attachments`,
//! `get_attachment`, `get_frontmatter`) moved to `read.rs` in #188, once the
//! read core took over the gating those helpers provided.

// The deserialization-only scope and retired legacy commit-summary fields are
// intentionally retained until every old client gets a structured invalid
// parameter response instead of silently accepting an ambiguous payload.
#![allow(dead_code)]

use serde::Deserialize;
use serde_json::{Value, json};

use std::str::FromStr;
use std::sync::Arc;

use crate::app_state::AppState;
use crate::runtime_config::ConfigSnapshot;
use crate::vault::{AttachmentOutcome, SectionMode};
use crate::vault_error::VaultOperationError;
use crate::vault_mutation::{NoteWriteOutcome, VaultMutation};
use crate::vault_read::{VaultReadCore, VaultReadError};
use crate::vault_registry::VaultId;
use crate::vault_runtime::VaultControlBlock;

use super::super::config::McpConfig;
use super::super::protocol::{JsonRpcFailure, tool_success};
use super::{non_empty_argument, write_tool_annotations};

/// A target resolved from the explicit MCP `vault_id`.  It is intentionally
/// not recoverable from `AppState`'s legacy single-Vault fields.
pub(super) struct McpVault {
    pub(super) vault_id: VaultId,
    control: VaultControlBlock,
    /// The live settings snapshot bound when this target was resolved, so the
    /// mutation core reads the same instance-wide defaults (the archive
    /// folder) the rest of this call does.
    settings: Arc<ConfigSnapshot>,
}

impl McpVault {
    /// The mutation core's view of this Vault. `scoped_vault` has already
    /// gated it and applied the core's own `ensure_mutable`, and the
    /// dispatcher holds its mutation lock (`mod.rs` for the length of one
    /// tool call, `batch.rs` for the length of a whole batch), which is why
    /// the operations below never re-take it.
    fn mutation(&self) -> VaultMutation {
        VaultMutation::gated(
            self.vault_id,
            self.control.clone(),
            Arc::clone(&self.settings),
        )
    }
}

/// Resolve the explicit `vault_id` without asserting anything about the
/// Vault's write posture, so [`scoped_vault`] can apply the capability check
/// separately and report it as its own outcome.
fn readable_vault(state: &AppState, arguments: &Value) -> Result<McpVault, JsonRpcFailure> {
    let raw = arguments
        .get("vault_id")
        .and_then(Value::as_str)
        .ok_or_else(|| JsonRpcFailure::invalid_params("vault_id is required"))?;
    let vault_id = VaultId::from_str(raw)
        .map_err(|_| JsonRpcFailure::invalid_params("vault_id must be a canonical Vault ID"))?;
    let core = VaultReadCore::new(&state.startup_sqlite, &state.vaults);
    let control = core.control_block(vault_id).map_err(vault_error)?;
    Ok(McpVault {
        vault_id,
        control,
        settings: state.runtime_snapshot(),
    })
}

pub(super) fn scoped_vault(
    state: &AppState,
    arguments: &Value,
) -> Result<McpVault, JsonRpcFailure> {
    let vault = readable_vault(state, arguments)?;
    crate::vault_mutation::ensure_mutable(vault.vault_id, &vault.control)
        .map_err(mutation_error)?;
    Ok(vault)
}

pub(super) async fn acquire_mutation(
    vault: &McpVault,
) -> Result<tokio::sync::OwnedMutexGuard<()>, JsonRpcFailure> {
    vault
        .mutation()
        .acquire_mutation()
        .await
        .map_err(mutation_error)
}

fn vault_error(error: VaultReadError) -> JsonRpcFailure {
    JsonRpcFailure::not_found(serde_json::to_string(&error).unwrap_or(error.message))
}

/// The MCP half of ADR-19's mapping for a Vault mutation: one structured core
/// error becomes a tool error carrying that same `{code, message, vault_id,
/// retryable}` payload, except the two shapes this surface has always
/// reported at the protocol level instead — a target path this instance will
/// not write (noise-excluded, or the reserved layer marker) is an invalid
/// parameter, and an instance-side failure is an internal error whose detail
/// this surface (unlike HTTP) reports.
fn mutation_error(error: VaultOperationError) -> JsonRpcFailure {
    match error.code.as_str() {
        "noise_excluded_write" | "layer_marker_write" => {
            JsonRpcFailure::invalid_params(error.message)
        }
        "internal_error" => JsonRpcFailure::internal(error.message),
        _ => JsonRpcFailure::not_found(
            serde_json::to_string(&error).unwrap_or_else(|_| error.message.clone()),
        ),
    }
}

/// The success half of that mapping: the core's typed outcome, already
/// carrying its resolved layer, shaped into this tool's result value.
fn note_write_result(vault_id: VaultId, outcome: NoteWriteOutcome) -> Value {
    tool_success(crate::mcp::results::result_to_value(
        &crate::mcp::results::NoteWriteResult {
            vault_id: vault_id.to_string(),
            ok: true,
            slug: outcome.slug,
            relative_path: outcome.relative_path,
            content_hash: outcome.content_hash,
            layer: outcome.layer,
            quality_warnings: outcome.quality_warnings,
            rewritten_notes: outcome.rewritten_notes,
            moved_assets: outcome.moved_assets,
            trashed_path: outcome.trashed_path,
        },
    ))
}

/// Every note/attachment mutation tool, and the one list that says so. The
/// top-level dispatcher (`mod.rs`), the `batch` allow-list (`batch.rs`), and
/// [`dispatch_write_tool`] below all read this rather than repeating the names,
/// so a new write tool is wired by adding it here and to the `match` — and
/// `write_ops_match_the_advertised_catalogue` fails if it is added to the
/// catalogue and forgotten here.
///
/// Vault-management tools (`create_vault` through `retry_vault`) are deliberately
/// absent: they mutate the registry, not a Vault's content, and are dispatched
/// and gated separately.
pub(super) const WRITE_OPS: &[&str] = &[
    "create_note",
    "update_note",
    "append_to_note",
    "edit_note",
    "replace_section",
    "update_frontmatter",
    "rename_note",
    "move_note",
    "move_rename_note",
    "archive_note",
    "delete_note",
    "import_attachment",
    "move_attachment",
    "rename_attachment",
    "delete_attachment",
];

/// Dispatches one write op to its underlying tool function. Shared by the
/// top-level MCP dispatcher (`mod.rs`, one call per request) and the `batch`
/// tool (`batch.rs`, one call per item): both resolve the target Vault and
/// its mutation lock themselves before calling this, since they hold that
/// lock on different schedules — a standalone call for just this one op, a
/// batch item for as long as its whole call keeps touching the same Vault.
pub(super) async fn dispatch_write_tool(
    state: AppState,
    vault: &McpVault,
    op: &str,
    arguments: Value,
    config: &McpConfig,
) -> Result<Value, JsonRpcFailure> {
    match op {
        "create_note" => create_note_tool(state, vault, arguments).await,
        "update_note" => update_note_tool(state, vault, arguments).await,
        "append_to_note" => append_to_note_tool(state, vault, arguments).await,
        "edit_note" => edit_note_tool(state, vault, arguments).await,
        "replace_section" => replace_section_tool(state, vault, arguments).await,
        "update_frontmatter" => update_frontmatter_tool(state, vault, arguments).await,
        "rename_note" => rename_note_tool(state, vault, arguments).await,
        "move_note" => move_note_tool(state, vault, arguments).await,
        "move_rename_note" => move_rename_note_tool(state, vault, arguments).await,
        "archive_note" => archive_note_tool(state, vault, arguments).await,
        "delete_note" => delete_note_tool(state, vault, arguments).await,
        "import_attachment" => import_attachment_tool(state, vault, arguments, config).await,
        "move_attachment" => move_attachment_tool(state, vault, arguments).await,
        "rename_attachment" => rename_attachment_tool(state, vault, arguments).await,
        "delete_attachment" => delete_attachment_tool(state, vault, arguments).await,
        // Unreachable while [`WRITE_OPS`] and the arms above agree, which
        // `write_ops_match_the_advertised_catalogue` enforces. An error rather
        // than a panic anyway: a name that drifts out of step must not be able
        // to take the process down from a request.
        _ => Err(JsonRpcFailure::invalid_params(format!(
            "MCP tool is catalogued as a write tool but has no dispatch: {op}"
        ))),
    }
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
    let outcome = vault
        .mutation()
        .create_note(
            &relative_path,
            &args.content,
            args.overwrite.unwrap_or(false),
        )
        .await
        .map_err(mutation_error)?;
    Ok(note_write_result(vault.vault_id, outcome))
}

pub(super) async fn update_note_tool(
    _state: AppState,
    vault: &McpVault,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: UpdateNoteArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid update_note arguments: {error}"))
    })?;
    let slug = non_empty_argument("slug", args.slug)?;
    let outcome = vault
        .mutation()
        .update_note(&slug, &args.content, &args.expected_content_hash)
        .await
        .map_err(mutation_error)?;
    Ok(note_write_result(vault.vault_id, outcome))
}

pub(super) async fn update_frontmatter_tool(
    _state: AppState,
    vault: &McpVault,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: UpdateFrontmatterArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid update_frontmatter arguments: {error}"))
    })?;
    let slug = non_empty_argument("slug", args.slug)?;
    let outcome = vault
        .mutation()
        .update_frontmatter(&slug, args.frontmatter, &args.expected_content_hash)
        .await
        .map_err(mutation_error)?;
    Ok(note_write_result(vault.vault_id, outcome))
}

pub(super) async fn append_to_note_tool(
    _state: AppState,
    vault: &McpVault,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: AppendNoteArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid append_to_note arguments: {error}"))
    })?;
    let slug = non_empty_argument("slug", args.slug)?;
    let content = non_empty_argument("content", args.content)?;
    let outcome = vault
        .mutation()
        .append_to_note(&slug, &content, &args.expected_content_hash)
        .await
        .map_err(mutation_error)?;
    Ok(note_write_result(vault.vault_id, outcome))
}

pub(super) async fn edit_note_tool(
    _state: AppState,
    vault: &McpVault,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: EditNoteArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid edit_note arguments: {error}"))
    })?;
    let slug = non_empty_argument("slug", args.slug)?;
    let outcome = vault
        .mutation()
        .edit_note(
            &slug,
            &args.old_string,
            &args.new_string,
            &args.expected_content_hash,
            args.replace_all.unwrap_or(false),
        )
        .await
        .map_err(mutation_error)?;
    Ok(note_write_result(vault.vault_id, outcome))
}

pub(super) async fn replace_section_tool(
    _state: AppState,
    vault: &McpVault,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: ReplaceSectionArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid replace_section arguments: {error}"))
    })?;
    let slug = non_empty_argument("slug", args.slug)?;
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
    let outcome = vault
        .mutation()
        .replace_section(
            &slug,
            &heading,
            mode,
            &args.content,
            &args.expected_content_hash,
        )
        .await
        .map_err(mutation_error)?;
    Ok(note_write_result(vault.vault_id, outcome))
}

pub(super) async fn rename_note_tool(
    _state: AppState,
    vault: &McpVault,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: RenameNoteArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid rename_note arguments: {error}"))
    })?;
    let slug = non_empty_argument("slug", args.slug)?;
    let new_title = non_empty_argument("new_title", args.new_title)?;
    // Adapter-owned because the two transports word it differently: this
    // surface reports an invalid parameter, HTTP an `invalid_write_input`
    // `400` (`handlers/vault_write.rs`'s rename route carries the same rule).
    if new_title.contains('/') || new_title.contains('\\') {
        return Err(JsonRpcFailure::invalid_params(
            "new_title cannot contain path separators",
        ));
    }
    let outcome = vault
        .mutation()
        .rename_note(&slug, &new_title, &args.expected_content_hash)
        .await
        .map_err(mutation_error)?;
    Ok(note_write_result(vault.vault_id, outcome))
}

pub(super) async fn move_note_tool(
    _state: AppState,
    vault: &McpVault,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: MoveNoteArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid move_note arguments: {error}"))
    })?;
    let slug = non_empty_argument("slug", args.slug)?;
    let outcome = vault
        .mutation()
        .move_note(&slug, &args.target_folder, &args.expected_content_hash)
        .await
        .map_err(mutation_error)?;
    Ok(note_write_result(vault.vault_id, outcome))
}

pub(super) async fn move_rename_note_tool(
    _state: AppState,
    vault: &McpVault,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: MoveRenameNoteArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid move_rename_note arguments: {error}"))
    })?;
    let slug = non_empty_argument("slug", args.slug)?;
    let target_relative_path =
        non_empty_argument("target_relative_path", args.target_relative_path)?;
    let outcome = vault
        .mutation()
        .move_rename_note(&slug, &target_relative_path, &args.expected_content_hash)
        .await
        .map_err(mutation_error)?;
    Ok(note_write_result(vault.vault_id, outcome))
}

pub(super) async fn archive_note_tool(
    _state: AppState,
    vault: &McpVault,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: ArchiveNoteArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid archive_note arguments: {error}"))
    })?;
    let slug = non_empty_argument("slug", args.slug)?;
    let outcome = vault
        .mutation()
        .archive_note(&slug, &args.expected_content_hash)
        .await
        .map_err(mutation_error)?;
    Ok(note_write_result(vault.vault_id, outcome))
}

pub(super) async fn delete_note_tool(
    _state: AppState,
    vault: &McpVault,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: DeleteNoteArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid delete_note arguments: {error}"))
    })?;
    let slug = non_empty_argument("slug", args.slug)?;
    let outcome = vault
        .mutation()
        .delete_note(&slug, &args.expected_content_hash)
        .await
        .map_err(mutation_error)?;
    Ok(note_write_result(vault.vault_id, outcome))
}

/// The base64 fallback for callers that cannot make an out-of-band HTTP
/// request. Decoding the payload is this transport's own business — the HTTP
/// upload route streams multipart bytes instead — so the cap on the encoded
/// length is checked here, before the decode, and the core then applies the
/// authoritative check to the decoded bytes.
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

    let outcome = vault
        .mutation()
        .import_attachment(
            &target_relative_path,
            bytes,
            config.max_base64_bytes,
            args.overwrite.unwrap_or(false),
        )
        .await
        .map_err(mutation_error)?;
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
    let outcome = vault
        .mutation()
        .move_attachment(&source_relative_path, &target_relative_path)
        .await
        .map_err(mutation_error)?;
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
    let outcome = vault
        .mutation()
        .rename_attachment(&source_relative_path, &new_filename)
        .await
        .map_err(mutation_error)?;
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
    let outcome = vault
        .mutation()
        .delete_attachment(&source_relative_path)
        .await
        .map_err(mutation_error)?;
    Ok(attachment_success(vault.vault_id, outcome))
}

fn attachment_success(vault_id: VaultId, outcome: AttachmentOutcome) -> Value {
    tool_success(crate::mcp::results::result_to_value(
        &crate::mcp::results::AttachmentWriteResult {
            vault_id: vault_id.to_string(),
            ok: true,
            attachment: outcome.attachment,
            rewritten_notes: outcome.rewritten_notes,
            trashed_path: outcome.trashed_path,
            cleanup_warning: outcome.cleanup_warning,
        },
    ))
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
            "name": "update_frontmatter",
            "description": "Shallow top-level YAML merge into an existing note's frontmatter, leaving the body untouched. An explicit null value deletes a key; keys not mentioned survive; nested mappings replace wholesale (shallow semantics). A note with no frontmatter block gets one created. Requires expected_content_hash from get_note.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "slug": {"type": "string", "minLength": 1},
                    "frontmatter": {"type": "object", "additionalProperties": true, "description": "Top-level frontmatter keys to set or replace. A null value deletes the key."},
                    "expected_content_hash": {"type": "string", "minLength": 1},
                    "commit_summary": {"type": "string", "description": "Optional one-line summary of this change for the git commit body."}
                },
                "required": ["slug", "frontmatter", "expected_content_hash"],
                "additionalProperties": false
            },
            "annotations": write_tool_annotations(true, false)
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
            "description": "Upload an attachment into one Vault by sending its bytes base64-encoded. This is the fallback for clients that cannot make an out-of-band HTTP request; it is size-limited (call get_attachment_import_config for this Vault to see the limit in bytes and the allowed extensions). Prefer the Vault-scoped HTTP upload endpoint (POST /api/v1/vaults/{vault_id}/attachments) when possible. Returns compact metadata for the imported file.",
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
struct UpdateFrontmatterArgs {
    vault_id: VaultId,
    slug: String,
    frontmatter: serde_json::Map<String, serde_json::Value>,
    expected_content_hash: String,
    #[serde(default)]
    commit_summary: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GetFrontmatterArgs {
    vault_id: VaultId,
    slug: String,
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
struct GetAttachmentArgs {
    vault_id: VaultId,
    relative_path: String,
    #[serde(default)]
    encoding: Option<AttachmentEncoding>,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum AttachmentEncoding {
    Url,
    Base64,
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
    fn recovery_required_write_errors_expose_bounded_guidance() {
        // The translation is the core's; this asserts what reaches an MCP
        // client through this surface's own mapping of it.
        let vault_id = crate::vault_registry::VaultId::generate().expect("generate Vault id");
        let failure = mutation_error(crate::vault_mutation::write_operation_error(
            vault_id,
            crate::vault::WriteError::recovery_required(
                "vault mutation rollback was incomplete: restore rewritten note [Backlink.md]"
                    .to_string(),
            ),
        ));

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
    fn write_ops_match_the_advertised_catalogue() {
        // The drift guard for the single source of truth: `WRITE_OPS` gates
        // dispatch in `mod.rs` and membership in `batch.rs`, while
        // `write_tools_list()` is what clients are actually told exists. A tool
        // catalogued but missing here would be advertised and then refused; one
        // listed here but not catalogued would be a gate on nothing.
        let mut advertised: Vec<String> = write_tools_list()
            .iter()
            .map(|tool| tool["name"].as_str().expect("tool name").to_string())
            .collect();
        let mut declared: Vec<String> = WRITE_OPS.iter().map(|op| (*op).to_string()).collect();
        advertised.sort();
        declared.sort();
        assert_eq!(
            advertised, declared,
            "WRITE_OPS and write_tools_list() must name exactly the same tools"
        );
    }

    #[test]
    fn mutation_error_maps_every_core_code_this_surface_can_receive() {
        // ADR-19: the core reports one structured error and this adapter owns
        // the JSON-RPC shape. Two meanings live only here — an unwritable
        // target path is an invalid parameter, and an instance-side failure is
        // an internal error whose detail this surface (unlike HTTP) reports.
        let vault_id = VaultId::generate().expect("generate Vault id");
        let map = |code: &str| {
            mutation_error(VaultOperationError::new(
                code,
                "detail",
                Some(vault_id),
                false,
            ))
        };

        let noise = map("noise_excluded_write");
        assert_eq!(noise.code, -32602);
        assert!(!noise.tool_level);
        assert_eq!(noise.message, "detail");

        let internal = map("internal_error");
        assert_eq!(internal.code, JsonRpcFailure::INTERNAL_ERROR_CODE);
        assert!(!internal.tool_level);
        assert_eq!(internal.message, "detail");

        // Everything else keeps carrying the structured payload verbatim, so
        // a client reads the same `{code, message, vault_id, retryable}` the
        // HTTP surface returns.
        for code in [
            "write_conflict",
            "capability_unavailable",
            "invalid_write_input",
            "note_not_found",
            "write_recovery_required",
            "write_failed",
            "vault_not_found",
            "vault_disabled",
            "vault_read_unavailable",
        ] {
            let failure = map(code);
            assert!(failure.tool_level, "{code} must stay a tool error");
            let payload: Value =
                serde_json::from_str(&failure.message).expect("structured payload");
            assert_eq!(payload["code"], code);
            assert_eq!(payload["message"], "detail");
            assert_eq!(payload["vault_id"], vault_id.to_string());
            assert_eq!(payload["retryable"], false);
        }
    }

    #[test]
    fn note_write_result_carries_the_cores_outcome_verbatim() {
        let vault_id = VaultId::generate().expect("generate Vault id");
        let value = note_write_result(
            vault_id,
            NoteWriteOutcome {
                slug: Some("clip".to_string()),
                relative_path: Some("sources/Clip".to_string()),
                content_hash: Some("h".to_string()),
                quality_warnings: vec!["warn".to_string()],
                rewritten_notes: 2,
                moved_assets: 1,
                trashed_path: None,
                layer: Some("sources".to_string()),
            },
        );
        let content = &value["structuredContent"];
        assert_eq!(content["ok"], true);
        assert_eq!(content["vault_id"], vault_id.to_string());
        assert_eq!(content["slug"], "clip");
        assert_eq!(content["relative_path"], "sources/Clip");
        assert_eq!(content["content_hash"], "h");
        assert_eq!(content["layer"], "sources");
        assert_eq!(content["quality_warnings"][0], "warn");
        assert_eq!(content["rewritten_notes"], 2);
        assert_eq!(content["moved_assets"], 1);
    }
}
