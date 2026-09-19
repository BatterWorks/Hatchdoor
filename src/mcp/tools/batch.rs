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
//! touched in between: [`batch_tool`] acquires every touched Vault's mutation
//! lock before the first item runs and holds them for the rest of the call,
//! rather than per item like a standalone write does, closing that window
//! instead of narrowing it.
//!
//! **Acquisition is ordered, not lazy** (issue #321). A batch is the only
//! thing in the instance that holds more than one Vault's mutation lock at a
//! time, so it is the only place a lock cycle can form — and it formed
//! trivially, because locks used to be taken lazily in whatever order the
//! caller's items happened to name Vaults. Two concurrent calls whose items
//! named Vaults A and B in opposite orders deadlocked each other for the life
//! of the process. [`lock_touched_vaults`] instead pre-scans the items,
//! collects the distinct Vault IDs, **sorts them**, and acquires in that one
//! canonical order. A total order over the only multi-lock holder makes a
//! cycle impossible, which is why there is no acquisition timeout here: a
//! batch may legitimately wait minutes behind a Git turn's network work
//! (ADR-18), and a timeout would turn that wait into a spurious failure while
//! buying nothing the ordering has not already bought.
//!
//! The pre-scan also resolves each Vault's control block for the whole call,
//! and every item against that Vault runs on that one block. The lock lives
//! on the control block, so re-resolving per item could hand a later item a
//! *replacement* block published by a mid-batch definition edit — a different
//! mutex, i.e. a write with no live exclusion.
//!
//! Since #321 a replacement block inherits the retiring one's exclusion, so a
//! definition edit is survivable rather than fatal, and both halves of the
//! call handle it. [`lock_one_vault`] re-resolves when the block it queued on
//! is retired before its turn at the lock arrives, and [`dispatch_one`]
//! re-resolves when the edit lands after the lock was taken, continuing only
//! against a live block that shares the exclusion this call holds. A Vault
//! that comes back without it, or does not come back at all, is refused with
//! a structured error rather than written unlocked, and every item naming a
//! Vault the pre-scan could not lock reports the reason it recorded.
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
use super::super::protocol::{JsonRpcFailure, OUTCOME_FIELD, tool_success};
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

/// Write ops a batch refuses although they are write tools. `rename_tag`
/// touches every note carrying a tag and has its own plan-then-apply
/// handshake; inside a best-effort batch with no rollback between items, the
/// all-or-nothing promise it makes would be one item's promise among many.
const NOT_BATCHABLE_WRITE_OPS: &[&str] = &["rename_tag"];

/// `(vault_id, slug) -> content_hash`, tracking each note's most recent
/// resulting hash from an earlier item in this same batch call. Keyed by the
/// raw `vault_id` string rather than a parsed `VaultId`: this is pure
/// in-batch bookkeeping, never used to resolve or authorize a Vault (every
/// dispatch still parses and validates `vault_id` itself), and `VaultId`
/// carries no `Hash` impl to key a map with.
type HashChain = HashMap<(String, String), String>;

/// One Vault this batch call writes to: the guard it holds on that Vault for
/// the whole call, and the control block that guard was taken from, resolved
/// once so every item against the Vault runs on one generation of it — see
/// the module doc comment for both.
struct LockedVault {
    vault_id: VaultId,
    vault: write::McpVault,
    /// Dropped when the batch returns, which is what releases the Vault.
    _guard: tokio::sync::OwnedMutexGuard<()>,
}

/// Why a Vault this batch's items name could not be locked, kept so every
/// item naming it reports the reason it would have reported standing alone,
/// rather than a generic refusal. Stored field by field because
/// [`JsonRpcFailure`] is not `Clone` and several items may name one Vault.
struct UnlockedVault {
    vault_id: VaultId,
    code: i64,
    message: String,
    tool_level: bool,
}

impl UnlockedVault {
    fn failure(&self) -> JsonRpcFailure {
        JsonRpcFailure {
            code: self.code,
            message: self.message.clone(),
            tool_level: self.tool_level,
        }
    }
}

/// What the pre-scan settled for this call: the Vaults it locked, in canonical
/// acquisition order, and the ones it could not lock with the reason. `Vec`s
/// rather than maps: `VaultId` has no `Hash` impl, and a batch touches at most
/// a handful of distinct Vaults, so a linear scan against
/// `BATCH_MAX_WRITE_ITEMS` (20) entries is cheap.
#[derive(Default)]
struct VaultLocks {
    held: Vec<LockedVault>,
    unlocked: Vec<UnlockedVault>,
}

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
        } else if NOT_BATCHABLE_WRITE_OPS.contains(&item.op.as_str()) {
            return Err(JsonRpcFailure::invalid_params(format!(
                "batch item {index}: op '{}' is not allowed inside batch; call it on its own",
                item.op
            )));
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
    // Taken before the first item runs, in one canonical order, and held
    // across the whole loop below (dropped only when `batch_tool` returns):
    // nothing outside this call can land a write that a later chained item's
    // substituted hash would then silently overwrite, and no two concurrent
    // batches can take two Vaults in opposite orders.
    let mut locks = lock_touched_vaults(&state, config, &args.operations).await;
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
                    let error = item_error_value(&value);
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

/// Take every Vault mutation lock this batch's write items will need, in one
/// canonical order, before any item runs.
///
/// The order is the Vault IDs sorted, which is what makes a lock cycle
/// between two concurrent batches impossible — see the module doc comment. A
/// Vault this cannot lock is recorded with the reason instead, and
/// [`dispatch_one`] hands that reason to every item naming it rather than
/// writing to a Vault this call does not hold.
async fn lock_touched_vaults(
    state: &AppState,
    config: &McpConfig,
    operations: &[BatchOperation],
) -> VaultLocks {
    let mut locks = VaultLocks::default();
    if !config.write_enabled {
        return locks;
    }
    let mut vault_ids: Vec<VaultId> = Vec::new();
    for item in operations {
        if !WRITE_OPS.contains(&item.op.as_str()) {
            continue;
        }
        let Ok(vault_id) = write::parse_vault_id(&item.arguments) else {
            continue;
        };
        if !vault_ids.contains(&vault_id) {
            vault_ids.push(vault_id);
        }
    }
    vault_ids.sort();

    locks.held.reserve(vault_ids.len());
    for vault_id in vault_ids {
        match lock_one_vault(state, vault_id).await {
            Ok(locked) => locks.held.push(locked),
            Err(failure) => locks.unlocked.push(UnlockedVault {
                vault_id,
                code: failure.code,
                message: failure.message,
                tool_level: failure.tool_level,
            }),
        }
    }
    locks
}

/// How many times acquisition may lose its control block to a reconcile before
/// the Vault is given up on. A definition edit revokes the block a waiter is
/// queued on, and the waiter learns that only once the lock is granted, so the
/// replacement has to be resolved and taken instead. Bounded because retrying
/// is only ever right for a *finite* burst of edits.
const MUTATION_ACQUIRE_ATTEMPTS: usize = 4;

/// Resolve one Vault and take its mutation lock, following a definition edit
/// that retires the control block while this is queued behind it.
async fn lock_one_vault(
    state: &AppState,
    vault_id: VaultId,
) -> Result<LockedVault, JsonRpcFailure> {
    let mut last = None;
    for _ in 0..MUTATION_ACQUIRE_ATTEMPTS {
        // Resolved inside the loop: after a retirement the live block is a
        // different one, and it is the live block this call must run on.
        let vault = write::scoped_vault_by_id(state, vault_id)?;
        match write::acquire_mutation(&vault).await {
            Ok(guard) => {
                return Ok(LockedVault {
                    vault_id,
                    vault,
                    _guard: guard,
                });
            }
            Err(failure) => last = Some(failure),
        }
    }
    Err(last.unwrap_or_else(|| write::exclusion_lost_error(vault_id)))
}

/// Dispatches one batch item to the same tool function a standalone call to
/// `op` would use. Mirrors `mod.rs`'s own dispatch match, restricted to the
/// note/attachment allowlist above. `locks` carries every Vault mutation
/// guard this batch call holds — see [`VaultLocks`] and
/// [`lock_touched_vaults`].
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
            let vault_id = write::parse_vault_id(&arguments)?;
            let Some(locked) = locks
                .held
                .iter_mut()
                .find(|locked| locked.vault_id == vault_id)
            else {
                // Not locked, so the pre-scan could not resolve, gate or take
                // this Vault. Report the reason it recorded — the same one a
                // standalone call would have given — rather than writing to a
                // Vault this call does not hold.
                return Err(locks
                    .unlocked
                    .iter()
                    .find(|unlocked| unlocked.vault_id == vault_id)
                    .map(UnlockedVault::failure)
                    .unwrap_or_else(|| write::exclusion_lost_error(vault_id)));
            };
            if !locked.vault.still_admits_operations() {
                // The Vault was reconciled mid-batch. A definition edit
                // publishes a replacement block that inherits the exclusion
                // (#321), so the guard held here still serializes against it
                // and the item runs on the live block. A Vault that is gone,
                // disabled, or now refuses writes reports that for itself,
                // and one whose replacement does *not* share the exclusion is
                // refused outright — running it would be an unlocked write.
                let fresh = write::scoped_vault_by_id(&state, vault_id)?;
                if !fresh.shares_write_exclusion(&locked.vault) {
                    return Err(write::exclusion_lost_error(vault_id));
                }
                locked.vault = fresh;
            }
            write::dispatch_write_tool(state, &locked.vault, op, arguments, config).await
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

/// Unwraps the error object out of a tool result that already rendered its own
/// failure envelope (the read tools return one instead of a `JsonRpcFailure`).
///
/// The batch item's own [`BatchItemResult::ok`] is the authoritative outcome
/// here, and it is what the tools reference tells a caller to read, so the
/// nested `error` stays the bare `{code, message, retryable, vault_id?}` object
/// it has always been. That means dropping the [`OUTCOME_FIELD`]
/// `tool_structured_error` sets: an item error carrying its own `ok: false`
/// next to the item's `ok: false` would say the same thing twice, in a place
/// where the write half of the allow-list says it once. Write items never reach
/// this function at all, since [`dispatch_one`] hands their failures back as a
/// `JsonRpcFailure` for [`failure_to_error_value`] to shape.
fn item_error_value(value: &Value) -> Value {
    let Some(mut error) = value.get("structuredContent").cloned() else {
        return json!({ "message": value["content"][0]["text"] });
    };
    if let Some(object) = error.as_object_mut() {
        object.remove(OUTCOME_FIELD);
    }
    error
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
        "description": "Execute an ordered list of note and attachment operations in one call — the same tools available standalone (create_note through delete_attachment, and every read tool except list_vaults). rename_tag is not allowed inside a batch; call it on its own. Vault-management tools (create_vault, edit_vault, enable_vault, disable_vault, disconnect_vault, sync_vault, retry_vault, refresh_vault, list_vaults) are not allowed inside a batch; those and any unrecognized op are rejected before anything executes. Execution is in order and best-effort: each item reports its own ok/result/error, one item failing does not stop the rest, and there is no rollback or mid-batch visibility between items. All resulting Vault changes are committed together on the Vault's next Git sync turn, the same as any other burst of writes. expected_content_hash checks are skipped between items that share a vault_id and slug: create or edit a note earlier in this batch, then reference it again later in the same call without knowing the intermediate hash; a note not otherwise touched in this batch still validates its expected_content_hash normally. A batch may contain at most 50 read-shaped items and 20 write-shaped items.",
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
            "refresh_vault",
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
    fn not_batchable_write_ops_are_write_ops() {
        for op in NOT_BATCHABLE_WRITE_OPS {
            assert!(WRITE_OPS.contains(op), "{op} must also be a write op");
            assert!(
                !HASH_CHAINED_OPS.contains(op),
                "{op} can never be chained into"
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

    /// Both halves of the allow-list produce the same bare error object for an
    /// item: the write half never passes through a tool-result envelope, and
    /// the read half has the envelope's failure marker taken back off.
    #[test]
    fn an_item_error_keeps_the_bare_domain_error_shape() {
        let domain = json!({"code": "note_not_found", "message": "gone", "retryable": false});
        let from_envelope =
            item_error_value(&crate::mcp::protocol::tool_structured_error(domain.clone()));
        assert_eq!(from_envelope, domain);
        assert_eq!(
            from_envelope,
            failure_to_error_value(JsonRpcFailure::not_found(domain.to_string())),
        );

        let plain_text = item_error_value(&crate::mcp::protocol::tool_error("no payload".into()));
        assert_eq!(plain_text, json!({"message": "no payload"}));
    }

    #[test]
    fn batch_tool_schema_requires_op_and_arguments_per_item() {
        let schema = batch_tool_schema();
        let item_schema = &schema["inputSchema"]["properties"]["operations"]["items"];
        assert_eq!(item_schema["required"], json!(["op", "arguments"]));
        assert_eq!(item_schema["additionalProperties"], false);
    }
}
