//! `/api/v1/vaults/{vault_id}/...` — exactly-one-Vault Markdown mutations,
//! attachment upload, and write-capabilities discovery.
//!
//! Since #186 every route here is HTTP shaping and nothing else: parse the
//! path and body, hand them to the Vault mutation core
//! ([`crate::vault_mutation`]), and turn the typed outcome or the structured
//! error into a status code and a JSON body. The Vault gate, the per-Vault
//! mutation lock, the index build, the slug lookup, the marker and noise
//! refusals, the archive prefix, the off-runtime write, and the write-error
//! translation all live there, shared with the MCP write tools (ADR-19,
//! ADR-03). The one exception is the attachment upload, which reads its own
//! multipart stream: the byte limit has to be bound and enforced while the
//! field is consumed, so that discipline stays where the bytes arrive and the
//! core takes over once there are decoded bytes to write.
//!
//! It reuses `handlers/vaults.rs`'s `VaultApiError`/`parse_vault_id`/rejection
//! helpers and `handlers/vault_content.rs`'s `vault_read_error_response`,
//! mounted alongside them in the same router group and sharing its auth
//! posture. Every route here is a content mutation or write-capability
//! discovery, so `src/server.rs` wraps each one in `reject_demo_mutation`
//! (#109): in demo mode it refuses with the shared `403 demo_read_only` error
//! before running, unlike `vault_content.rs`'s exact reads and
//! `vault_collection_reads.rs`'s one-or-all reads, which stay reachable
//! unauthenticated in demo mode. The literal `all` is never accepted
//! (mutations always name exactly one Vault ID — issue #62).
//!
//! Unlike the legacy single-Vault write API, a mutation response never
//! includes a `git_sync_warning`: the managed-Git scheduler has no
//! debounced-on-write hook (it runs on its own poll/manual schedule), so
//! there is nothing per-mutation to report that `GET /api/v1/vaults` does not
//! already expose through that Vault's own `git` status.
//!
//! Internal helpers propagate a small typed `(StatusCode, VaultApiError)` pair
//! rather than a built `Response`, mirroring `vault_content.rs`'s convention —
//! `axum::response::Response` is large enough to trip
//! `clippy::result_large_err`, and building the response body only at the
//! outermost point keeps every intermediate `Result` small.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Multipart, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use crate::app_state::AppState;
use crate::handlers::vault_content::vault_read_error_response;
use crate::handlers::vaults::{VaultApiError, parse_vault_id};
use crate::vault::{AttachmentInfo, AttachmentOutcome};
use crate::vault_error::VaultOperationError;
use crate::vault_mutation::{NoteWriteOutcome, VaultMutationCore};
use crate::vault_read::VaultReadError;
use crate::vault_registry::VaultId;

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

/// The HTTP half of ADR-19's mapping for a Vault mutation: one structured
/// core error becomes a status code plus the same `{code, message, vault_id,
/// retryable}` body the surface has always returned. Codes the mutation core
/// does not raise — the Vault-resolution and index-build failures it inherits
/// from the read core — fall through to the shared read bucket, so a mutation
/// reports a missing, disabled, or unavailable Vault exactly as a read does.
fn mutation_error_response(error: VaultOperationError) -> Response {
    let status = match error.code.as_str() {
        // `write_failed` carries the underlying I/O detail, which this surface
        // has always sanitized away; MCP reports it verbatim under that code.
        "write_failed" | "internal_error" => {
            return crate::handlers::vaults::internal_error_response(error.message, error.vault_id);
        }
        // A partially-applied multi-phase mutation needs operator action, so
        // its message survives rather than collapsing into the generic
        // sanitized internal error.
        "write_recovery_required" => StatusCode::INTERNAL_SERVER_ERROR,
        "write_conflict" | "capability_unavailable" => StatusCode::CONFLICT,
        "invalid_write_input" | "noise_excluded_write" | "layer_marker_write" => {
            StatusCode::BAD_REQUEST
        }
        "note_not_found" => StatusCode::NOT_FOUND,
        _ => {
            return vault_read_error_response(VaultReadError {
                code: error.code,
                message: error.message,
                vault_id: error.vault_id,
                retryable: error.retryable,
            });
        }
    };
    error.respond(status)
}

/// The success half of that mapping: the core's typed outcome, already
/// carrying its resolved layer, shaped into this route's response body.
fn note_write_response(vault_id: VaultId, outcome: NoteWriteOutcome) -> Response {
    (
        StatusCode::OK,
        Json(VaultWriteOutcomeResponse {
            vault_id,
            ok: true,
            slug: outcome.slug,
            relative_path: outcome.relative_path,
            content_hash: outcome.content_hash,
            quality_warnings: outcome.quality_warnings,
            rewritten_notes: outcome.rewritten_notes,
            moved_assets: outcome.moved_assets,
            trashed_path: outcome.trashed_path,
            layer: outcome.layer,
        }),
    )
        .into_response()
}

fn respond((status, error): ApiError) -> Response {
    error.respond(status)
}

fn bad_request_error(error: VaultApiError) -> ApiError {
    (StatusCode::BAD_REQUEST, error)
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

/// The one argument check both this surface and MCP still make for
/// themselves: an omitted-but-present field is a transport-shaped complaint,
/// and the two transports word it differently. Everything past this point is
/// the core's.
fn non_empty_input(vault_id: VaultId, field: &str, value: String) -> Result<String, ApiError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(invalid_input_error(vault_id, field));
    }
    Ok(trimmed.to_string())
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

    match VaultMutationCore::from_state(&state)
        .create_note(vault_id, &relative_path, &payload.content, false)
        .await
    {
        Ok(outcome) => note_write_response(vault_id, outcome),
        Err(error) => mutation_error_response(error),
    }
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

    match VaultMutationCore::from_state(&state)
        .update_note(
            vault_id,
            &slug,
            &payload.content,
            &payload.expected_content_hash,
        )
        .await
    {
        Ok(outcome) => note_write_response(vault_id, outcome),
        Err(error) => mutation_error_response(error),
    }
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
    // Adapter-owned because the two transports word it differently: this
    // surface reports `invalid_write_input`/`400`, MCP an invalid parameter
    // (`mcp/tools/write.rs`'s `rename_note_tool` carries the same rule).
    if new_title.contains('/') || new_title.contains('\\') {
        return respond((
            StatusCode::BAD_REQUEST,
            VaultApiError::new(
                "invalid_write_input",
                "new_title cannot contain path separators",
                Some(vault_id),
                false,
            ),
        ));
    }

    match VaultMutationCore::from_state(&state)
        .rename_note(vault_id, &slug, &new_title, &payload.expected_content_hash)
        .await
    {
        Ok(outcome) => note_write_response(vault_id, outcome),
        Err(error) => mutation_error_response(error),
    }
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

    match VaultMutationCore::from_state(&state)
        .move_note(
            vault_id,
            &slug,
            &payload.target_folder,
            &payload.expected_content_hash,
        )
        .await
    {
        Ok(outcome) => note_write_response(vault_id, outcome),
        Err(error) => mutation_error_response(error),
    }
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

    match VaultMutationCore::from_state(&state)
        .move_rename_note(
            vault_id,
            &slug,
            &target_relative_path,
            &payload.expected_content_hash,
        )
        .await
    {
        Ok(outcome) => note_write_response(vault_id, outcome),
        Err(error) => mutation_error_response(error),
    }
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

    match VaultMutationCore::from_state(&state)
        .archive_note(vault_id, &slug, &payload.expected_content_hash)
        .await
    {
        Ok(outcome) => note_write_response(vault_id, outcome),
        Err(error) => mutation_error_response(error),
    }
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

    match VaultMutationCore::from_state(&state)
        .delete_note(vault_id, &slug, &payload.expected_content_hash)
        .await
    {
        Ok(outcome) => note_write_response(vault_id, outcome),
        Err(error) => mutation_error_response(error),
    }
}

/// `POST /api/v1/vaults/{vault_id}/attachments`
///
/// The only route here that reads its body itself. Multipart fields arrive as
/// a stream, and the byte limit must be bound and enforced *while* they are
/// read rather than after, so that discipline stays where the bytes arrive;
/// the core takes over once there are decoded bytes to write.
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

    match VaultMutationCore::from_state(&state)
        .import_attachment(
            vault_id,
            &target_relative_path,
            file_bytes,
            max_attachment_bytes,
            false,
        )
        .await
    {
        Ok(outcome) => attachment_outcome_response(vault_id, outcome),
        Err(error) => mutation_error_response(error),
    }
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
///
/// The core reports what the Vault itself permits; the operator-facing
/// `warnings` are this surface's own, because only a browser client cares
/// that the instance serves writes without web authentication.
pub async fn vault_scoped_write_capabilities_handler(
    State(state): State<AppState>,
    Path(raw_vault_id): Path<String>,
) -> Response {
    let vault_id = match parse_vault_id(&raw_vault_id) {
        Ok(vault_id) => vault_id,
        Err(error) => return respond(bad_request_error(error)),
    };
    let capabilities = match VaultMutationCore::from_state(&state).write_capabilities(vault_id) {
        Ok(capabilities) => capabilities,
        Err(error) => return mutation_error_response(error),
    };

    let mut warnings = Vec::new();
    if capabilities.enabled() && !state.web_auth_enabled {
        warnings.push(
            "Frontend writes are enabled without requiring Hatchdoor web authentication; this is unauthenticated and should not be exposed to untrusted networks.".to_string(),
        );
    }
    if !capabilities.vault_writable {
        warnings
            .push("Vault path is not writable; browser write features are disabled.".to_string());
    }
    if !capabilities.mutate_capable {
        warnings
            .push("This Vault's current source and lifecycle do not allow mutation.".to_string());
    }

    (
        StatusCode::OK,
        Json(VaultWriteCapabilitiesResponse {
            vault_id,
            enabled: capabilities.enabled(),
            warnings,
        }),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutation_error_response_maps_every_core_code_this_surface_can_receive() {
        // ADR-19: the core reports one structured error and this adapter owns
        // the status code. `write_failed` carries the underlying I/O detail,
        // which this surface has always sanitized away; MCP reports it.
        let vault_id = VaultId::generate().expect("generate Vault id");
        let map = |code: &str| {
            mutation_error_response(VaultOperationError::new(
                code,
                "detail",
                Some(vault_id),
                false,
            ))
        };

        assert_eq!(map("write_conflict").status(), StatusCode::CONFLICT);
        assert_eq!(map("capability_unavailable").status(), StatusCode::CONFLICT);
        assert_eq!(map("invalid_write_input").status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            map("noise_excluded_write").status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(map("layer_marker_write").status(), StatusCode::BAD_REQUEST);
        assert_eq!(map("note_not_found").status(), StatusCode::NOT_FOUND);
        assert_eq!(
            map("write_recovery_required").status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            map("internal_error").status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            map("write_failed").status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        // Codes the mutation core inherits from the read core fall through to
        // the shared read bucket, so a mutation reports an unreachable Vault
        // exactly as a read of that Vault does.
        assert_eq!(map("vault_not_found").status(), StatusCode::NOT_FOUND);
        assert_eq!(map("vault_disabled").status(), StatusCode::CONFLICT);
        assert_eq!(
            map("vault_read_unavailable").status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[tokio::test]
    async fn mutation_error_response_sanitizes_only_the_two_internal_codes() {
        let vault_id = VaultId::generate().expect("generate Vault id");
        async fn body(vault_id: VaultId, code: &str) -> serde_json::Value {
            let response = mutation_error_response(VaultOperationError::new(
                code,
                "disk full at /srv/vaults/secret",
                Some(vault_id),
                false,
            ));
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("body");
            serde_json::from_slice::<serde_json::Value>(&bytes).expect("json")
        }

        let sanitized = body(vault_id, "write_failed").await;
        assert_eq!(sanitized["code"], "internal_error");
        assert_eq!(sanitized["message"], "Internal server error");

        // A partially-applied multi-phase mutation needs operator action, so
        // its message must survive rather than collapse into the generic one.
        let recovery = body(vault_id, "write_recovery_required").await;
        assert_eq!(recovery["code"], "write_recovery_required");
        assert_eq!(recovery["message"], "disk full at /srv/vaults/secret");
    }

    #[tokio::test]
    async fn note_write_response_carries_the_cores_outcome_verbatim() {
        let vault_id = VaultId::generate().expect("generate Vault id");
        let response = note_write_response(
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
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let body: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
        assert_eq!(body["ok"], true);
        assert_eq!(body["vault_id"], vault_id.to_string());
        assert_eq!(body["slug"], "clip");
        assert_eq!(body["relative_path"], "sources/Clip");
        assert_eq!(body["content_hash"], "h");
        assert_eq!(body["layer"], "sources");
        assert_eq!(body["quality_warnings"][0], "warn");
        assert_eq!(body["rewritten_notes"], 2);
        assert_eq!(body["moved_assets"], 1);
    }
}
