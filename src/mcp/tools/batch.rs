//! The `batch` MCP tool: executes a caller-supplied ordered list of note and
//! attachment operations in one call. Gated by `HATCHDOOR_MCP_WRITE_ENABLED`
//! only for the write-shaped items it contains — a read-only batch runs
//! whatever the instance's write posture.
//!
//! Every item delegates to the exact same `read`/`write` tool functions a
//! standalone call would use, so a batch behaves like N sequential single-tool
//! calls except for three deliberate relaxations: one round trip, best-effort
//! continuation past a failing item, and (issue #177) `expected_content_hash`
//! chaining between items in the same call that touch the same note — see
//! [`apply_hash_chain`].
//!
//! Chaining trusts this batch's own prior write, not the caller's own value,
//! so it must never be checked against a Vault an external writer could have
//! touched in between: [`batch_tool`] acquires each touched Vault's mutation
//! lock once and holds it for the rest of the call, rather than per item like
//! a standalone write does, closing that window instead of narrowing it.
//!
//! Note what the caller pays for that: while a batch runs, every other writer
//! to a Vault it has already written — the Web UI, the V1 HTTP adapter, another
//! MCP call — waits. A batch is capped at [`BATCH_MAX_WRITE_ITEMS`] writes for
//! this reason as much as for load.
//!
//! **One commit per batch** (#177) falls out of that same lock rather than any
//! Git handling here: this module writes Markdown exactly as the standalone
//! tools do, and a Vault's sync turn takes the same mutation lock, so no turn
//! can interleave with a batch and split it across commits. The turn then finds
//! every one of the batch's writes dirty together and commits them as one, which
//! `git::sync::tests::one_turn_commits_a_whole_batch_of_writes_as_a_single_commit`
//! asserts. The per-Vault turn is now the only synchronisation mechanism
//! (ADR-18, #185), so this holds for every synced Vault; the legacy
//! single-Vault sync path that could split a batch across commits no longer
//! exists.

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::app_state::AppState;
use crate::vault_registry::VaultId;

use super::super::config::McpConfig;
use super::super::limits::{BATCH_MAX_READ_ITEMS, BATCH_MAX_WRITE_ITEMS};
use super::super::protocol::{JsonRpcFailure, tool_success};
use super::super::results::{BatchItemResult, BatchResult, result_to_value};
use super::write::WRITE_OPS;
use super::{READ_OPS, WRITE_DISABLED_MESSAGE, dispatch_read_tool, write, write_tool_annotations};

/// The write ops that carry both `slug` and `expected_content_hash` — the
/// only ones eligible for within-batch hash chaining. `create_note` and the
/// attachment tools take no `expected_content_hash` and are never chained
/// into.
const HASH_CHAINED_OPS: &[&str] = &[
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
];

/// `(vault_id, slug) -> content_hash`, tracking each note's most recent
/// resulting hash from an earlier item in this same batch call. Keyed by the
/// raw `vault_id` string rather than a parsed `VaultId`: this is pure
/// in-batch bookkeeping, never used to resolve or authorize a Vault (every
/// dispatch still parses and validates `vault_id` itself), and `VaultId`
/// carries no `Hash` impl to key a map with.
type HashChain = HashMap<(String, String), String>;

/// One mutation guard per Vault this batch call has touched, held from the
/// first write item against that Vault through the end of the whole call — see
/// the module doc comment for why. A `Vec` rather than a map: `VaultId` has no
/// `Hash` impl, and a batch touches at most a handful of distinct Vaults, so a
/// linear scan against `BATCH_MAX_WRITE_ITEMS` (20) entries is cheap.
type VaultLocks = Vec<(VaultId, tokio::sync::OwnedMutexGuard<()>)>;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BatchArgs {
    operations: Vec<BatchOperation>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BatchOperation {
    op: String,
    arguments: Value,
}

pub(super) async fn batch_tool(
    state: AppState,
    arguments: Value,
    config: &McpConfig,
) -> Result<Value, JsonRpcFailure> {
    let args: BatchArgs = serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!("Invalid batch arguments: {error}"))
    })?;
    if args.operations.is_empty() {
        return Err(JsonRpcFailure::invalid_params(
            "batch operations cannot be empty",
        ));
    }

    let mut read_count = 0usize;
    let mut write_count = 0usize;
    for (index, item) in args.operations.iter().enumerate() {
        if READ_OPS.contains(&item.op.as_str()) {
            read_count += 1;
        } else if WRITE_OPS.contains(&item.op.as_str()) {
            write_count += 1;
        } else {
            return Err(JsonRpcFailure::invalid_params(format!(
                "batch item {index}: op '{}' is not a valid batch operation; vault-management \
                 tools and unknown tools are not allowed inside batch",
                item.op
            )));
        }
    }
    if read_count > BATCH_MAX_READ_ITEMS {
        return Err(JsonRpcFailure::invalid_params(format!(
            "batch contains {read_count} read-shaped items, exceeding the limit of {BATCH_MAX_READ_ITEMS}"
        )));
    }
    if write_count > BATCH_MAX_WRITE_ITEMS {
        return Err(JsonRpcFailure::invalid_params(format!(
            "batch contains {write_count} write-shaped items, exceeding the limit of {BATCH_MAX_WRITE_ITEMS}"
        )));
    }

    let mut chain: HashChain = HashMap::new();
    // Held across the whole loop below (dropped only when `batch_tool`
    // returns): once a Vault has been written to by this batch, its mutation
    // lock stays held until the call finishes, so nothing outside this call
    // can land a write that a later chained item's substituted hash would
    // then silently overwrite.
    let mut locks: VaultLocks = Vec::new();
    let mut items = Vec::with_capacity(args.operations.len());
    let mut succeeded = 0usize;
    let mut failed = 0usize;

    for (index, item) in args.operations.into_iter().enumerate() {
        let BatchOperation { op, arguments } = item;
        let arguments = apply_hash_chain(&op, arguments, &chain);
        match dispatch_one(state.clone(), config, &op, arguments, &mut locks).await {
            Ok(value) => {
                if value.get("isError").and_then(Value::as_bool) == Some(true) {
                    failed += 1;
                    let error = value
                        .get("structuredContent")
                        .cloned()
                        .unwrap_or_else(|| json!({ "message": value["content"][0]["text"] }));
                    items.push(BatchItemResult {
                        index,
                        op,
                        ok: false,
                        result: None,
                        error: Some(error),
                    });
                } else {
                    succeeded += 1;
                    let result = value.get("structuredContent").cloned();
                    if let Some(result) = &result {
                        record_chain(&mut chain, &op, result);
                    }
                    items.push(BatchItemResult {
                        index,
                        op,
                        ok: true,
                        result,
                        error: None,
                    });
                }
            }
            Err(failure) => {
                failed += 1;
                items.push(BatchItemResult {
                    index,
                    op,
                    ok: false,
                    result: None,
                    error: Some(failure_to_error_value(failure)),
                });
            }
        }
    }

    Ok(tool_success(result_to_value(&BatchResult {
        items,
        succeeded,
        failed,
    })))
}

/// Dispatches one batch item to the same tool function a standalone call to
/// `op` would use. Mirrors `mod.rs`'s own dispatch match, restricted to the
/// note/attachment allowlist above. `locks` carries every Vault mutation
/// guard this batch call has acquired so far — see [`VaultLocks`].
async fn dispatch_one(
    state: AppState,
    config: &McpConfig,
    op: &str,
    arguments: Value,
    locks: &mut VaultLocks,
) -> Result<Value, JsonRpcFailure> {
    match op {
        _ if READ_OPS.contains(&op) => dispatch_read_tool(state, config, op, arguments).await,
        _ if WRITE_OPS.contains(&op) => {
            if !config.write_enabled {
                return Err(JsonRpcFailure::invalid_params(WRITE_DISABLED_MESSAGE));
            }
            let vault = write::scoped_vault(&state, &arguments)?;
            // Acquire this Vault's mutation lock only the first time the
            // batch touches it, and hold the guard in `locks` for the rest
            // of the call rather than dropping it at the end of this item —
            // see the module doc comment. `tokio::sync::Mutex` is not
            // reentrant, so re-acquiring an already-held guard here would
            // deadlock; the linear scan below is what prevents that.
            if !locks.iter().any(|(id, _)| *id == vault.vault_id) {
                locks.push((vault.vault_id, write::acquire_mutation(&vault).await?));
            }
            write::dispatch_write_tool(state, &vault, op, arguments, config).await
        }
        _ => Err(JsonRpcFailure::invalid_params(format!(
            "batch op '{op}' is not a valid batch operation"
        ))),
    }
}

/// Before dispatch, substitutes the tracked in-batch hash for a hash-chained
/// op's `expected_content_hash` when this batch call has already written the
/// same `(vault_id, slug)` — silently discarding whatever the caller supplied
/// for that field, since they cannot know the intermediate hash without an
/// extra round trip. A note untouched earlier in this batch keeps the
/// caller's own value and validates normally, exactly like a standalone call.
fn apply_hash_chain(op: &str, mut arguments: Value, chain: &HashChain) -> Value {
    if !HASH_CHAINED_OPS.contains(&op) {
        return arguments;
    }
    let key = arguments
        .get("vault_id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .zip(
            arguments
                .get("slug")
                .and_then(Value::as_str)
                .map(str::to_string),
        );
    if let Some(key) = key
        && let Some(hash) = chain.get(&key)
        && let Some(object) = arguments.as_object_mut()
    {
        object.insert("expected_content_hash".to_string(), json!(hash));
    }
    arguments
}

/// After a successful note write, records its resulting `(vault_id, slug) ->
/// content_hash` so a later item in this batch can chain off it —
/// `create_note` included, so an item can create a note and edit it later in
/// the same call without an intermediate read. A delete's null `content_hash`
/// is never recorded: nothing left to chain into.
fn record_chain(chain: &mut HashChain, op: &str, result: &Value) {
    if !(HASH_CHAINED_OPS.contains(&op) || op == "create_note") {
        return;
    }
    let vault_id = result.get("vault_id").and_then(Value::as_str);
    let slug = result.get("slug").and_then(Value::as_str);
    let hash = result.get("content_hash").and_then(Value::as_str);
    if let (Some(vault_id), Some(slug), Some(hash)) = (vault_id, slug, hash) {
        chain.insert((vault_id.to_string(), slug.to_string()), hash.to_string());
    }
}

/// Renders a per-item dispatch failure the same way the top-level dispatcher
/// renders a tool-level one (`mod.rs`'s own tail): a JSON-object message
/// decodes to the structured domain error it already is, and a plain-text
/// message (an invalid-params rejection, say) falls back to a `{code,
/// message}` pair carrying the JSON-RPC error code.
fn failure_to_error_value(failure: JsonRpcFailure) -> Value {
    match serde_json::from_str::<Value>(&failure.message) {
        Ok(structured) => structured,
        Err(_) => json!({ "code": failure.code, "message": failure.message }),
    }
}

pub(super) fn batch_tool_schema() -> Value {
    json!({
        "name": "batch",
        "description": "Execute an ordered list of note and attachment operations in one call — the same tools available standalone (create_note through delete_attachment, and every read tool except list_vaults). Vault-management tools (create_vault, edit_vault, enable_vault, disable_vault, disconnect_vault, sync_vault, retry_vault, list_vaults) are not allowed inside a batch; those and any unrecognized op are rejected before anything executes. Execution is in order and best-effort: each item reports its own ok/result/error, one item failing does not stop the rest, and there is no rollback or mid-batch visibility between items. All resulting Vault changes are committed together on the Vault's next Git sync turn, the same as any other burst of writes. expected_content_hash checks are skipped between items that share a vault_id and slug: create or edit a note earlier in this batch, then reference it again later in the same call without knowing the intermediate hash; a note not otherwise touched in this batch still validates its expected_content_hash normally. A batch may contain at most 50 read-shaped items and 20 write-shaped items.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "operations": {
                    "type": "array",
                    "minItems": 1,
                    "description": "Ordered operations to execute in this batch call.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "op": {
                                "type": "string",
                                "minLength": 1,
                                "description": "One of the note or attachment tool names, e.g. create_note, update_note, get_note, delete_attachment."
                            },
                            "arguments": {
                                "type": "object",
                                "description": "That tool's own arguments exactly as it is called standalone, including vault_id."
                            }
                        },
                        "required": ["op", "arguments"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["operations"],
            "additionalProperties": false
        },
        "annotations": write_tool_annotations(true, false)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_and_write_op_sets_are_disjoint_and_exclude_vault_management() {
        for op in READ_OPS {
            assert!(
                !WRITE_OPS.contains(op),
                "{op} listed in both READ_OPS and WRITE_OPS"
            );
        }
        for excluded in [
            "list_vaults",
            "create_vault",
            "edit_vault",
            "enable_vault",
            "disable_vault",
            "disconnect_vault",
            "sync_vault",
            "retry_vault",
            "get_model_setup_status",
            "accept_gemma_terms",
            "decline_gemma_terms",
            "batch",
        ] {
            assert!(
                !READ_OPS.contains(&excluded) && !WRITE_OPS.contains(&excluded),
                "{excluded} must not be an allowed batch op"
            );
        }
    }

    #[test]
    fn every_batch_op_is_an_advertised_tool() {
        // `READ_OPS` and `WRITE_OPS` gate what a batch may name. A name in
        // either that the catalogue does not advertise would be a batch-only
        // tool no client could discover; one that dispatch cannot answer would
        // be advertised and then refused. Both are drift, and this is where it
        // fails.
        let config = McpConfig {
            enabled: true,
            write_enabled: true,
            max_attachment_bytes: 0,
            max_base64_bytes: 0,
            bearer_token: None,
            allowed_origins: Vec::new(),
            rate_limits_enabled: true,
        };
        let advertised: Vec<String> = super::super::tools_list(&config)
            .iter()
            .map(|tool| tool["name"].as_str().expect("tool name").to_string())
            .collect();
        for op in READ_OPS.iter().chain(WRITE_OPS.iter()) {
            assert!(
                advertised.contains(&(*op).to_string()),
                "{op} is an allowed batch op but is not advertised in tools/list"
            );
        }
    }

    #[test]
    fn hash_chained_ops_are_a_subset_of_write_ops_without_create_or_attachments() {
        for op in HASH_CHAINED_OPS {
            assert!(WRITE_OPS.contains(op), "{op} must also be a write op");
        }
        assert!(!HASH_CHAINED_OPS.contains(&"create_note"));
        assert!(!HASH_CHAINED_OPS.contains(&"import_attachment"));
        assert!(!HASH_CHAINED_OPS.contains(&"move_attachment"));
    }

    #[test]
    fn apply_hash_chain_overrides_only_a_tracked_hit() {
        let mut chain = HashChain::new();
        chain.insert(
            ("vault-a".to_string(), "home".to_string()),
            "fnv1a64:new".to_string(),
        );

        let overridden = apply_hash_chain(
            "update_note",
            json!({"vault_id": "vault-a", "slug": "home", "expected_content_hash": "stale", "content": "x"}),
            &chain,
        );
        assert_eq!(overridden["expected_content_hash"], "fnv1a64:new");

        let untouched = apply_hash_chain(
            "update_note",
            json!({"vault_id": "vault-a", "slug": "other", "expected_content_hash": "caller-supplied", "content": "x"}),
            &chain,
        );
        assert_eq!(untouched["expected_content_hash"], "caller-supplied");

        // create_note carries no expected_content_hash and is not chained into.
        let create = apply_hash_chain(
            "create_note",
            json!({"vault_id": "vault-a", "relative_path": "New.md", "content": "x"}),
            &chain,
        );
        assert!(create.get("expected_content_hash").is_none());
    }

    #[test]
    fn record_chain_tracks_writes_and_skips_deletes() {
        let mut chain = HashChain::new();
        record_chain(
            &mut chain,
            "create_note",
            &json!({"vault_id": "vault-a", "ok": true, "slug": "new", "content_hash": "fnv1a64:1"}),
        );
        assert_eq!(
            chain.get(&("vault-a".to_string(), "new".to_string())),
            Some(&"fnv1a64:1".to_string())
        );

        record_chain(
            &mut chain,
            "delete_note",
            &json!({"vault_id": "vault-a", "ok": true, "slug": "new", "content_hash": Value::Null}),
        );
        // The delete carries no content_hash to chain into, so the prior
        // entry is left exactly as it was rather than cleared to a garbage
        // value — a later reference to the deleted slug still fails at
        // note_entry lookup, which is the correct signal.
        assert_eq!(
            chain.get(&("vault-a".to_string(), "new".to_string())),
            Some(&"fnv1a64:1".to_string())
        );
    }

    #[test]
    fn failure_to_error_value_prefers_the_structured_payload() {
        let structured = failure_to_error_value(JsonRpcFailure::not_found(
            json!({"code": "note_not_found", "message": "gone", "retryable": false}).to_string(),
        ));
        assert_eq!(structured["code"], "note_not_found");

        let plain = failure_to_error_value(JsonRpcFailure::invalid_params("bad input"));
        assert_eq!(plain["code"], -32602);
        assert_eq!(plain["message"], "bad input");
    }

    #[test]
    fn batch_tool_schema_requires_op_and_arguments_per_item() {
        let schema = batch_tool_schema();
        let item_schema = &schema["inputSchema"]["properties"]["operations"]["items"];
        assert_eq!(item_schema["required"], json!(["op", "arguments"]));
        assert_eq!(item_schema["additionalProperties"], false);
    }
}
