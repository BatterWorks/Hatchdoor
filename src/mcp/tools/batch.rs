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
//! A note the batch only *read* is a fourth, opt-in relaxation (#599): a
//! write that passes the marker `"from_batch"` as its `expected_content_hash`
//! is checked against the hash an earlier `get_note`, `get_note_outline`,
//! `get_note_section` or `get_frontmatter` item returned for that note. See
//! [`apply_read_hash`]. Nothing is skipped: the write still compares that hash
//! with the file, so a note changed since the read is a `write_conflict`.
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
use super::super::protocol::{
    JsonRpcFailure, MAX_JSONRPC_RESPONSE_BYTES, OUTCOME_FIELD, tool_success,
};
use super::super::results::{BatchItemResult, BatchResult, result_to_value};
use super::write::WRITE_OPS;
use super::{
    READ_OPS, WRITE_DISABLED_CODE, WRITE_DISABLED_MESSAGE, dispatch_read_tool, write,
    write_tool_annotations,
};

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

/// Write ops a batch refuses although they are write tools. `rename_tag` and
/// `delete_tag` touch every note carrying a tag and have their own
/// plan-then-apply handshake; inside a best-effort batch with no rollback
/// between items, the all-or-nothing promise each makes would be one item's
/// promise among many.
const NOT_BATCHABLE_WRITE_OPS: &[&str] = &["rename_tag", "delete_tag"];

/// The reads whose result carries the whole note's `content_hash`, the same
/// string a write compares. A successful item of one of these records that
/// hash for a later `"from_batch"` write on the same note (#599).
const HASH_READ_OPS: &[&str] = &[
    "get_note",
    "get_note_outline",
    "get_note_section",
    "get_frontmatter",
];

/// `(vault_id, slug) -> content_hash`, tracking each note's most recent
/// resulting hash from an earlier item in this same batch call. Keyed by the
/// raw `vault_id` string rather than a parsed `VaultId`: this is pure
/// in-batch bookkeeping, never used to resolve or authorize a Vault (every
/// dispatch still parses and validates `vault_id` itself), and `VaultId`
/// carries no `Hash` impl to key a map with.
type HashChain = HashMap<(String, String), String>;

/// `(vault_id, slug) -> content_hash` for notes an earlier item in this batch
/// call *read*, kept apart from [`HashChain`] because the two are trusted
/// differently. A written hash replaces whatever the caller sent. A read hash
/// is used only when the caller asks for it with [`write::FROM_BATCH_HASH`]:
/// a caller who sends a real hash planned the edit on that version, and must
/// not have it re-pointed at one it never saw.
type ReadHashes = HashMap<(String, String), String>;

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
    domain_error: Option<Value>,
}

impl UnlockedVault {
    fn failure(&self) -> JsonRpcFailure {
        JsonRpcFailure {
            code: self.code,
            message: self.message.clone(),
            tool_level: self.tool_level,
            domain_error: self.domain_error.clone(),
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

/// Appended to every refusal of a malformed batch payload, so the caller learns
/// the shape it should have sent rather than only the next field serde expected.
const BATCH_SHAPE_HINT: &str = "batch takes only `operations`, an array of `{op, arguments}` \
     items. Each tool's own arguments, vault_id and commit_summary included, go inside an \
     item's `arguments`.";

/// Parses the `batch` arguments, naming every unknown field in one refusal.
///
/// `deny_unknown_fields` stops at the first stray key, and without
/// `preserve_order` serde walks keys alphabetically, so a caller sending a
/// top-level `vault_id` and a batch-level `commit_summary` used to learn about
/// them one round trip at a time (#297). The keys at both levels are checked
/// here first; serde then reports anything else, such as a missing `op` or a
/// wrong type, with the same shape appended.
fn parse_batch_args(arguments: Value) -> Result<BatchArgs, JsonRpcFailure> {
    let mut unknown: Vec<String> = Vec::new();
    if let Some(object) = arguments.as_object() {
        unknown.extend(unknown_keys(object, &["operations"], ""));
        if let Some(operations) = object.get("operations").and_then(Value::as_array) {
            for (index, item) in operations.iter().enumerate() {
                // A non-object item has no keys to name; serde refuses it below.
                if let Some(item) = item.as_object() {
                    let prefix = format!("operations[{index}].");
                    unknown.extend(unknown_keys(item, &["op", "arguments"], &prefix));
                }
            }
        }
    }
    if !unknown.is_empty() {
        let noun = if unknown.len() == 1 {
            "field"
        } else {
            "fields"
        };
        return Err(JsonRpcFailure::invalid_params(format!(
            "Invalid batch arguments: unknown {noun} {}. {BATCH_SHAPE_HINT}",
            unknown.join(", ")
        )));
    }
    serde_json::from_value(arguments).map_err(|error| {
        JsonRpcFailure::invalid_params(format!(
            "Invalid batch arguments: {error}. {BATCH_SHAPE_HINT}"
        ))
    })
}

/// The keys of `object` outside `known`, sorted and quoted with `prefix` so
/// the refusal reads the same whatever order the caller sent them in.
fn unknown_keys(
    object: &serde_json::Map<String, Value>,
    known: &[&str],
    prefix: &str,
) -> Vec<String> {
    let mut keys: Vec<String> = object
        .keys()
        .filter(|key| !known.contains(&key.as_str()))
        .map(|key| format!("`{prefix}{key}`"))
        .collect();
    keys.sort();
    keys
}

pub(super) async fn batch_tool(
    state: AppState,
    arguments: Value,
    config: &McpConfig,
) -> Result<Value, JsonRpcFailure> {
    run_batch(state, arguments, config, &mut |_| {}).await
}

/// [`batch_tool`], calling `before_item` with each item's index just before
/// that item runs. Production passes a no-op; a test uses it to change a file
/// between two items, which nothing outside the call can otherwise time.
pub(super) async fn run_batch(
    state: AppState,
    arguments: Value,
    config: &McpConfig,
    before_item: &mut (dyn FnMut(usize) + Send),
) -> Result<Value, JsonRpcFailure> {
    let args = parse_batch_args(arguments)?;
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
    let mut read_hashes: ReadHashes = HashMap::new();
    // Taken before the first item runs, in one canonical order, and held
    // across the whole loop below (dropped only when `batch_tool` returns):
    // nothing outside this call can land a write that a later chained item's
    // substituted hash would then silently overwrite, and no two concurrent
    // batches can take two Vaults in opposite orders.
    let mut locks = lock_touched_vaults(&state, config, &args.operations).await;
    let mut items = Vec::with_capacity(args.operations.len());
    let mut succeeded = 0usize;
    let mut failed = 0usize;
    // Serialized bytes of the items so far, against [`BATCH_REPLY_BUDGET`].
    let mut reply_bytes = 0usize;

    for (index, item) in args.operations.into_iter().enumerate() {
        let BatchOperation { op, arguments } = item;
        before_item(index);
        let arguments = apply_hash_chain(&op, arguments, &chain);
        let arguments = apply_read_hash(&op, arguments, &read_hashes);
        let remaining = BATCH_REPLY_BUDGET.saturating_sub(reply_bytes);
        let mut item = match dispatch_one(
            state.clone(),
            config,
            &op,
            arguments,
            &mut locks,
            remaining,
        )
        .await
        {
            Ok(mut value) => {
                if value.get("isError").and_then(Value::as_bool) == Some(true) {
                    let error = item_error_value(&value);
                    BatchItemResult {
                        index,
                        op,
                        ok: false,
                        result: None,
                        error: Some(error),
                    }
                } else {
                    // Moved, not copied: it can be an attachment of several MB.
                    let result = value.get_mut("structuredContent").map(Value::take);
                    if let Some(result) = &result {
                        record_chain(&mut chain, &op, result);
                    }
                    BatchItemResult {
                        index,
                        op,
                        ok: true,
                        result,
                        error: None,
                    }
                }
            }
            Err(failure) => BatchItemResult {
                index,
                op,
                ok: false,
                result: None,
                error: Some(failure_to_error_value(failure)),
            },
        };
        let mut size = serialized_len(&item);
        // A read's answer that does not fit is dropped here rather than kept
        // for a reply that would then be refused whole. A write's answer is
        // always kept: the write happened, and its result is small.
        if item.ok && READ_OPS.contains(&item.op.as_str()) && size > remaining {
            item = reply_budget_exceeded(item.index, item.op);
            size = serialized_len(&item);
        }
        reply_bytes = reply_bytes.saturating_add(size);
        // After the budget check, so a read whose answer was dropped, which
        // the caller sees as a failed item, lends no hash to a later write.
        if let Some(result) = &item.result {
            record_read_hash(&mut read_hashes, &item.op, result);
        }
        if item.ok {
            succeeded += 1;
        } else {
            failed += 1;
        }
        items.push(item);
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
        match lock_one_vault(state, vault_id, config).await {
            Ok(locked) => locks.held.push(locked),
            Err(failure) => locks.unlocked.push(UnlockedVault {
                vault_id,
                code: failure.code,
                message: failure.message,
                tool_level: failure.tool_level,
                domain_error: failure.domain_error,
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
    config: &McpConfig,
) -> Result<LockedVault, JsonRpcFailure> {
    let mut last = None;
    for _ in 0..MUTATION_ACQUIRE_ATTEMPTS {
        // Resolved inside the loop: after a retirement the live block is a
        // different one, and it is the live block this call must run on.
        let vault = write::scoped_vault_by_id(state, vault_id, config)?;
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
    reply_budget: usize,
) -> Result<Value, JsonRpcFailure> {
    match op {
        // A base64 attachment is the one read whose size the batch can know
        // before reading it, so it is refused up front when it cannot fit in
        // what is left of the reply (#632).
        "get_attachment" => {
            let limit = config.base64_limit();
            let cap = limit.min(base64_bytes_fitting(reply_budget));
            let mut value = super::read::get_attachment_tool(state, arguments, config, cap).await?;
            // The refusal names the cap it was held to; when that was the
            // reply's room rather than the base64 limit, it says so, since
            // the same call made on its own may well be answered.
            let refused_for_room = cap < limit
                && value.pointer("/structuredContent/code")
                    == Some(&json!("attachment_too_large_for_base64"));
            if refused_for_room {
                value["structuredContent"]["message"] = json!(format!(
                    "This attachment does not fit in what is left of the batch reply ({cap} bytes \
                     before encoding). Nothing was read: call get_attachment on its own, or \
                     split the batch."
                ));
            }
            Ok(value)
        }
        _ if READ_OPS.contains(&op) => dispatch_read_tool(state, config, op, arguments).await,
        _ if WRITE_OPS.contains(&op) => {
            // The refusal carries a stable string code (#327), like every
            // other item error: an item is data, not a JSON-RPC error.
            if !config.write_enabled {
                return Err(structured_item_failure(
                    &crate::vault_error::VaultOperationError::new(
                        WRITE_DISABLED_CODE,
                        WRITE_DISABLED_MESSAGE,
                        None,
                        false,
                    ),
                ));
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
                let fresh = write::scoped_vault_by_id(&state, vault_id, config)?;
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

/// The serialized bytes a batch's items may add up to (#632): a reply is
/// refused whole past `MAX_JSONRPC_RESPONSE_BYTES`, so the batch stops
/// accumulating before it, leaving room for the envelope. Without this, fifty
/// base64 attachments were all read, encoded and held, several hundred
/// megabytes, only for the finished reply to be refused.
const BATCH_REPLY_BUDGET: usize = MAX_JSONRPC_RESPONSE_BYTES - 64 * 1024;

/// What one item adds to the reply besides an attachment's encoded bytes:
/// its index, op, the result's other fields.
const ITEM_FRAMING_BYTES: usize = 4 * 1024;

/// The most decoded attachment bytes whose base64 fits in `reply_budget`.
fn base64_bytes_fitting(reply_budget: usize) -> u64 {
    (reply_budget.saturating_sub(ITEM_FRAMING_BYTES) / 4 * 3) as u64
}

/// The item that stands in for a read whose answer did not fit in the reply.
fn reply_budget_exceeded(index: usize, op: String) -> BatchItemResult {
    BatchItemResult {
        index,
        op,
        ok: false,
        result: None,
        error: Some(json!({
            "code": BATCH_REPLY_TOO_LARGE_CODE,
            "message": format!(
                "This item's answer did not fit in the batch reply, which holds {BATCH_REPLY_BUDGET} \
                 bytes in all. Nothing was changed: call it on its own, or split the batch."
            ),
            "retryable": false,
        })),
    }
}

/// The code a read item carries when its answer did not fit (#632).
const BATCH_REPLY_TOO_LARGE_CODE: &str = "batch_reply_too_large";

/// How many bytes `value` serializes to, without keeping them.
fn serialized_len(value: &impl serde::Serialize) -> usize {
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0);
    // Serializing a `Value`-backed result into a sink that never fails cannot
    // fail; were it to, counting nothing only makes the budget more lenient.
    let _ = serde_json::to_writer(&mut counter, value);
    counter.0
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
    if let Some(key) = note_key(&arguments)
        && let Some(hash) = chain.get(&key)
        && let Some(object) = arguments.as_object_mut()
    {
        object.insert("expected_content_hash".to_string(), json!(hash));
    }
    arguments
}

/// The `(vault_id, slug)` an item's arguments name, as the caller spelled
/// them, when both are strings.
fn note_key(arguments: &Value) -> Option<(String, String)> {
    let vault_id = arguments.get("vault_id").and_then(Value::as_str)?;
    let slug = arguments.get("slug").and_then(Value::as_str)?;
    Some((vault_id.to_string(), slug.to_string()))
}

/// Runs after [`apply_hash_chain`]: a hash-chained op that still carries the
/// marker [`write::FROM_BATCH_HASH`], so one this batch has not written, gets
/// the hash an earlier read item in this call returned for the same
/// `(vault_id, slug)` (#599). The write then validates that hash against the
/// file like any other, so a note changed since the read fails with
/// `write_conflict`. With no such read the marker is left in place, and
/// [`write::dispatch_write_tool`] refuses it before anything is written.
///
/// Any other value is left alone, a real hash above all: an earlier read of a
/// newer version must not rescue an edit planned on an older one.
fn apply_read_hash(op: &str, mut arguments: Value, read_hashes: &ReadHashes) -> Value {
    if !HASH_CHAINED_OPS.contains(&op) || !write::asks_for_batch_hash(&arguments) {
        return arguments;
    }
    if let Some(key) = note_key(&arguments)
        && let Some(hash) = read_hashes.get(&key)
        && let Some(object) = arguments.as_object_mut()
    {
        object.insert("expected_content_hash".to_string(), json!(hash));
    }
    arguments
}

/// After a successful read of one of [`HASH_READ_OPS`], records the whole
/// note's `content_hash` under the slug the result reports, which is the slug
/// the item asked for: an exact read matches a slug exactly. `get_note` nests
/// the slug and hash under `note`; the other three carry them at the top
/// level.
fn record_read_hash(read_hashes: &mut ReadHashes, op: &str, result: &Value) {
    if !HASH_READ_OPS.contains(&op) {
        return;
    }
    let note = result.get("note").unwrap_or(result);
    let vault_id = result.get("vault_id").and_then(Value::as_str);
    let slug = note.get("slug").and_then(Value::as_str);
    let hash = note.get("content_hash").and_then(Value::as_str);
    if let (Some(vault_id), Some(slug), Some(hash)) = (vault_id, slug, hash) {
        read_hashes.insert((vault_id.to_string(), slug.to_string()), hash.to_string());
    }
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
        // No read tool renders an unstructured error today; should one ever,
        // its item still carries a string code (#327).
        return json!({
            "code": plain_failure_code(0),
            "message": value["content"][0]["text"],
            "retryable": false,
        });
    };
    if let Some(object) = error.as_object_mut() {
        object.remove(OUTCOME_FIELD);
    }
    error
}

/// A per-item refusal shaped as the structured domain error it is, so
/// [`failure_to_error_value`] hands the item that object verbatim.
fn structured_item_failure(error: &crate::vault_error::VaultOperationError) -> JsonRpcFailure {
    JsonRpcFailure::not_found(
        serde_json::to_string(error).unwrap_or_else(|_| error.message.clone()),
    )
}

/// Renders a per-item dispatch failure as the bare `{code, message,
/// vault_id?, retryable}` object every item error shares, and `code` is
/// always a stable string (#327): the server instructions tell agents to
/// branch on it, so it must never be a JSON-RPC number.
///
/// A failure carrying its structured domain error (an unwritable target path)
/// reports that; a JSON-object message decodes to the structured error it
/// already is, the same way the top-level dispatcher's tail does; and a
/// plain-text message (an argument-parse rejection, a missing `vault_id`)
/// falls back to a string code named for its JSON-RPC class, with the number
/// kept alongside as `jsonrpc_code`.
fn failure_to_error_value(failure: JsonRpcFailure) -> Value {
    if let Some(domain_error) = failure.domain_error {
        return domain_error;
    }
    match serde_json::from_str::<Value>(&failure.message) {
        Ok(structured) if structured.get("code").is_some_and(Value::is_string) => structured,
        _ => json!({
            "code": plain_failure_code(failure.code),
            "message": failure.message,
            "retryable": false,
            "jsonrpc_code": failure.code,
        }),
    }
}

/// The stable string code for a plain-text failure, by its JSON-RPC class.
fn plain_failure_code(jsonrpc_code: i64) -> &'static str {
    match jsonrpc_code {
        -32602 => "invalid_arguments",
        -32601 => "unknown_operation",
        JsonRpcFailure::INTERNAL_ERROR_CODE => "internal_error",
        _ => "operation_failed",
    }
}

pub(super) fn batch_tool_schema() -> Value {
    json!({
        "name": "batch",
        "description": "Execute an ordered list of note and attachment operations in one call — the same tools available standalone (create_note through delete_attachment, and every read tool except list_vaults, read_docs and search_docs). rename_tag and delete_tag are not allowed inside a batch; call them on their own. Vault-management tools (create_vault, edit_vault, enable_vault, disable_vault, disconnect_vault, sync_vault, retry_vault, publish_recovery_branch, refresh_vault, list_vaults) and the manual tools (read_docs, search_docs) are not allowed inside a batch; those and any unrecognized op are rejected before anything executes. Execution is in order and best-effort: each item reports its own ok/result/error, one item failing does not stop the rest, and there is no rollback or mid-batch visibility between items. All resulting Vault changes are committed together on the Vault's next Git sync turn, the same as any other burst of writes. expected_content_hash checks are skipped between items that share a vault_id and slug: create or edit a note earlier in this batch, then reference it again later in the same call without knowing the intermediate hash; a note not otherwise touched in this batch still validates its expected_content_hash normally. To edit a note you have not read yet in one call, read it earlier in the same batch with get_note, get_note_outline, get_note_section or get_frontmatter, then pass the literal \"from_batch\" as the write's expected_content_hash: the write is checked against the hash that read returned, and fails with write_conflict if the note changed in between. The write must name the note by the same vault_id and slug as the read. \"from_batch\" with no earlier item for that note, or outside a batch, fails with code batch_hash_unavailable and writes nothing. A real hash is always checked as given, whatever an earlier read returned. create_upload_link does not take \"from_batch\": it needs the real hash. A batch may contain at most 50 read-shaped items and 20 write-shaped items. Its reply holds about 8 MB in all: a read whose answer would not fit fails with code batch_reply_too_large, and a base64 get_attachment that would not fit fails with attachment_too_large_for_base64 before it is read; call those on their own. Each search_notes item counts against the per-minute tool-call quota as a standalone search would. Every failed item's error carries a string code to branch on. Unlike every other tool, batch takes no top-level vault_id and no batch-level commit_summary: each goes inside the arguments of the operations whose tool takes it.",
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
                                "description": "That tool's own arguments exactly as it is called standalone. This is the only place vault_id and commit_summary go; batch itself takes neither."
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
            "read_docs",
            "search_docs",
            "create_vault",
            "edit_vault",
            "enable_vault",
            "disable_vault",
            "disconnect_vault",
            "sync_vault",
            "retry_vault",
            "publish_recovery_branch",
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
            public_url: None,
            request_origin: None,
            client_name: None,
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
    fn hash_read_ops_are_read_ops() {
        for op in HASH_READ_OPS {
            assert!(READ_OPS.contains(op), "{op} must be a batchable read");
        }
    }

    #[test]
    fn apply_read_hash_fills_only_the_marker_on_a_note_that_was_read() {
        let mut reads = ReadHashes::new();
        reads.insert(
            ("vault-a".to_string(), "note".to_string()),
            "fnv1a64:read".to_string(),
        );
        let edit = |slug: &str, hash: &str| json!({"vault_id": "vault-a", "slug": slug, "expected_content_hash": hash});

        let filled = apply_read_hash("edit_note", edit("note", "from_batch"), &reads);
        assert_eq!(filled["expected_content_hash"], "fnv1a64:read");

        // A real hash is the caller's claim about the version it planned on.
        let kept = apply_read_hash("edit_note", edit("note", "fnv1a64:older"), &reads);
        assert_eq!(kept["expected_content_hash"], "fnv1a64:older");

        // No read of this note: the marker stays for the write to refuse.
        let unread = apply_read_hash("edit_note", edit("other", "from_batch"), &reads);
        assert_eq!(unread["expected_content_hash"], "from_batch");

        // Not a hash-chained op.
        let upload = apply_read_hash("create_upload_link", edit("note", "from_batch"), &reads);
        assert_eq!(upload["expected_content_hash"], "from_batch");
    }

    #[test]
    fn record_read_hash_reads_both_result_shapes() {
        let mut reads = ReadHashes::new();
        let key = |slug: &str| ("vault-a".to_string(), slug.to_string());

        record_read_hash(
            &mut reads,
            "get_note",
            &json!({"vault_id": "vault-a", "note": {"slug": "nested", "content_hash": "fnv1a64:n"}}),
        );
        assert_eq!(reads.get(&key("nested")), Some(&"fnv1a64:n".to_string()));

        record_read_hash(
            &mut reads,
            "get_frontmatter",
            &json!({"vault_id": "vault-a", "slug": "flat", "content_hash": "fnv1a64:f"}),
        );
        assert_eq!(reads.get(&key("flat")), Some(&"fnv1a64:f".to_string()));

        // A read that returns no whole-note hash to spend records nothing.
        record_read_hash(
            &mut reads,
            "get_note_links",
            &json!({"vault_id": "vault-a", "slug": "links", "content_hash": "fnv1a64:l"}),
        );
        assert_eq!(reads.get(&key("links")), None);
    }

    #[test]
    fn failure_to_error_value_prefers_the_structured_payload() {
        let structured = failure_to_error_value(JsonRpcFailure::not_found(
            json!({"code": "note_not_found", "message": "gone", "retryable": false}).to_string(),
        ));
        assert_eq!(structured["code"], "note_not_found");

        let plain = failure_to_error_value(JsonRpcFailure::invalid_params("bad input"));
        assert_eq!(plain["code"], "invalid_arguments");
        assert_eq!(plain["message"], "bad input");
        assert_eq!(plain["retryable"], false);
        assert_eq!(plain["jsonrpc_code"], -32602);

        let internal = failure_to_error_value(JsonRpcFailure::internal("boom"));
        assert_eq!(internal["code"], "internal_error");

        let carried = failure_to_error_value(
            JsonRpcFailure::invalid_params("noise")
                .with_domain_error(json!({"code": "noise_excluded_write", "message": "noise"})),
        );
        assert_eq!(carried["code"], "noise_excluded_write");
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
        assert_eq!(
            plain_text,
            json!({"code": "operation_failed", "message": "no payload", "retryable": false})
        );
    }

    #[test]
    fn unknown_fields_are_collected_across_both_levels_in_one_message() {
        let failure = parse_batch_args(json!({
            "vault_id": "v",
            "commit_summary": "s",
            "operations": [
                {"op": "get_note", "arguments": {}},
                {"op": "update_note", "arguments": {}, "slug": "a", "commit_summary": "c"},
                "not an object"
            ]
        }))
        .expect_err("unknown fields are refused");

        assert_eq!(failure.code, JsonRpcFailure::invalid_params("").code);
        assert_eq!(
            failure.message,
            "Invalid batch arguments: unknown fields `commit_summary`, `vault_id`, \
             `operations[1].commit_summary`, `operations[1].slug`. batch takes only \
             `operations`, an array of `{op, arguments}` items. Each tool's own arguments, \
             vault_id and commit_summary included, go inside an item's `arguments`."
        );
    }

    #[test]
    fn a_single_unknown_field_is_named_in_the_singular() {
        let failure = parse_batch_args(json!({"vault_id": "v", "operations": []}))
            .expect_err("unknown field is refused");
        assert!(
            failure
                .message
                .starts_with("Invalid batch arguments: unknown field `vault_id`. batch takes only"),
            "{}",
            failure.message
        );
    }

    #[test]
    fn a_malformed_payload_without_unknown_fields_still_states_the_shape() {
        let failure = parse_batch_args(json!({"operations": [{"op": "get_note"}]}))
            .expect_err("a missing field is refused");
        assert!(failure.message.contains("missing field `arguments`"));
        assert!(
            failure
                .message
                .contains("batch takes only `operations`, an array of `{op, arguments}` items"),
            "{}",
            failure.message
        );
    }

    #[test]
    fn a_well_formed_payload_parses() {
        let args = parse_batch_args(json!({
            "operations": [{"op": "get_note", "arguments": {"vault_id": "v", "slug": "home"}}]
        }))
        .expect("valid batch arguments");
        assert_eq!(args.operations.len(), 1);
        assert_eq!(args.operations[0].op, "get_note");
    }

    #[test]
    fn batch_tool_description_states_where_vault_id_and_commit_summary_belong() {
        let schema = batch_tool_schema();
        let description = schema["description"].as_str().expect("description");
        assert!(
            description.contains(
                "Unlike every other tool, batch takes no top-level vault_id and no batch-level \
                 commit_summary: each goes inside the arguments of the operations whose tool \
                 takes it."
            ),
            "{description}"
        );
        let arguments = schema["inputSchema"]["properties"]["operations"]["items"]["properties"]
            ["arguments"]["description"]
            .as_str()
            .expect("arguments description");
        assert!(
            arguments.contains("the only place vault_id and commit_summary go"),
            "{arguments}"
        );
    }

    #[test]
    fn batch_tool_schema_keeps_operations_as_its_only_property() {
        let schema = batch_tool_schema();
        let input = &schema["inputSchema"];
        assert_eq!(input["required"], json!(["operations"]));
        assert_eq!(input["additionalProperties"], false);
        assert_eq!(
            input["properties"]
                .as_object()
                .expect("properties")
                .keys()
                .collect::<Vec<_>>(),
            vec!["operations"]
        );
    }

    #[test]
    fn batch_tool_schema_requires_op_and_arguments_per_item() {
        let schema = batch_tool_schema();
        let item_schema = &schema["inputSchema"]["properties"]["operations"]["items"];
        assert_eq!(item_schema["required"], json!(["op", "arguments"]));
        assert_eq!(item_schema["additionalProperties"], false);
    }
}
