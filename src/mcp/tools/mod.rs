//! MCP tool surface. Dispatch and the shared helpers live here, and permission
//! is decided here alone: [`READ_OPS`] names what answers under read
//! permission, [`write::WRITE_OPS`] what needs `HATCHDOOR_MCP_WRITE_ENABLED`.
//!
//! Every read tool now lives in `read`, every mutation in `write`; the split
//! that once put three Vault-scoped reads next to the mutations (for their
//! scoping helpers) went with #188, which moved that gating into the read
//! core. Read the two op lists to know what a tool requires.

mod batch;
mod read;
mod write;

use serde_json::{Value, json};

use super::config::McpConfig;
use super::protocol::{JsonRpcFailure, tool_error, tool_structured_error, tool_success};
use crate::app_state::AppState;

/// Shared by every call site that rejects a write-shaped tool while
/// `HATCHDOOR_MCP_WRITE_ENABLED` is off: the top-level dispatcher below (for
/// both a standalone write tool and a vault-management tool) and `batch`'s
/// own per-item gate, so the wording can't drift between them.
pub(super) const WRITE_DISABLED_MESSAGE: &str =
    "MCP write tools are disabled by HATCHDOOR_MCP_WRITE_ENABLED";

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

    if name == "get_model_setup_status" {
        return Ok(tool_success(crate::mcp::results::result_to_value(
            &model_setup_status_result(&state),
        )));
    }

    // While model setup is still pending, only the explicit model-setup calls
    // and Vault collection discovery/management may run. `state.startup`
    // tracks the legacy single-Vault embedding-model setup, which has no
    // bearing on the Vault registry: zero Vaults or a registry in Recovery are
    // normal, expected states, and an agent must be able to see and repair the
    // collection precisely then. This mirrors `handlers/vaults.rs`, whose
    // whole HTTP surface is deliberately not gated by this legacy readiness
    // signal. The full tool catalogue is still advertised so MCP clients that
    // cache tools at connection time need no restart once setup completes.
    //
    // The question is `model_setup_pending`, not whether the collection's
    // indexes are settled: the same tracker doubles as the live
    // indexing-progress channel, so asking the latter also caught every
    // routine post-write reindex and told the caller to go accept a licence it
    // had accepted months earlier (#191). Scanning and indexing fall through
    // to the Vault-scoped cores, which report a rebuilding Vault themselves
    // and accurately.
    if state.startup.model_setup_pending() && !is_collection_management_tool(name) {
        return match name {
            "accept_gemma_terms" => select_model_tool(
                state,
                crate::model_setup::SelectedModel::Gemma,
                crate::mcp::results::ModelChoiceResult {
                    accepted: true,
                    model: crate::model_setup::GEMMA_MODEL_ID,
                },
            ),
            "decline_gemma_terms" => select_model_tool(
                state,
                crate::model_setup::SelectedModel::Nomic,
                crate::mcp::results::ModelChoiceResult {
                    accepted: false,
                    model: crate::model_setup::NOMIC_MODEL_ID,
                },
            ),
            // Logged because the client is the only party that used to learn
            // of this rejection: it produced no server-side record at all, so
            // a session full of them was invisible in the logs (#191).
            _ => {
                tracing::info!(
                    tool = name,
                    setup_state = state.startup.status().state,
                    "MCP tool call rejected: first-run model setup is not complete"
                );
                Ok(tool_error(
                    "Hatchdoor is still being set up. Use get_model_setup_status, accept_gemma_terms, or decline_gemma_terms first.".to_string(),
                ))
            }
        };
    }

    if matches!(name, "accept_gemma_terms" | "decline_gemma_terms") {
        return Ok(tool_error(
            "A search model is already set up. Changing models after setup is not supported."
                .to_string(),
        ));
    }

    let outcome = match name {
        // Collection discovery, deliberately outside `READ_OPS`: it answers
        // about the Vault collection rather than about any Vault's content, so
        // a `batch` item never names it.
        "list_vaults" => read::list_vaults_tool(state, arguments).await,
        read_op if READ_OPS.contains(&read_op) => {
            dispatch_read_tool(state, config, name, arguments).await
        }
        // Not gated on `write_enabled` at this level: a batch may be
        // read-only. Any write-shaped item inside it is gated individually,
        // the same way a standalone write tool call is below.
        "batch" => batch::batch_tool(state, arguments, config).await,
        "create_vault" if config.write_enabled => read::create_vault_tool(state, arguments).await,
        "edit_vault" if config.write_enabled => read::edit_vault_tool(state, arguments).await,
        "enable_vault" if config.write_enabled => read::enable_vault_tool(state, arguments).await,
        "disable_vault" if config.write_enabled => read::disable_vault_tool(state, arguments).await,
        "disconnect_vault" if config.write_enabled => {
            read::disconnect_vault_tool(state, arguments).await
        }
        "sync_vault" if config.write_enabled => read::sync_vault_tool(state, arguments).await,
        "retry_vault" if config.write_enabled => read::retry_vault_tool(state, arguments).await,
        write_op if write::WRITE_OPS.contains(&write_op) && config.write_enabled => {
            let vault = write::scoped_vault(&state, &arguments)?;
            // This is the same per-Vault mutation lock used by the V1 HTTP
            // adapter.  The legacy instance-wide AppState lock deliberately
            // does not participate in this scoped path.
            let _guard = write::acquire_mutation(&vault).await?;
            write::dispatch_write_tool(state, &vault, name, arguments, config).await
        }
        write_op if write::WRITE_OPS.contains(&write_op) => {
            Err(JsonRpcFailure::invalid_params(WRITE_DISABLED_MESSAGE))
        }
        "create_vault" | "edit_vault" | "enable_vault" | "disable_vault" | "disconnect_vault"
        | "sync_vault" | "retry_vault" => {
            Err(JsonRpcFailure::invalid_params(WRITE_DISABLED_MESSAGE))
        }
        other => Err(JsonRpcFailure::invalid_params(format!(
            "Unknown MCP tool: {other}"
        ))),
    };

    // Tool-level failures (e.g. "note not found") are rendered as an isError
    // tool result so read and write tools report the same conditions the same
    // way; genuine protocol errors stay JSON-RPC errors.
    match outcome {
        Err(failure) if failure.tool_level => match serde_json::from_str::<Value>(&failure.message)
        {
            Ok(error) => Ok(tool_structured_error(error)),
            Err(_) => Ok(tool_error(failure.message)),
        },
        other => other,
    }
}

fn select_model_tool(
    state: AppState,
    selected: crate::model_setup::SelectedModel,
    success: crate::mcp::results::ModelChoiceResult,
) -> Result<Value, JsonRpcFailure> {
    match crate::server::select_model_and_start(state, selected) {
        Ok(()) => Ok(tool_success(crate::mcp::results::result_to_value(&success))),
        Err(crate::server::ModelChoiceError::AlreadyActive) => Ok(tool_error(
            "A search model setup is already active. Changing models after setup begins is not supported."
                .to_string(),
        )),
        Err(crate::server::ModelChoiceError::Persist(error)) => {
            Err(JsonRpcFailure::internal(error))
        }
    }
}

/// Every read tool that answers about a Vault's content, and the one list that
/// says so. The top-level dispatcher above and the `batch` allow-list
/// (`batch.rs`) both read this rather than repeating the names, mirroring
/// [`write::WRITE_OPS`] on the mutation side.
///
/// `list_vaults` is deliberately absent — it is collection discovery, not
/// content — and so are the setup tools, which are unreachable once setup has
/// finished and have nothing to do with Note or attachment content.
pub(super) const READ_OPS: &[&str] = &[
    "search_notes",
    "get_note",
    "get_note_links",
    "resolve_wikilink",
    "get_tree",
    "get_stats",
    "get_graph",
    "recently_modified",
    "get_attachment_import_config",
    "list_note_attachments",
    "get_attachment",
    "get_frontmatter",
];

/// Dispatches one read op to its underlying tool function. Shared by the
/// top-level MCP dispatcher above (one call per request) and the `batch` tool
/// (one call per item), so the two can never drift on how a read is answered.
pub(super) async fn dispatch_read_tool(
    state: AppState,
    config: &McpConfig,
    op: &str,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    match op {
        "search_notes" => read::search_notes_tool(state, arguments).await,
        "get_note" => read::get_note_tool(state, arguments).await,
        "get_note_links" => read::get_note_links_tool(state, arguments).await,
        "resolve_wikilink" => read::resolve_wikilink_tool(state, arguments).await,
        "get_tree" => read::get_tree_tool(state, arguments).await,
        "get_stats" => read::get_stats_tool(state, arguments).await,
        "get_graph" => read::get_graph_tool(state, arguments).await,
        "recently_modified" => read::recently_modified_tool(state, arguments).await,
        // Not gated on `write_enabled`: the tool reports the write posture
        // rather than exercising it, and an agent that cannot upload still
        // needs to be told so, with the reason.
        "get_attachment_import_config" => {
            read::attachment_import_config_tool(state, config, arguments).await
        }
        // Reading which attachments a Note references is a read, and is
        // answered under the same permission as reading the Note itself. It
        // lived behind the write gate only because it was catalogued next to
        // the attachment mutations.
        "list_note_attachments" => read::list_note_attachments_tool(state, arguments).await,
        // Fetching an attachment's bytes is a read, like list_note_attachments;
        // it needs `config` for the base64 encoding's size cap.
        "get_attachment" => read::get_attachment_tool(state, arguments, config).await,
        // Reading a Note's frontmatter projection is a read, like reading the
        // Note itself.
        "get_frontmatter" => read::get_frontmatter_tool(state, arguments).await,
        // Unreachable while [`READ_OPS`] and the arms above agree, which
        // `read_ops_are_all_advertised` enforces. An error rather than a panic:
        // a name that drifts out of step must not take the process down.
        _ => Err(JsonRpcFailure::invalid_params(format!(
            "MCP tool is catalogued as a read tool but has no dispatch: {op}"
        ))),
    }
}

/// The typed `get_model_setup_status` answer. The data-notice text is part
/// of the operator-facing privacy promise and stays byte-identical to what
/// this tool has always reported.
fn model_setup_status_result(state: &AppState) -> crate::mcp::results::ModelSetupStatusResult {
    crate::mcp::results::ModelSetupStatusResult {
        state: state.startup.status(),
        gemma: crate::mcp::results::ModelSetupModelInfo {
            model: crate::model_setup::GEMMA_MODEL_ID,
            terms_url: Some(crate::model_setup::GEMMA_TERMS_URL),
            policy_url: Some(crate::model_setup::GEMMA_POLICY_URL),
            terms_version: crate::model_setup::GEMMA_TERMS_VERSION,
            repository: crate::model_setup::GEMMA_REPOSITORY,
            revision: crate::model_setup::GEMMA_REVISION,
        },
        fallback: crate::mcp::results::ModelSetupFallbackInfo {
            model: crate::model_setup::NOMIC_MODEL_ID,
            notice: "Nomic is the fallback if you decline Gemma. It supports English only and still provides solid search, but Gemma performed better in Hatchdoor's tests, including English searches. Nomic uses about 1.3 GB of RAM while indexing; Gemma uses about 0.5 GB.".to_string(),
        },
    }
}

pub fn tools_list(config: &McpConfig) -> Vec<Value> {
    let mut tools = read::read_tools_list();
    tools.push(batch::batch_tool_schema());
    if config.write_enabled {
        tools.extend(read::management_tools_list());
        tools.extend(write::write_tools_list());
    }
    with_output_schemas(tools)
}

/// Attaches each advertised tool's `outputSchema`, generated from the same
/// typed result structure its responses serialize from (`src/mcp/results.rs`).
/// Every tool in the catalogue must have one; a missing entry fails the
/// catalogue build rather than advertising a schemaless tool.
fn with_output_schemas(mut tools: Vec<Value>) -> Vec<Value> {
    for tool in &mut tools {
        let name = tool["name"]
            .as_str()
            .expect("every advertised MCP tool has a name");
        let schema = crate::mcp::results::output_schema_for(name)
            .unwrap_or_else(|| panic!("{name} advertises no output schema"));
        tool["outputSchema"] = serde_json::to_value(&schema).expect("schema serializes");
    }
    tools
}

/// Setup tools are always advertised alongside the vault tools so clients that
/// cache their tool list on connection can complete first-run setup and then use
/// the vault without reconnecting.
pub fn setup_tools_list() -> Vec<Value> {
    with_output_schemas(vec![
        json!({
            "name": "get_model_setup_status",
            "description": "Show Hatchdoor's first-run embedding model setup status, Gemma terms links, and the local-data privacy notice.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": read_only_tool_annotations(),
        }),
        json!({
            "name": "accept_gemma_terms",
            "description": "Accept the Gemma terms for this local Hatchdoor instance, then download the multilingual default model and begin indexing. The acceptance record stays local and does not change ownership of vault data.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": write_tool_annotations(false, true),
        }),
        json!({
            "name": "decline_gemma_terms",
            "description": "Decline Gemma terms, remove any Gemma download/cache, then download Nomic Embed Text v1.5 and begin indexing. Nomic supports English only. It still provides solid search, but Gemma performed better in Hatchdoor's tests, including English searches, and uses less RAM while indexing.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": write_tool_annotations(true, true),
        }),
    ])
}

/// Vault collection discovery/management tools mirror `handlers/vaults.rs`'s
/// `/api/v1/vaults` surface, which stays reachable at zero enabled Vaults or a
/// registry needing recovery. `config.write_enabled` still gates the
/// mutating ones exactly as it does when the legacy readiness gate is not the
/// blocker in play.
fn is_collection_management_tool(name: &str) -> bool {
    matches!(
        name,
        "list_vaults"
            | "create_vault"
            | "edit_vault"
            | "enable_vault"
            | "disable_vault"
            | "disconnect_vault"
            | "sync_vault"
            | "retry_vault"
    )
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

pub(super) fn write_tool_annotations(destructive: bool, idempotent: bool) -> Value {
    json!({
        "readOnlyHint": false,
        "destructiveHint": destructive,
        "idempotentHint": idempotent,
        "openWorldHint": false,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tempfile::TempDir;

    use super::*;
    use crate::embed::{Embedder, StubEmbedder};
    use crate::startup::StartupTracker;

    /// The lifecycle test only exercises model-setup claiming, so no Vault is
    /// registered, and nothing under test reads a Vault snapshot.
    fn setup_state_with_claimed_lifecycle() -> (AppState, TempDir) {
        let tmp = TempDir::new().expect("temp dir");
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
        let (mcp_tools_changed, _) = tokio::sync::broadcast::channel(16);
        let (vault_work, _vault_worker) = crate::vault_work::VaultWorkCoordinator::new();
        let managed_git = Arc::new(crate::git::ManagedGitScheduler::without_durable_state(
            vault_work.clone(),
        ));
        let state = AppState {
            vault_registry: crate::vault_registry::VaultRegistryStore::new(
                tmp.path().join("state/vaults.json"),
            ),
            vaults: crate::vault_runtime::VaultCollectionRuntime::new(),
            vault_work,
            managed_git,
            legacy_migration_recovery: Arc::new(std::sync::RwLock::new(None)),
            startup_sqlite: Arc::new(
                crate::cache::SqliteCache::in_memory(384).expect("in-memory cache"),
            ),
            mcp_tools_changed,
            embedder,
            runtime_embedder: Arc::new(crate::embed::RuntimeEmbedder::new()),
            model_setup: Arc::new(crate::model_setup::ModelSetup::new(
                tmp.path().join("models"),
            )),
            model_setup_started: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            web_auth_enabled: false,
            demo_mode: false,
            runtime_config: crate::runtime_config::RuntimeConfig::for_tests(),
            startup: StartupTracker::terms_required(),
        };
        (state, tmp)
    }

    #[tokio::test]
    async fn claimed_model_setup_refuses_mcp_choice_without_persisting_it() {
        let (state, _tmp) = setup_state_with_claimed_lifecycle();

        let outcome = handle_tools_call(
            state.clone(),
            Some(json!({ "name": "decline_gemma_terms", "arguments": {} })),
            &McpConfig::disabled(),
        )
        .await
        .expect("tool result");

        assert_eq!(outcome["isError"], true);
        assert_eq!(
            state
                .model_setup
                .selected()
                .expect("selection after refusal"),
            crate::model_setup::SelectedModel::TermsRequired,
            "a lost MCP lifecycle claim must not persist a choice the runtime did not adopt"
        );
    }
}
