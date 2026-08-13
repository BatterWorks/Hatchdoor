//! Vault-scoped MCP read tools.  These are deliberately thin in-process
//! adapters over the same V1 handlers and shared cores used by HTTP: MCP owns
//! JSON-RPC framing, while scope parsing, projections, and error shapes stay
//! in the Vault API surface.

use axum::body::to_bytes;
use axum::extract::{Path, Query, State};
use axum::response::Response;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::app_state::AppState;
use crate::handlers::{vault_collection_reads, vault_content, vaults};

use super::super::protocol::{JsonRpcFailure, tool_structured_error, tool_success};
use super::read_only_tool_annotations;

const MAX_TOOL_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

pub(super) async fn handler_payload(response: Response) -> Result<Value, JsonRpcFailure> {
    let status = response.status();
    let bytes = to_bytes(response.into_body(), MAX_TOOL_RESPONSE_BYTES)
        .await
        .map_err(|error| JsonRpcFailure::internal(format!("read Vault response body: {error}")))?;
    let payload = serde_json::from_slice(&bytes).map_err(|error| {
        JsonRpcFailure::internal(format!("decode Vault response body: {error}"))
    })?;
    Ok(if status.is_success() {
        tool_success(payload)
    } else {
        tool_structured_error(payload)
    })
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
    https_credentials: Option<crate::handlers::vaults::HttpsCredentialsPatch>,
    #[serde(default)]
    confirm_identity_change: bool,
    #[serde(default)]
    archive_folder: Option<String>,
    #[serde(default)]
    commit_identity: Option<crate::vault_registry::VaultCommitIdentity>,
}

pub(super) async fn list_vaults_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let _: EmptyArgs = parse("list_vaults", arguments)?;
    handler_payload(vaults::list_vaults_handler(State(state)).await).await
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
    handler_payload(
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
    handler_payload(
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
    handler_payload(
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
    handler_payload(
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
    handler_payload(
        vault_collection_reads::vault_scope_tree_handler(State(state), Path(args.scope)).await,
    )
    .await
}

pub(super) async fn get_stats_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: ScopeArgs = parse("get_stats", arguments)?;
    handler_payload(
        vault_collection_reads::vault_scope_stats_handler(State(state), Path(args.scope)).await,
    )
    .await
}

pub(super) async fn get_graph_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: ScopeArgs = parse("get_graph", arguments)?;
    handler_payload(
        vault_collection_reads::vault_scope_graph_handler(State(state), Path(args.scope)).await,
    )
    .await
}

pub(super) async fn recently_modified_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: RecentArgs = parse("recently_modified", arguments)?;
    handler_payload(
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

/// Registry writes deliberately call the same revisioned collection handlers
/// as HTTP.  The create operation is the sole control exception without a
/// `vault_id`: the shared registry generates the immutable ID atomically when
/// the expected registry revision commits; every control of an existing Vault
/// takes exactly one `vault_id`.
pub(super) async fn create_vault_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let request: vaults::CreateVaultRequest = parse("create_vault", arguments)?;
    handler_payload(vaults::create_vault_handler(State(state), Ok(axum::Json(request))).await).await
}

pub(super) async fn edit_vault_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: EditVaultArgs = parse("edit_vault", arguments)?;
    let request = vaults::EditVaultRequest {
        expected_registry_revision: args.expected_registry_revision,
        name: args.name,
        source: args.source,
        exclude_patterns: args.exclude_patterns,
        https_credentials: args
            .https_credentials
            .unwrap_or(vaults::HttpsCredentialsPatch::Keep),
        confirm_identity_change: args.confirm_identity_change,
        archive_folder: args.archive_folder,
        commit_identity: args.commit_identity,
    };
    handler_payload(
        vaults::edit_vault_handler(State(state), Path(args.vault_id), Ok(axum::Json(request)))
            .await,
    )
    .await
}

pub(super) async fn enable_vault_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: VaultControlArgs = parse("enable_vault", arguments)?;
    handler_payload(
        vaults::enable_vault_handler(
            State(state),
            Path(args.vault_id),
            Ok(Query(vaults::RevisionQuery {
                expected_registry_revision: args.expected_registry_revision,
            })),
        )
        .await,
    )
    .await
}

pub(super) async fn disable_vault_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: VaultControlArgs = parse("disable_vault", arguments)?;
    handler_payload(
        vaults::disable_vault_handler(
            State(state),
            Path(args.vault_id),
            Ok(Query(vaults::RevisionQuery {
                expected_registry_revision: args.expected_registry_revision,
            })),
        )
        .await,
    )
    .await
}

pub(super) async fn disconnect_vault_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: VaultControlArgs = parse("disconnect_vault", arguments)?;
    handler_payload(
        vaults::disconnect_vault_handler(
            State(state),
            Path(args.vault_id),
            Ok(Query(vaults::RevisionQuery {
                expected_registry_revision: args.expected_registry_revision,
            })),
        )
        .await,
    )
    .await
}

pub(super) async fn sync_vault_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: VaultIdArgs = parse("sync_vault", arguments)?;
    handler_payload(vaults::sync_vault_handler(State(state), Path(args.vault_id)).await).await
}

pub(super) async fn retry_vault_tool(
    state: AppState,
    arguments: Value,
) -> Result<Value, JsonRpcFailure> {
    let args: VaultIdArgs = parse("retry_vault", arguments)?;
    handler_payload(vaults::retry_vault_handler(State(state), Path(args.vault_id)).await).await
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
        json!({"name":"create_vault","description":"Create a Vault definition with the shared revisioned collection contract. The registry assigns its immutable Vault ID after a successful create; use list_vaults to discover it.","inputSchema":{"type":"object","properties":{"expected_registry_revision":{"type":"integer","minimum":0},"name":{"type":"string","minLength":1},"enabled":{"type":"boolean","default":true},"source":{"type":"object","description":"A shared VaultSource object: local, existing_git, or managed_git."},"exclude_patterns":{"type":"array","items":{"type":"string"},"default":[]},"https_credentials":{"type":"object","description":"Optional HTTPS token input. username is optional; a documented fixed placeholder is used when omitted. It is redacted from every response."},"archive_folder":{"type":"string","description":"Optional per-Vault archive folder. Absent means the instance-wide default applies."},"commit_identity":{"type":"object","description":"Optional per-Vault commit author identity: {name, email}. Absent means the instance-wide default applies.","properties":{"name":{"type":"string","minLength":1},"email":{"type":"string","minLength":1}},"required":["name","email"],"additionalProperties":false}},"required":["expected_registry_revision","name","source"],"additionalProperties":false},"annotations":super::write_tool_annotations(true, false)}),
        json!({"name":"edit_vault","description":"Edit exactly one Vault definition with optimistic registry revision control.","inputSchema":{"type":"object","properties":{"vault_id":vault_id_schema(),"expected_registry_revision":{"type":"integer","minimum":0},"name":{"type":"string","minLength":1},"source":{"type":"object","description":"A shared VaultSource object: local, existing_git, or managed_git."},"exclude_patterns":{"type":"array","items":{"type":"string"},"default":[]},"https_credentials":{"type":"object","description":"A shared credentials patch: {action: keep|remove|replace}; replace's username is optional (a documented fixed placeholder is used when omitted); replacement input is never echoed."},"confirm_identity_change":{"type":"boolean","default":false},"archive_folder":{"type":"string","description":"Optional per-Vault archive folder. Absent means the instance-wide default applies."},"commit_identity":{"type":"object","description":"Optional per-Vault commit author identity: {name, email}. Absent means the instance-wide default applies.","properties":{"name":{"type":"string","minLength":1},"email":{"type":"string","minLength":1}},"required":["name","email"],"additionalProperties":false}},"required":["vault_id","expected_registry_revision","name","source"],"additionalProperties":false},"annotations":super::write_tool_annotations(true, false)}),
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
        for name in ["get_note", "get_note_links", "resolve_wikilink"] {
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
    /// parses straight into `vaults::CreateVaultRequest`, which embeds the
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
        let request: vaults::CreateVaultRequest = serde_json::from_value(value)
            .expect("create_vault must accept poll_interval_secs on an existing_git source");
        assert!(matches!(
            request.source,
            crate::vault_registry::VaultSource::ExistingGit {
                poll_interval_secs: 60,
                ..
            }
        ));
    }
}
