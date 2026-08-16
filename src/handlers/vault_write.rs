//! `/api/v1/vaults/{vault_id}/...` — exactly-one-Vault Markdown mutations,
//! attachment upload, and write-capabilities discovery.
//!
//! This is a Vault-scoped adapter over the unchanged shared mutation layer in
//! [`crate::vault::write`] (ADR-03): every handler here resolves exactly one
//! Vault's [`crate::vault_runtime::VaultControlBlock`], serializes concurrent
//! mutations to that Vault through its own `acquire_mutation` lock (never the
//! legacy single-Vault `AppState::vault_write_lock`), and calls straight into
//! already-implemented, already-unit-tested write functions. It reuses
//! `handlers/vaults.rs`'s `VaultApiError`/`parse_vault_id`/rejection helpers
//! and `handlers/vault_content.rs`'s `vault_read_error_response`, mounted
//! alongside them in the same router group and sharing its auth posture.
//! Every route here is a content mutation or write-capability discovery, so
//! `src/server.rs` wraps each one in `reject_demo_mutation` (#109): in demo
//! mode it refuses with the shared `403 demo_read_only` error before running,
//! unlike `vault_content.rs`'s exact reads and `vault_collection_reads.rs`'s
//! one-or-all reads, which stay reachable unauthenticated in demo mode. The
//! literal `all` is never accepted (mutations always name exactly one Vault
//! ID — issue #62).
//!
//! Unlike the legacy single-Vault write API, a mutation response never
//! includes a `git_sync_warning`: the managed-Git scheduler has no
//! debounced-on-write hook (it runs on its own poll/manual schedule), so
//! there is nothing per-mutation to report that `GET /api/v1/vaults` does not
//! already expose through that Vault's own `git` status.
//!
//! Internal helpers propagate small typed errors (`VaultReadError`, or a
//! `(StatusCode, VaultApiError)` pair) rather than a built `Response`, mirroring
//! `vault_content.rs`'s convention — `axum::response::Response` is large
//! enough to trip `clippy::result_large_err`, and building the response body
//! only at the outermost point keeps every intermediate `Result` small.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Multipart, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use crate::app_state::AppState;
use crate::handlers::vault_content::vault_read_error_response;
use crate::handlers::vaults::{VaultApiError, parse_vault_id};
use crate::vault::{
    AttachmentInfo, AttachmentOutcome, ExcludeMatcher, LayerMap, VaultIndex, WriteError,
    WriteOutcome, archive_note, create_note, delete_note, import_attachment_bytes,
    move_or_rename_note, update_note,
};
use crate::vault_read::{VaultReadCore, VaultReadError};
use crate::vault_registry::VaultId;
use crate::vault_runtime::VaultControlBlock;

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VaultCreateNoteRequest {
    pub relative_path: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VaultUpdateNoteRequest {
    pub content: String,
    pub expected_content_hash: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VaultRenameNoteRequest {
    pub new_title: String,
    pub expected_content_hash: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VaultMoveNoteRequest {
    pub target_folder: String,
    pub expected_content_hash: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VaultMoveRenameNoteRequest {
    pub target_relative_path: String,
    pub expected_content_hash: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VaultArchiveNoteRequest {
    pub expected_content_hash: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VaultDeleteNoteRequest {
    pub expected_content_hash: String,
}

#[derive(Debug, Serialize)]
pub struct VaultWriteCapabilitiesResponse {
    pub vault_id: VaultId,
    pub enabled: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct VaultWriteOutcomeResponse {
    pub vault_id: VaultId,
    pub ok: bool,
    pub slug: Option<String>,
    pub relative_path: Option<String>,
    pub content_hash: Option<String>,
    pub quality_warnings: Vec<String>,
    pub rewritten_notes: usize,
    pub moved_assets: usize,
    pub trashed_path: Option<String>,
    /// The resulting note's layer (`None` = default surface).
    pub layer: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct VaultAttachmentOutcomeResponse {
    pub vault_id: VaultId,
    pub ok: bool,
    pub attachment: AttachmentInfo,
    pub rewritten_notes: usize,
    pub trashed_path: Option<String>,
    pub cleanup_warning: Option<String>,
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// A `(status, body)` pair small enough to propagate through intermediate
/// `Result`s; built into a real `Response` only at the point it is returned
/// from a handler.
type ApiError = (StatusCode, VaultApiError);

fn respond((status, error): ApiError) -> Response {
    error.respond(status)
}

fn bad_request_error(error: VaultApiError) -> ApiError {
    (StatusCode::BAD_REQUEST, error)
}

fn note_not_found_error(vault_id: VaultId, slug: &str) -> ApiError {
    (
        StatusCode::NOT_FOUND,
        VaultApiError::new(
            "note_not_found",
            format!("Note not found: {slug}"),
            Some(vault_id),
            false,
        ),
    )
}

fn capability_unavailable_error(vault_id: VaultId) -> ApiError {
    (
        StatusCode::CONFLICT,
        VaultApiError::new(
            "capability_unavailable",
            "This Vault's current source and lifecycle do not allow mutation",
            Some(vault_id),
            false,
        ),
    )
}

fn write_error_response(vault_id: VaultId, error: WriteError) -> Response {
    // A partially-applied multi-phase mutation needs operator action, so its
    // message must survive rather than collapse into the generic sanitized
    // internal error every other `Io` failure gets.
    if let Some(message) = error.recovery_message() {
        return VaultApiError::new(
            "write_recovery_required",
            message.to_string(),
            Some(vault_id),
            false,
        )
        .respond(StatusCode::INTERNAL_SERVER_ERROR);
    }
    match error {
        WriteError::Conflict(message) => {
            VaultApiError::new("write_conflict", message, Some(vault_id), true)
                .respond(StatusCode::CONFLICT)
        }
        WriteError::InvalidInput(message) => {
            VaultApiError::new("invalid_write_input", message, Some(vault_id), false)
                .respond(StatusCode::BAD_REQUEST)
        }
        WriteError::Io(message) => {
            crate::handlers::vaults::internal_error_response(message, Some(vault_id))
        }
    }
}

fn noise_write_error(vault_id: VaultId, path: &str) -> ApiError {
    (
        StatusCode::BAD_REQUEST,
        VaultApiError::new(
            "noise_excluded_write",
            format!(
                "'{path}' matches this Vault's noise-exclusion pattern and would be ignored by \
                 the index; choose a path outside the excluded set."
            ),
            Some(vault_id),
            false,
        ),
    )
}

fn invalid_input_error(vault_id: VaultId, field: &str) -> ApiError {
    (
        StatusCode::BAD_REQUEST,
        VaultApiError::new(
            "invalid_write_input",
            format!("{field} cannot be empty"),
            Some(vault_id),
            false,
        ),
    )
}

fn non_empty_input(vault_id: VaultId, field: &str, value: String) -> Result<String, ApiError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(invalid_input_error(vault_id, field));
    }
    Ok(trimmed.to_string())
}

fn replace_filename(relative_path: &str, new_title: &str) -> String {
    let directory = relative_path.rsplit_once('/').map(|(dir, _)| dir);
    match directory {
        Some(dir) if !dir.is_empty() => format!("{dir}/{new_title}.md"),
        _ => format!("{new_title}.md"),
    }
}

/// The gated control block for a mutation: not-found, disabled, and
/// no-runtime resolve through the same check exact reads use.
fn mutation_control_block(
    state: &AppState,
    vault_id: VaultId,
) -> Result<VaultControlBlock, VaultReadError> {
    let core = VaultReadCore::new(&state.startup_sqlite, &state.vaults);
    core.control_block(vault_id)
}

/// This Vault's current source/lifecycle capability, e.g. a Pull-only managed
/// Git Vault never allows mutation (issue #62).
fn ensure_mutable(vault_id: VaultId, control: &VaultControlBlock) -> Result<(), ApiError> {
    if control.snapshot().capabilities.mutate {
        Ok(())
    } else {
        Err(capability_unavailable_error(vault_id))
    }
}

async fn acquire_mutation(
    vault_id: VaultId,
    control: &VaultControlBlock,
) -> Result<tokio::sync::OwnedMutexGuard<()>, VaultReadError> {
    control
        .acquire_mutation()
        .await
        .map_err(|error| crate::vault_read::runtime_error(vault_id, error))
}

/// Builds this Vault's authoritative index off the async runtime: like the
/// legacy write API's `current_index`, this is a synchronous full-Vault
/// filesystem scan and must never run directly on a tokio worker.
async fn authoritative_index(
    vault_id: VaultId,
    control: &VaultControlBlock,
) -> Result<VaultIndex, VaultReadError> {
    let control = control.clone();
    match tokio::task::spawn_blocking(move || {
        control
            .authoritative_index()
            .map_err(|error| crate::vault_read::runtime_error(vault_id, error))
    })
    .await
    {
        Ok(result) => result,
        Err(join_error) => Err(VaultReadError {
            code: "vault_read_unavailable".to_string(),
            message: format!("vault index build panicked: {join_error}"),
            vault_id: Some(vault_id),
            retryable: true,
        }),
    }
}

/// Builds this Vault's metadata-only catalog off the async runtime, like
/// `authoritative_index` but skipping the content-reading wikilink-graph pass
/// (`vault/links.rs`): `create_note` has no pre-write entry to fetch a slug
/// from, so it needs this before the write to compute one, but never needs
/// the link graph itself.
async fn authoritative_catalog(
    vault_id: VaultId,
    control: &VaultControlBlock,
) -> Result<VaultIndex, VaultReadError> {
    let control = control.clone();
    match tokio::task::spawn_blocking(move || {
        control
            .authoritative_catalog()
            .map_err(|error| crate::vault_read::runtime_error(vault_id, error))
    })
    .await
    {
        Ok(result) => result,
        Err(join_error) => Err(VaultReadError {
            code: "vault_read_unavailable".to_string(),
            message: format!("vault catalog build panicked: {join_error}"),
            vault_id: Some(vault_id),
            retryable: true,
        }),
    }
}

fn note_entry(
    vault_id: VaultId,
    index: &VaultIndex,
    slug: &str,
) -> Result<crate::vault::NoteEntry, ApiError> {
    let slug = slug.trim();
    if slug.is_empty() {
        return Err(note_not_found_error(vault_id, slug));
    }
    index
        .find_by_slug(slug)
        .cloned()
        .ok_or_else(|| note_not_found_error(vault_id, slug))
}

/// Refuse a write whose target path matches this Vault's own noise-exclusion
/// pattern: the index applies the same matcher, so the file would land on
/// disk yet be invisible to every read surface. Mirrors the legacy write
/// API's `reject_noise_write`, scoped to this Vault's own exclude patterns
/// rather than the instance-wide legacy scan config.
fn reject_noise_write(
    vault_id: VaultId,
    control: &VaultControlBlock,
    path: &str,
) -> Option<Response> {
    let exclude = match ExcludeMatcher::new(control.definition().exclude_patterns()) {
        Ok(exclude) => exclude,
        Err(error) => {
            return Some(crate::handlers::vaults::internal_error_response(
                error,
                Some(vault_id),
            ));
        }
    };
    exclude
        .is_excluded(std::path::Path::new(path.trim()), false)
        .then(|| respond(noise_write_error(vault_id, path)))
}

/// Run a synchronous vault write op on the blocking pool. Moves/deletes
/// rewrite every backlinking note, which must not stall a tokio worker; a
/// panic maps to a `WriteError` instead of unwinding the handler.
async fn run_write_op<T>(
    op: impl FnOnce() -> Result<T, WriteError> + Send + 'static,
) -> Result<T, WriteError>
where
    T: Send + 'static,
{
    tokio::task::spawn_blocking(op)
        .await
        .unwrap_or_else(|join_error| {
            Err(WriteError::Io(format!("write task panicked: {join_error}")))
        })
}

/// Every write op (`vault/write/notes.rs`) now sets `outcome.slug` itself
/// from its own pre-write catalog/index, so this no longer looks anything up
/// by re-walking the Vault: `layer` is derived straight from `layers`
/// (already in memory from that same pre-write fetch) via the identical
/// path-based `layer_for` a real index build uses, except a delete — which
/// leaves no note behind, so it always reports `None` regardless of path.
/// `trashed_path.is_some()` is currently only ever set by `delete_note`, so
/// it stands in for "this is a delete"; a future write op that starts
/// setting `trashed_path` for a non-delete reason would need to update this.
fn write_outcome_response(layers: &LayerMap, outcome: WriteOutcome) -> WriteOutcomeFields {
    let layer = if outcome.trashed_path.is_some() {
        None
    } else {
        outcome
            .relative_path
            .as_deref()
            .and_then(|relative_path| layers.layer_for(relative_path))
            .map(str::to_string)
    };
    WriteOutcomeFields {
        slug: outcome.slug,
        relative_path: outcome.relative_path,
        content_hash: outcome.content_hash,
        quality_warnings: outcome.quality_warnings,
        rewritten_notes: outcome.rewritten_notes,
        moved_assets: outcome.moved_assets,
        trashed_path: outcome.trashed_path,
        layer,
    }
}

struct WriteOutcomeFields {
    slug: Option<String>,
    relative_path: Option<String>,
    content_hash: Option<String>,
    quality_warnings: Vec<String>,
    rewritten_notes: usize,
    moved_assets: usize,
    trashed_path: Option<String>,
    layer: Option<String>,
}

/// Preserves the rejection's real status — e.g. `413` for a body over the
/// length limit, `422` for well-formed JSON missing a required field — same
/// as the shared `json_rejection_response` (`handlers/vaults.rs`), so
/// clients/proxies keying off status codes are not misled by a flattened
/// `400`. This returns a typed `Result` rather than a built `Response` so
/// callers here can propagate it with `?`/`match ... => return respond(error)`,
/// keeping every intermediate `Result` in this file small per the
/// `clippy::result_large_err` note at the top of the file —
/// `json_rejection_response` returns a built `Response` directly because its
/// own callers (`vaults.rs`, `vault_content.rs`) don't share that convention.
fn write_payload<T>(payload: Result<Json<T>, JsonRejection>) -> Result<T, ApiError> {
    match payload {
        Ok(Json(payload)) => Ok(payload),
        Err(rejection) => Err((
            rejection.status(),
            VaultApiError::new("invalid_request_body", rejection.body_text(), None, false),
        )),
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `POST /api/v1/vaults/{vault_id}/notes`
pub async fn vault_scoped_create_note_handler(
    State(state): State<AppState>,
    Path(raw_vault_id): Path<String>,
    payload: Result<Json<VaultCreateNoteRequest>, JsonRejection>,
) -> Response {
    let vault_id = match parse_vault_id(&raw_vault_id) {
        Ok(vault_id) => vault_id,
        Err(error) => return respond(bad_request_error(error)),
    };
    let payload = match write_payload(payload) {
        Ok(payload) => payload,
        Err(error) => return respond(error),
    };
    let relative_path = match non_empty_input(vault_id, "relative_path", payload.relative_path) {
        Ok(relative_path) => relative_path,
        Err(error) => return respond(error),
    };

    let control = match mutation_control_block(&state, vault_id) {
        Ok(control) => control,
        Err(error) => return vault_read_error_response(error),
    };
    if let Err(error) = ensure_mutable(vault_id, &control) {
        return respond(error);
    }
    if let Some(response) = reject_noise_write(vault_id, &control, &relative_path) {
        return response;
    }
    let _guard = match acquire_mutation(vault_id, &control).await {
        Ok(guard) => guard,
        Err(error) => return vault_read_error_response(error),
    };
    // A metadata-only catalog, not the full index: create_note has no
    // pre-write entry to read a slug from, so it needs this to compute one,
    // but never touches the (expensive, content-reading) wikilink graph.
    let catalog = match authoritative_catalog(vault_id, &control).await {
        Ok(catalog) => catalog,
        Err(error) => return vault_read_error_response(error),
    };
    let layers = catalog.layers.clone();
    let vault_path = control.vault_path().to_path_buf();
    let outcome = match run_write_op(move || {
        create_note(
            &vault_path,
            &relative_path,
            &payload.content,
            false,
            &catalog,
        )
    })
    .await
    {
        Ok(outcome) => outcome,
        Err(error) => return write_error_response(vault_id, error),
    };

    finalize_note_write_response(vault_id, &layers, outcome)
}

/// `PUT /api/v1/vaults/{vault_id}/notes/{slug}`
pub async fn vault_scoped_update_note_handler(
    State(state): State<AppState>,
    Path((raw_vault_id, slug)): Path<(String, String)>,
    payload: Result<Json<VaultUpdateNoteRequest>, JsonRejection>,
) -> Response {
    let vault_id = match parse_vault_id(&raw_vault_id) {
        Ok(vault_id) => vault_id,
        Err(error) => return respond(bad_request_error(error)),
    };
    let payload = match write_payload(payload) {
        Ok(payload) => payload,
        Err(error) => return respond(error),
    };

    let control = match mutation_control_block(&state, vault_id) {
        Ok(control) => control,
        Err(error) => return vault_read_error_response(error),
    };
    if let Err(error) = ensure_mutable(vault_id, &control) {
        return respond(error);
    }
    let _guard = match acquire_mutation(vault_id, &control).await {
        Ok(guard) => guard,
        Err(error) => return vault_read_error_response(error),
    };
    let index = match authoritative_index(vault_id, &control).await {
        Ok(index) => index,
        Err(error) => return vault_read_error_response(error),
    };
    let entry = match note_entry(vault_id, &index, &slug) {
        Ok(entry) => entry,
        Err(error) => return respond(error),
    };
    let outcome = match run_write_op(move || {
        update_note(&entry, &payload.content, &payload.expected_content_hash)
    })
    .await
    {
        Ok(outcome) => outcome,
        Err(error) => return write_error_response(vault_id, error),
    };

    finalize_note_write_response(vault_id, &index.layers, outcome)
}

/// `PATCH /api/v1/vaults/{vault_id}/notes/{slug}/rename`
pub async fn vault_scoped_rename_note_handler(
    State(state): State<AppState>,
    Path((raw_vault_id, slug)): Path<(String, String)>,
    payload: Result<Json<VaultRenameNoteRequest>, JsonRejection>,
) -> Response {
    let vault_id = match parse_vault_id(&raw_vault_id) {
        Ok(vault_id) => vault_id,
        Err(error) => return respond(bad_request_error(error)),
    };
    let payload = match write_payload(payload) {
        Ok(payload) => payload,
        Err(error) => return respond(error),
    };
    let new_title = match non_empty_input(vault_id, "new_title", payload.new_title) {
        Ok(new_title) => new_title,
        Err(error) => return respond(error),
    };
    if new_title.contains('/') || new_title.contains('\\') {
        return VaultApiError::new(
            "invalid_write_input",
            "new_title cannot contain path separators",
            Some(vault_id),
            false,
        )
        .respond(StatusCode::BAD_REQUEST);
    }

    let control = match mutation_control_block(&state, vault_id) {
        Ok(control) => control,
        Err(error) => return vault_read_error_response(error),
    };
    if let Err(error) = ensure_mutable(vault_id, &control) {
        return respond(error);
    }
    let _guard = match acquire_mutation(vault_id, &control).await {
        Ok(guard) => guard,
        Err(error) => return vault_read_error_response(error),
    };
    let index = match authoritative_index(vault_id, &control).await {
        Ok(index) => index,
        Err(error) => return vault_read_error_response(error),
    };
    let entry = match note_entry(vault_id, &index, &slug) {
        Ok(entry) => entry,
        Err(error) => return respond(error),
    };
    let target_relative_path = replace_filename(&entry.relative_path, &new_title);
    if let Some(response) = reject_noise_write(vault_id, &control, &target_relative_path) {
        return response;
    }
    // The write closure below moves `index` in (it must own it to cross onto
    // the blocking pool), so its `LayerMap` — all the response needs — is
    // cloned out first rather than paying for a second index build after.
    let layers = index.layers.clone();
    let vault_path = control.vault_path().to_path_buf();
    let outcome = match run_write_op(move || {
        move_or_rename_note(
            &vault_path,
            &index,
            &entry,
            &target_relative_path,
            &payload.expected_content_hash,
        )
    })
    .await
    {
        Ok(outcome) => outcome,
        Err(error) => return write_error_response(vault_id, error),
    };

    finalize_note_write_response(vault_id, &layers, outcome)
}

/// `PATCH /api/v1/vaults/{vault_id}/notes/{slug}/move`
pub async fn vault_scoped_move_note_handler(
    State(state): State<AppState>,
    Path((raw_vault_id, slug)): Path<(String, String)>,
    payload: Result<Json<VaultMoveNoteRequest>, JsonRejection>,
) -> Response {
    let vault_id = match parse_vault_id(&raw_vault_id) {
        Ok(vault_id) => vault_id,
        Err(error) => return respond(bad_request_error(error)),
    };
    let payload = match write_payload(payload) {
        Ok(payload) => payload,
        Err(error) => return respond(error),
    };

    let control = match mutation_control_block(&state, vault_id) {
        Ok(control) => control,
        Err(error) => return vault_read_error_response(error),
    };
    if let Err(error) = ensure_mutable(vault_id, &control) {
        return respond(error);
    }
    let _guard = match acquire_mutation(vault_id, &control).await {
        Ok(guard) => guard,
        Err(error) => return vault_read_error_response(error),
    };
    let index = match authoritative_index(vault_id, &control).await {
        Ok(index) => index,
        Err(error) => return vault_read_error_response(error),
    };
    let entry = match note_entry(vault_id, &index, &slug) {
        Ok(entry) => entry,
        Err(error) => return respond(error),
    };
    let target_folder = payload.target_folder.trim().trim_matches('/');
    let file_name = entry
        .relative_path
        .rsplit('/')
        .next()
        .unwrap_or(&entry.relative_path);
    let target_relative_path = if target_folder.is_empty() {
        file_name.to_string()
    } else {
        format!("{target_folder}/{file_name}")
    };
    if let Some(response) = reject_noise_write(vault_id, &control, &target_relative_path) {
        return response;
    }
    let layers = index.layers.clone();
    let vault_path = control.vault_path().to_path_buf();
    let expected_content_hash = payload.expected_content_hash;
    let outcome = match run_write_op(move || {
        move_or_rename_note(
            &vault_path,
            &index,
            &entry,
            &target_relative_path,
            &expected_content_hash,
        )
    })
    .await
    {
        Ok(outcome) => outcome,
        Err(error) => return write_error_response(vault_id, error),
    };

    finalize_note_write_response(vault_id, &layers, outcome)
}

/// `PATCH /api/v1/vaults/{vault_id}/notes/{slug}/move-rename`
pub async fn vault_scoped_move_rename_note_handler(
    State(state): State<AppState>,
    Path((raw_vault_id, slug)): Path<(String, String)>,
    payload: Result<Json<VaultMoveRenameNoteRequest>, JsonRejection>,
) -> Response {
    let vault_id = match parse_vault_id(&raw_vault_id) {
        Ok(vault_id) => vault_id,
        Err(error) => return respond(bad_request_error(error)),
    };
    let payload = match write_payload(payload) {
        Ok(payload) => payload,
        Err(error) => return respond(error),
    };
    let target_relative_path = match non_empty_input(
        vault_id,
        "target_relative_path",
        payload.target_relative_path,
    ) {
        Ok(target_relative_path) => target_relative_path,
        Err(error) => return respond(error),
    };

    let control = match mutation_control_block(&state, vault_id) {
        Ok(control) => control,
        Err(error) => return vault_read_error_response(error),
    };
    if let Err(error) = ensure_mutable(vault_id, &control) {
        return respond(error);
    }
    let _guard = match acquire_mutation(vault_id, &control).await {
        Ok(guard) => guard,
        Err(error) => return vault_read_error_response(error),
    };
    let index = match authoritative_index(vault_id, &control).await {
        Ok(index) => index,
        Err(error) => return vault_read_error_response(error),
    };
    let entry = match note_entry(vault_id, &index, &slug) {
        Ok(entry) => entry,
        Err(error) => return respond(error),
    };
    if let Some(response) = reject_noise_write(vault_id, &control, &target_relative_path) {
        return response;
    }
    let layers = index.layers.clone();
    let vault_path = control.vault_path().to_path_buf();
    let outcome = match run_write_op(move || {
        move_or_rename_note(
            &vault_path,
            &index,
            &entry,
            &target_relative_path,
            &payload.expected_content_hash,
        )
    })
    .await
    {
        Ok(outcome) => outcome,
        Err(error) => return write_error_response(vault_id, error),
    };

    finalize_note_write_response(vault_id, &layers, outcome)
}

/// `PATCH /api/v1/vaults/{vault_id}/notes/{slug}/archive`
pub async fn vault_scoped_archive_note_handler(
    State(state): State<AppState>,
    Path((raw_vault_id, slug)): Path<(String, String)>,
    payload: Result<Json<VaultArchiveNoteRequest>, JsonRejection>,
) -> Response {
    let vault_id = match parse_vault_id(&raw_vault_id) {
        Ok(vault_id) => vault_id,
        Err(error) => return respond(bad_request_error(error)),
    };
    let payload = match write_payload(payload) {
        Ok(payload) => payload,
        Err(error) => return respond(error),
    };

    let control = match mutation_control_block(&state, vault_id) {
        Ok(control) => control,
        Err(error) => return vault_read_error_response(error),
    };
    if let Err(error) = ensure_mutable(vault_id, &control) {
        return respond(error);
    }
    let _guard = match acquire_mutation(vault_id, &control).await {
        Ok(guard) => guard,
        Err(error) => return vault_read_error_response(error),
    };
    let index = match authoritative_index(vault_id, &control).await {
        Ok(index) => index,
        Err(error) => return vault_read_error_response(error),
    };
    let entry = match note_entry(vault_id, &index, &slug) {
        Ok(entry) => entry,
        Err(error) => return respond(error),
    };
    // The Vault's own configured archive folder overrides the instance-wide
    // setting when present (#130).
    let snapshot = state.runtime_snapshot();
    let archive_prefix = match AppState::vault_archive_prefix(Some(control.definition()), &snapshot)
    {
        Ok(prefix) => prefix,
        Err(error) => {
            return crate::handlers::vaults::internal_error_response(error, Some(vault_id));
        }
    };
    let archive_folder = archive_prefix.trim().trim_matches('/');
    let file_name = entry
        .relative_path
        .rsplit('/')
        .next()
        .unwrap_or(&entry.relative_path);
    let target_relative_path = format!("{archive_folder}/{file_name}");
    if let Some(response) = reject_noise_write(vault_id, &control, &target_relative_path) {
        return response;
    }
    let layers = index.layers.clone();
    let vault_path = control.vault_path().to_path_buf();
    let outcome = match run_write_op(move || {
        archive_note(
            &vault_path,
            &index,
            &entry,
            &archive_prefix,
            &payload.expected_content_hash,
        )
    })
    .await
    {
        Ok(outcome) => outcome,
        Err(error) => return write_error_response(vault_id, error),
    };

    finalize_note_write_response(vault_id, &layers, outcome)
}

/// `DELETE /api/v1/vaults/{vault_id}/notes/{slug}`
pub async fn vault_scoped_delete_note_handler(
    State(state): State<AppState>,
    Path((raw_vault_id, slug)): Path<(String, String)>,
    payload: Result<Json<VaultDeleteNoteRequest>, JsonRejection>,
) -> Response {
    let vault_id = match parse_vault_id(&raw_vault_id) {
        Ok(vault_id) => vault_id,
        Err(error) => return respond(bad_request_error(error)),
    };
    let payload = match write_payload(payload) {
        Ok(payload) => payload,
        Err(error) => return respond(error),
    };

    let control = match mutation_control_block(&state, vault_id) {
        Ok(control) => control,
        Err(error) => return vault_read_error_response(error),
    };
    if let Err(error) = ensure_mutable(vault_id, &control) {
        return respond(error);
    }
    let _guard = match acquire_mutation(vault_id, &control).await {
        Ok(guard) => guard,
        Err(error) => return vault_read_error_response(error),
    };
    let index = match authoritative_index(vault_id, &control).await {
        Ok(index) => index,
        Err(error) => return vault_read_error_response(error),
    };
    let entry = match note_entry(vault_id, &index, &slug) {
        Ok(entry) => entry,
        Err(error) => return respond(error),
    };
    let layers = index.layers.clone();
    let vault_path = control.vault_path().to_path_buf();
    let outcome = match run_write_op(move || {
        delete_note(&vault_path, &index, &entry, &payload.expected_content_hash)
    })
    .await
    {
        Ok(outcome) => outcome,
        Err(error) => return write_error_response(vault_id, error),
    };

    finalize_note_write_response(vault_id, &layers, outcome)
}

/// Builds the mutation response straight from `layers` — the `LayerMap`
/// already sitting in memory from the write's own pre-write index/catalog
/// fetch — instead of rebuilding a fresh authoritative index after the write
/// has already committed to disk (issue #101: a rescan here would delay an
/// otherwise-completed mutation response, and could turn a rescan failure
/// into an HTTP error after the write already succeeded).
fn finalize_note_write_response(
    vault_id: VaultId,
    layers: &LayerMap,
    outcome: WriteOutcome,
) -> Response {
    let fields = write_outcome_response(layers, outcome);
    (
        StatusCode::OK,
        Json(VaultWriteOutcomeResponse {
            vault_id,
            ok: true,
            slug: fields.slug,
            relative_path: fields.relative_path,
            content_hash: fields.content_hash,
            quality_warnings: fields.quality_warnings,
            rewritten_notes: fields.rewritten_notes,
            moved_assets: fields.moved_assets,
            trashed_path: fields.trashed_path,
            layer: fields.layer,
        }),
    )
        .into_response()
}

/// `POST /api/v1/vaults/{vault_id}/attachments`
pub async fn vault_scoped_upload_attachment_handler(
    State(state): State<AppState>,
    Path(raw_vault_id): Path<String>,
    mut multipart: Multipart,
) -> Response {
    let vault_id = match parse_vault_id(&raw_vault_id) {
        Ok(vault_id) => vault_id,
        Err(error) => return respond(bad_request_error(error)),
    };

    // Bind the live setting before consuming a field. The static router guard
    // only protects the process-wide maximum because it cannot inspect the
    // snapshot selected for this request. Attachment size stays an
    // instance-wide setting (issue #62), not per-Vault.
    let snapshot = state.runtime_snapshot();
    let max_attachment_bytes = match AppState::runtime_mcp_config(&snapshot) {
        Ok(config) => config.max_attachment_bytes,
        Err(error) => {
            return crate::handlers::vaults::internal_error_response(error, Some(vault_id));
        }
    };

    let mut target_relative_path: Option<String> = None;
    let mut file_bytes: Option<Vec<u8>> = None;
    while let Some(field) = match multipart.next_field().await {
        Ok(field) => field,
        Err(error) => {
            return VaultApiError::new(
                "invalid_write_input",
                format!("invalid multipart upload: {error}"),
                Some(vault_id),
                false,
            )
            .respond(StatusCode::BAD_REQUEST);
        }
    } {
        let name = field.name().unwrap_or("").to_string();
        if name == "target_relative_path" {
            let value = match read_multipart_field(field, MAX_TARGET_RELATIVE_PATH_BYTES).await {
                Ok(value) => value,
                Err(error) => {
                    return VaultApiError::new(
                        "invalid_write_input",
                        format!("invalid target_relative_path field: {error}"),
                        Some(vault_id),
                        false,
                    )
                    .respond(StatusCode::BAD_REQUEST);
                }
            };
            let value = match String::from_utf8(value) {
                Ok(value) => value,
                Err(_) => {
                    return VaultApiError::new(
                        "invalid_write_input",
                        "target_relative_path must be valid UTF-8".to_string(),
                        Some(vault_id),
                        false,
                    )
                    .respond(StatusCode::BAD_REQUEST);
                }
            };
            target_relative_path = Some(value);
        } else if name == "file" {
            let bytes = match read_multipart_field(field, max_attachment_bytes).await {
                Ok(bytes) => bytes,
                Err(error) => {
                    return VaultApiError::new(
                        "invalid_write_input",
                        format!("invalid file field: {error}"),
                        Some(vault_id),
                        false,
                    )
                    .respond(StatusCode::BAD_REQUEST);
                }
            };
            file_bytes = Some(bytes);
        }
    }

    let target_relative_path = match non_empty_input(
        vault_id,
        "target_relative_path",
        target_relative_path.unwrap_or_default(),
    ) {
        Ok(path) => path,
        Err(error) => return respond(error),
    };
    let file_bytes = match file_bytes {
        Some(bytes) if !bytes.is_empty() => bytes,
        _ => return respond(invalid_input_error(vault_id, "file")),
    };

    let control = match mutation_control_block(&state, vault_id) {
        Ok(control) => control,
        Err(error) => return vault_read_error_response(error),
    };
    if let Err(error) = ensure_mutable(vault_id, &control) {
        return respond(error);
    }
    if let Some(response) = reject_noise_write(vault_id, &control, &target_relative_path) {
        return response;
    }
    let _guard = match acquire_mutation(vault_id, &control).await {
        Ok(guard) => guard,
        Err(error) => return vault_read_error_response(error),
    };
    let vault_path = control.vault_path().to_path_buf();
    let outcome = match run_write_op(move || {
        import_attachment_bytes(
            &vault_path,
            &target_relative_path,
            &file_bytes,
            max_attachment_bytes,
            false,
        )
    })
    .await
    {
        Ok(outcome) => outcome,
        Err(error) => return write_error_response(vault_id, error),
    };

    attachment_outcome_response(vault_id, outcome)
}

/// Field names are protocol metadata, never a second unbounded upload body.
const MAX_TARGET_RELATIVE_PATH_BYTES: u64 = 16 * 1024;

/// Consume multipart fields incrementally. Axum's convenient `text`/`bytes`
/// methods collect a whole field before returning, which would let a lowered
/// live attachment limit be bypassed until allocation had already happened.
async fn read_multipart_field(
    mut field: axum::extract::multipart::Field<'_>,
    max_bytes: u64,
) -> Result<Vec<u8>, String> {
    let capacity = usize::try_from(max_bytes.min(64 * 1024)).unwrap_or(64 * 1024);
    let mut output = Vec::with_capacity(capacity);
    while let Some(chunk) = field.chunk().await.map_err(|error| error.body_text())? {
        let next_len = output
            .len()
            .checked_add(chunk.len())
            .ok_or_else(|| "multipart field is too large".to_string())?;
        if next_len as u64 > max_bytes {
            return Err(format!(
                "multipart field exceeds the {max_bytes}-byte limit"
            ));
        }
        output.extend_from_slice(&chunk);
    }
    Ok(output)
}

fn attachment_outcome_response(vault_id: VaultId, outcome: AttachmentOutcome) -> Response {
    (
        StatusCode::OK,
        Json(VaultAttachmentOutcomeResponse {
            vault_id,
            ok: true,
            attachment: outcome.attachment,
            rewritten_notes: outcome.rewritten_notes,
            trashed_path: outcome.trashed_path,
            cleanup_warning: outcome.cleanup_warning,
        }),
    )
        .into_response()
}

/// `GET /api/v1/vaults/{vault_id}/write-capabilities`
pub async fn vault_scoped_write_capabilities_handler(
    State(state): State<AppState>,
    Path(raw_vault_id): Path<String>,
) -> Response {
    let vault_id = match parse_vault_id(&raw_vault_id) {
        Ok(vault_id) => vault_id,
        Err(error) => return respond(bad_request_error(error)),
    };
    let control = match mutation_control_block(&state, vault_id) {
        Ok(control) => control,
        Err(error) => return vault_read_error_response(error),
    };
    let core = VaultReadCore::new(&state.startup_sqlite, &state.vaults);
    let vault_path = match core.vault_directory(vault_id) {
        Ok(path) => path,
        Err(error) => return vault_read_error_response(error),
    };
    let vault_writable = std::fs::metadata(&vault_path)
        .map(|metadata| !metadata.permissions().readonly())
        .unwrap_or(false);
    let mutate_capable = control.snapshot().capabilities.mutate;

    let mut warnings = Vec::new();
    if mutate_capable && vault_writable && !state.web_auth_enabled {
        warnings.push(
            "Frontend writes are enabled without requiring Hatchdoor web authentication; this is unauthenticated and should not be exposed to untrusted networks.".to_string(),
        );
    }
    if !vault_writable {
        warnings
            .push("Vault path is not writable; browser write features are disabled.".to_string());
    }
    if !mutate_capable {
        warnings
            .push("This Vault's current source and lifecycle do not allow mutation.".to_string());
    }

    (
        StatusCode::OK,
        Json(VaultWriteCapabilitiesResponse {
            vault_id,
            enabled: mutate_capable && vault_writable,
            warnings,
        }),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn run_write_op_passes_through_the_ops_result() {
        let ok: Result<u32, WriteError> = run_write_op(|| Ok(7)).await;
        assert_eq!(ok, Ok(7));

        let conflict: Result<u32, WriteError> =
            run_write_op(|| Err(WriteError::Conflict("taken".to_string()))).await;
        assert_eq!(conflict, Err(WriteError::Conflict("taken".to_string())));
    }

    #[tokio::test]
    async fn run_write_op_maps_a_panicking_op_to_an_io_write_error() {
        let result: Result<(), WriteError> = run_write_op(|| panic!("boom")).await;
        match result {
            Err(WriteError::Io(message)) => assert!(message.contains("panicked")),
            other => panic!("expected Io error, got {other:?}"),
        }
    }

    #[test]
    fn replace_filename_keeps_the_directory_and_swaps_only_the_stem() {
        assert_eq!(
            replace_filename("notes/Home.md", "Renamed"),
            "notes/Renamed.md"
        );
        assert_eq!(replace_filename("Home.md", "Renamed"), "Renamed.md");
    }

    #[test]
    fn write_error_response_maps_issue_62_http_meaning_buckets() {
        let vault_id = VaultId::generate().expect("generate Vault id");
        let conflict =
            write_error_response(vault_id, WriteError::Conflict("stale hash".to_string()));
        assert_eq!(conflict.status(), StatusCode::CONFLICT);

        let invalid =
            write_error_response(vault_id, WriteError::InvalidInput("bad path".to_string()));
        assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

        let io = write_error_response(vault_id, WriteError::Io("disk full".to_string()));
        assert_eq!(io.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn write_outcome_response_derives_layer_from_the_layer_map_alone() {
        // issue #101: `write_outcome_response` takes a `&LayerMap`, not a
        // `&VaultIndex` — it structurally cannot rebuild a full authoritative
        // index (there is no index for it to rebuild), so the second
        // post-write rescan the issue flags is not just avoided but
        // impossible to reintroduce here by accident.
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let root = tmp.path();
        std::fs::create_dir_all(root.join("sources")).expect("mkdir");
        std::fs::write(root.join("sources/.hatchdoor-layer"), "name: sources\n").expect("marker");
        let layers = LayerMap::collect(root).expect("collect layers");

        let updated = WriteOutcome {
            slug: Some("clip".to_string()),
            relative_path: Some("sources/Clip".to_string()),
            content_hash: Some("h".to_string()),
            quality_warnings: Vec::new(),
            rewritten_notes: 0,
            moved_assets: 0,
            trashed_path: None,
            affected_paths: Vec::new(),
        };
        let fields = write_outcome_response(&layers, updated);
        assert_eq!(fields.slug.as_deref(), Some("clip"));
        assert_eq!(fields.layer.as_deref(), Some("sources"));

        let deleted = WriteOutcome {
            slug: Some("clip".to_string()),
            relative_path: Some("sources/Clip".to_string()),
            content_hash: None,
            quality_warnings: Vec::new(),
            rewritten_notes: 0,
            moved_assets: 0,
            trashed_path: Some(".hatchdoor-trash/sources/Clip".to_string()),
            affected_paths: Vec::new(),
        };
        let fields = write_outcome_response(&layers, deleted);
        assert_eq!(
            fields.layer, None,
            "a delete leaves no note behind, so layer stays null even though the path matches a named layer"
        );
    }
}
