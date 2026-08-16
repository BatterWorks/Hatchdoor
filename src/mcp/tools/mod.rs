//! MCP tool surface. Dispatch and the shared helpers live here; the tool
//! implementations and their JSON schemas are split by permission boundary
//! into `read` (always available) and `write` (gated by
//! `HATCHDOOR_MCP_WRITE_ENABLED`), mirroring how `McpConfig` gates them.

mod read;
mod write;

use serde_json::{Value, json};

use super::config::McpConfig;
use super::protocol::{JsonRpcFailure, tool_error, tool_structured_error, tool_success};
use crate::app_state::AppState;

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
        return Ok(tool_success(model_setup_status_payload(&state)));
    }

    // Before the first index exists, only the explicit model-setup calls and
    // Vault collection discovery/management may run. `state.startup` tracks
    // the legacy single-Vault embedding-model setup, which has no bearing on
    // the Vault registry: zero Vaults or a registry in Recovery are normal,
    // expected states, and an agent must be able to see and repair the
    // collection precisely then. This mirrors `handlers/vaults.rs`, whose
    // whole HTTP surface is deliberately not gated by this legacy readiness
    // signal. The full tool catalogue is still advertised so MCP clients that
    // cache tools at connection time need no restart once setup completes.
    if !state.startup.is_ready() && !is_collection_management_tool(name) {
        return match name {
            "accept_gemma_terms" => select_model_tool(
                state,
                crate::model_setup::SelectedModel::Gemma,
                json!({ "accepted": true, "model": crate::model_setup::GEMMA_MODEL_ID }),
            ),
            "decline_gemma_terms" => select_model_tool(
                state,
                crate::model_setup::SelectedModel::Nomic,
                json!({ "accepted": false, "model": crate::model_setup::NOMIC_MODEL_ID }),
            ),
            _ => Ok(tool_error(
                "Hatchdoor is still being set up. Use get_model_setup_status, accept_gemma_terms, or decline_gemma_terms first.".to_string(),
            )),
        };
    }

    if matches!(name, "accept_gemma_terms" | "decline_gemma_terms") {
        return Ok(tool_error(
            "A search model is already set up. Changing models after setup is not supported."
                .to_string(),
        ));
    }

    let outcome = match name {
        "list_vaults" => read::list_vaults_tool(state, arguments).await,
        "search_notes" => read::search_notes_tool(state, arguments).await,
        "get_note" => read::get_note_tool(state, arguments).await,
        "get_note_links" => read::get_note_links_tool(state, arguments).await,
        "resolve_wikilink" => read::resolve_wikilink_tool(state, arguments).await,
        "get_tree" => read::get_tree_tool(state, arguments).await,
        "get_stats" => read::get_stats_tool(state, arguments).await,
        "get_graph" => read::get_graph_tool(state, arguments).await,
        "recently_modified" => read::recently_modified_tool(state, arguments).await,
        "create_vault" if config.write_enabled => read::create_vault_tool(state, arguments).await,
        "edit_vault" if config.write_enabled => read::edit_vault_tool(state, arguments).await,
        "enable_vault" if config.write_enabled => read::enable_vault_tool(state, arguments).await,
        "disable_vault" if config.write_enabled => read::disable_vault_tool(state, arguments).await,
        "disconnect_vault" if config.write_enabled => {
            read::disconnect_vault_tool(state, arguments).await
        }
        "sync_vault" if config.write_enabled => read::sync_vault_tool(state, arguments).await,
        "retry_vault" if config.write_enabled => read::retry_vault_tool(state, arguments).await,
        "create_note" | "update_note" | "append_to_note" | "edit_note" | "replace_section"
        | "rename_note" | "move_note" | "move_rename_note" | "archive_note" | "delete_note"
        | "import_attachment" | "move_attachment" | "rename_attachment" | "delete_attachment"
            if config.write_enabled =>
        {
            let vault = write::scoped_vault(&state, &arguments)?;
            // This is the same per-Vault mutation lock used by the V1 HTTP
            // adapter.  The legacy instance-wide AppState lock deliberately
            // does not participate in this scoped path.
            let _guard = write::acquire_mutation(&vault).await?;
            match name {
                "create_note" => write::create_note_tool(state, &vault, arguments).await,
                "update_note" => write::update_note_tool(state, &vault, arguments).await,
                "append_to_note" => write::append_to_note_tool(state, &vault, arguments).await,
                "edit_note" => write::edit_note_tool(state, &vault, arguments).await,
                "replace_section" => write::replace_section_tool(state, &vault, arguments).await,
                "rename_note" => write::rename_note_tool(state, &vault, arguments).await,
                "move_note" => write::move_note_tool(state, &vault, arguments).await,
                "move_rename_note" => write::move_rename_note_tool(state, &vault, arguments).await,
                "archive_note" => write::archive_note_tool(state, &vault, arguments).await,
                "delete_note" => write::delete_note_tool(state, &vault, arguments).await,
                "import_attachment" => {
                    write::import_attachment_tool(state, &vault, arguments, config).await
                }
                "move_attachment" => write::move_attachment_tool(state, &vault, arguments).await,
                "rename_attachment" => {
                    write::rename_attachment_tool(state, &vault, arguments).await
                }
                "delete_attachment" => {
                    write::delete_attachment_tool(state, &vault, arguments).await
                }
                _ => unreachable!(),
            }
        }
        "list_note_attachments" if config.write_enabled => {
            let vault = write::scoped_vault(&state, &arguments)?;
            write::list_note_attachments_tool(state, &vault, arguments).await
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
        "create_vault" | "edit_vault" | "enable_vault" | "disable_vault" | "disconnect_vault"
        | "sync_vault" | "retry_vault" => Err(JsonRpcFailure::invalid_params(
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
    success: Value,
) -> Result<Value, JsonRpcFailure> {
    match crate::server::select_model_and_start(state, selected) {
        Ok(()) => Ok(tool_success(success)),
        Err(crate::server::ModelChoiceError::AlreadyActive) => Ok(tool_error(
            "A search model setup is already active. Changing models after setup begins is not supported."
                .to_string(),
        )),
        Err(crate::server::ModelChoiceError::Persist(error)) => {
            Err(JsonRpcFailure::internal(error))
        }
    }
}

fn model_setup_status_payload(state: &AppState) -> Value {
    json!({
        "state": state.startup.status(),
        "gemma": {
            "model": crate::model_setup::GEMMA_MODEL_ID,
            "terms_url": crate::model_setup::GEMMA_TERMS_URL,
            "policy_url": crate::model_setup::GEMMA_POLICY_URL,
            "terms_version": crate::model_setup::GEMMA_TERMS_VERSION,
            "repository": crate::model_setup::GEMMA_REPOSITORY,
            "revision": crate::model_setup::GEMMA_REVISION,
            "data_notice": "Accepting the terms does not change ownership of your vault data. The acceptance record stays on this machine and is not sent anywhere."
        },
        "fallback": {
            "model": crate::model_setup::NOMIC_MODEL_ID,
            "notice": "Nomic is the fallback if you decline Gemma. It supports English only and still provides solid search, but Gemma performed better in Hatchdoor's tests, including English searches. Nomic uses about 1.3 GB of RAM while indexing; Gemma uses about 0.5 GB."
        }
    })
}

pub fn tools_list(config: &McpConfig) -> Vec<Value> {
    let mut tools = read::read_tools_list();
    if config.write_enabled {
        tools.extend(read::management_tools_list());
        tools.extend(write::write_tools_list());
    }
    tools
}

/// Setup tools are always advertised alongside the vault tools so clients that
/// cache their tool list on connection can complete first-run setup and then use
/// the vault without reconnecting.
pub fn setup_tools_list() -> Vec<Value> {
    vec![
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
    ]
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
    use tokio::sync::RwLock;

    use super::*;
    use crate::embed::{Embedder, StubEmbedder};
    use crate::startup::StartupTracker;

    /// The lifecycle test only exercises model-setup claiming, so no Vault is
    /// registered: `ready_vault` stays `None` and nothing under test reads it.
    fn setup_state_with_claimed_lifecycle() -> (AppState, TempDir) {
        let tmp = TempDir::new().expect("temp dir");
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
        let (vault_events, _) = tokio::sync::broadcast::channel(64);
        let (mcp_tools_changed, _) = tokio::sync::broadcast::channel(16);
        let (vault_work, _vault_worker) = crate::vault_work::VaultWorkCoordinator::new();
        let managed_git = Arc::new(crate::git::ManagedGitScheduler::new(vault_work.clone()));
        let state = AppState {
            cache_db_path: tmp.path().join("cache.sqlite3"),
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
            ready_vault: Arc::new(RwLock::new(None)),
            vault_revision: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            vault_events,
            mcp_tools_changed,
            embedder,
            runtime_embedder: Arc::new(crate::embed::RuntimeEmbedder::new()),
            model_setup: Arc::new(crate::model_setup::ModelSetup::new(
                tmp.path().join("models"),
            )),
            model_setup_started: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            startup_git_config: Arc::new(None),
            web_auth_enabled: false,
            demo_mode: false,
            vault_write_lock: Arc::new(tokio::sync::Mutex::new(())),
            git_sync: Arc::new(RwLock::new(None)),
            scan_config_cache: Arc::new(std::sync::RwLock::new(None)),
            refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
            index_status: crate::app_state::IndexStatusTracker::up_to_date(),
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
