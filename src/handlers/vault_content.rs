//! `/api/v1/vaults/{vault_id}/...` — exact note, link, and resolution reads,
//! canonical Note identity, and the contained asset/attachment/download
//! resources for exactly one Vault.
//!
//! This is a Vault-scoped adapter over [`crate::vault_read::VaultReadCore`]'s
//! exact-read methods, mounted alongside `handlers/vaults.rs`'s collection
//! surface and reusing its established `VaultApiError{code, message,
//! vault_id?, retryable}` shape and HTTP-status conventions. Every operation
//! here targets exactly one Vault ID; the literal `all` is not accepted (that
//! is the one-or-all collection-read surface owned by #100). Exact reads
//! always inspect the requested Vault's authoritative Markdown directory
//! (never the disposable shared cache), so indexing lag never applies to
//! them. Every route here is a read, so none of them is wrapped in
//! `reject_demo_mutation` (#109): they stay reachable unauthenticated in demo
//! mode, unlike the mutation and Vault-control routes in
//! `vaults.rs`/`vault_write.rs`.

use axum::Json;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::api_types::{
    ResolveAssetResult, ResolveBatchRequest, ResolveQuery, ResolveTargetResult,
};
use crate::app_state::{AppState, internal_error};
use crate::handlers::api::MAX_RESOLVE_BATCH;
use crate::handlers::assets::{asset_error_parts, asset_response};
use crate::handlers::downloads::{ExportError, NoteExport, build_note_export, download_response};
use crate::handlers::vaults::{
    VaultApiError, internal_error_response, json_rejection_response, parse_vault_id,
    query_rejection_response,
};
use crate::vault_read::{
    AssetPathError, AssetReadError, OffloadedReadError, VaultReadError, VaultReads,
    VaultResolveResponse,
};
use crate::vault_registry::VaultId;

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct VaultResolveBatchResponse {
    pub vault_id: VaultId,
    pub results: Vec<ResolveTargetResult>,
    /// Empty unless the request carried `asset_targets` (#158), so a client
    /// resolving note links only sees exactly what it saw before.
    pub asset_results: Vec<ResolveAssetResult>,
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// The Vault-relative folder containing `note_path`, `""` at the Vault root.
/// Backslashes are folded first so a client that sends a Windows-style path
/// does not resolve every asset from the root.
fn note_parent_dir(note_path: &str) -> String {
    let normalized = note_path.replace('\\', "/");
    let mut parts = normalized
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    parts.pop();
    parts.join("/")
}

fn bad_request(error: VaultApiError) -> Response {
    error.respond(StatusCode::BAD_REQUEST)
}

fn note_not_found_response(vault_id: VaultId, slug: &str) -> Response {
    crate::vault_read::note_not_found(vault_id, slug).respond(StatusCode::NOT_FOUND)
}

/// Maps one offloaded read's failure onto a response: the Vault's own
/// structured failure through the bucket map below, and a blocking task that
/// never completed through the same opaque `500` every other instance-side
/// fault reports.
fn read_error_response(error: OffloadedReadError) -> Response {
    match error {
        OffloadedReadError::Read(error) => vault_read_error_response(error),
        OffloadedReadError::Failed(message) => internal_error(message).into_response(),
    }
}

/// Maps a [`VaultReadError`] (from exact-read/asset-directory gating, and
/// from the one-or-all collection-read and search shared core in
/// `handlers/vault_collection_reads.rs`, which reuses this rather than
/// duplicating the same bucket logic) onto the shared `VaultApiError` shape
/// and issue #62's HTTP-meaning buckets: absence is `404`, a current-state
/// conflict (disabled) is `409`, malformed input is `400`, and every
/// transient-unavailability code is `503`. A malformed per-Vault scan
/// configuration is a `500` — it depends on saved exclusion patterns, not on
/// this request, so retrying the same request cannot help.
pub(crate) fn vault_read_error_response(error: VaultReadError) -> Response {
    let error = error.into_operation_error();
    let status = match error.code.as_str() {
        "vault_not_found" | "note_not_found" => StatusCode::NOT_FOUND,
        "vault_disabled" => StatusCode::CONFLICT,
        "vault_scan_config_invalid" => StatusCode::INTERNAL_SERVER_ERROR,
        "invalid_scope" | "invalid_search_query" | "invalid_layer_selection" => {
            StatusCode::BAD_REQUEST
        }
        _ => StatusCode::SERVICE_UNAVAILABLE,
    };
    error.respond(status)
}

/// Maps the read core's containment outcome onto the shared `VaultApiError`
/// shape, with the status `handlers/assets.rs` pairs with that outcome.
fn vault_asset_error_response(
    kind: AssetPathError,
    requested_path: &str,
    vault_id: VaultId,
) -> Response {
    let (code, status, message) = asset_error_parts(kind, requested_path);
    VaultApiError::new(code, message, Some(vault_id), false).respond(status)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/v1/vaults/{vault_id}/notes/{slug}`
pub async fn vault_scoped_note_handler(
    State(state): State<AppState>,
    Path((raw_vault_id, slug)): Path<(String, String)>,
) -> Response {
    let vault_id = match parse_vault_id(&raw_vault_id) {
        Ok(vault_id) => vault_id,
        Err(error) => return bad_request(error),
    };
    let lookup_slug = slug.clone();
    let result = VaultReads::new(&state)
        .read(move |core| core.exact_note(vault_id, &lookup_slug))
        .await;
    match result {
        Ok(Some(note)) => (StatusCode::OK, Json(note)).into_response(),
        Ok(None) => note_not_found_response(vault_id, &slug),
        Err(error) => read_error_response(error),
    }
}

/// `GET /api/v1/vaults/{vault_id}/notes/{slug}/links`
pub async fn vault_scoped_note_links_handler(
    State(state): State<AppState>,
    Path((raw_vault_id, slug)): Path<(String, String)>,
) -> Response {
    let vault_id = match parse_vault_id(&raw_vault_id) {
        Ok(vault_id) => vault_id,
        Err(error) => return bad_request(error),
    };
    let lookup_slug = slug.clone();
    let result = VaultReads::new(&state)
        .read(move |core| core.exact_note_links(vault_id, &lookup_slug))
        .await;
    match result {
        Ok(Some(links)) => (StatusCode::OK, Json(links)).into_response(),
        Ok(None) => note_not_found_response(vault_id, &slug),
        Err(error) => read_error_response(error),
    }
}

/// `GET /api/v1/vaults/{vault_id}/stats/detail` — the rich, exact single-Vault
/// statistics report `handlers/vault_collection_reads.rs`'s lean
/// `{scope}/stats` collection projection cannot back. Never `all`: a distinct
/// path from the collection route rather than a query flag, matching the
/// exact-vs-collection route split already established for every other pair
/// in this file/`vault_collection_reads.rs`.
pub async fn vault_scoped_stats_detail_handler(
    State(state): State<AppState>,
    Path(raw_vault_id): Path<String>,
) -> Response {
    let vault_id = match parse_vault_id(&raw_vault_id) {
        Ok(vault_id) => vault_id,
        Err(error) => return bad_request(error),
    };
    let result = VaultReads::new(&state)
        .read(move |core| core.statistics_detail(vault_id))
        .await;
    match result {
        Ok(stats) => (StatusCode::OK, Json(stats)).into_response(),
        Err(error) => read_error_response(error),
    }
}

enum DownloadOutcome {
    NotFound,
    ExportError(String),
    TooLarge,
    Export(NoteExport),
}

/// `GET /api/v1/vaults/{vault_id}/notes/{slug}/download`
pub async fn vault_scoped_note_download_handler(
    State(state): State<AppState>,
    Path((raw_vault_id, slug)): Path<(String, String)>,
) -> Response {
    let vault_id = match parse_vault_id(&raw_vault_id) {
        Ok(vault_id) => vault_id,
        Err(error) => return bad_request(error),
    };
    let lookup_slug = slug.clone();
    let result = VaultReads::new(&state)
        .read(move |core| {
            // Note content and its containing directory must come from the same
            // Vault control-block fetch: a concurrent edit reconciles a
            // *replacement* control block rather than mutating the current one
            // in place, so two independent lookups could otherwise pair this
            // note's content with a different Vault generation's directory.
            let Some((note, vault_root)) = core.exact_note_for_download(vault_id, &lookup_slug)?
            else {
                return Ok(DownloadOutcome::NotFound);
            };
            Ok(match build_note_export(&vault_root, &note.note) {
                Ok(export) => DownloadOutcome::Export(export),
                Err(ExportError::TooLarge) => DownloadOutcome::TooLarge,
                Err(ExportError::Failed(message)) => DownloadOutcome::ExportError(message),
            })
        })
        .await;

    let outcome = match result {
        Ok(outcome) => outcome,
        Err(error) => return read_error_response(error),
    };

    match outcome {
        DownloadOutcome::Export(export) => download_response(export),
        DownloadOutcome::NotFound => note_not_found_response(vault_id, &slug),
        DownloadOutcome::ExportError(message) => internal_error_response(message, Some(vault_id)),
        DownloadOutcome::TooLarge => VaultApiError::new(
            "note_export_too_large",
            "Note export exceeds the server download size limit".to_string(),
            Some(vault_id),
            false,
        )
        .respond(StatusCode::PAYLOAD_TOO_LARGE),
    }
}

/// `GET /api/v1/vaults/{vault_id}/resolve?target=...`
pub async fn vault_scoped_resolve_handler(
    State(state): State<AppState>,
    Path(raw_vault_id): Path<String>,
    query: Result<Query<ResolveQuery>, QueryRejection>,
) -> Response {
    let vault_id = match parse_vault_id(&raw_vault_id) {
        Ok(vault_id) => vault_id,
        Err(error) => return bad_request(error),
    };
    let Query(query) = match query {
        Ok(query) => query,
        Err(error) => return query_rejection_response(error),
    };
    let result = VaultReads::new(&state)
        .read(move |core| core.resolve_wikilink(vault_id, &query.target))
        .await;
    match result {
        Ok(resolved) => (
            StatusCode::OK,
            Json(VaultResolveResponse {
                vault_id,
                slug: resolved.map(|resolved| resolved.slug),
            }),
        )
            .into_response(),
        Err(error) => read_error_response(error),
    }
}

/// `POST /api/v1/vaults/{vault_id}/resolve-batch`
pub async fn vault_scoped_resolve_batch_handler(
    State(state): State<AppState>,
    Path(raw_vault_id): Path<String>,
    request: Result<Json<ResolveBatchRequest>, JsonRejection>,
) -> Response {
    let vault_id = match parse_vault_id(&raw_vault_id) {
        Ok(vault_id) => vault_id,
        Err(error) => return bad_request(error),
    };
    let Json(payload) = match request {
        Ok(payload) => payload,
        Err(error) => return json_rejection_response(error),
    };
    if payload.targets.len() + payload.asset_targets.len() > MAX_RESOLVE_BATCH {
        return VaultApiError::new(
            "resolve_batch_too_large",
            format!("Too many targets (max {MAX_RESOLVE_BATCH})"),
            Some(vault_id),
            false,
        )
        .respond(StatusCode::PAYLOAD_TOO_LARGE);
    }

    let reads = VaultReads::new(&state);
    let snapshot = state.runtime_snapshot();
    let vaults = state.vaults.clone();
    let control = vaults.runtime(vault_id);
    let archive_prefix = match AppState::vault_archive_prefix(
        control.as_ref().map(|control| control.definition()),
        &snapshot,
    ) {
        Ok(prefix) => prefix,
        Err(error) => return internal_error_response(error, Some(vault_id)),
    };

    let result = reads
        .read(move |core| {
            // One authoritative-index build for the whole batch: `resolve_batch`
            // resolves every target — note and asset alike — against it, rather
            // than paying a full Vault scan per target the way looping
            // `resolve_wikilink` would.
            let note_dir = payload
                .note_path
                .as_deref()
                .map(note_parent_dir)
                .unwrap_or_default();
            let (resolved, resolved_assets) = core.resolve_batch(
                vault_id,
                &payload.targets,
                &payload.asset_targets,
                &note_dir,
            )?;
            let asset_results = payload
                .asset_targets
                .into_iter()
                .zip(resolved_assets)
                .map(|(target, path)| ResolveAssetResult { target, path })
                .collect::<Vec<_>>();
            let results = payload
                .targets
                .into_iter()
                .zip(resolved)
                .map(|(target, resolved)| match resolved {
                    Some(resolved) => ResolveTargetResult {
                        target,
                        slug: Some(resolved.slug),
                        archived: resolved.relative_path.starts_with(&*archive_prefix),
                    },
                    None => ResolveTargetResult {
                        target,
                        slug: None,
                        archived: false,
                    },
                })
                .collect::<Vec<_>>();
            Ok((results, asset_results))
        })
        .await;

    match result {
        Ok((results, asset_results)) => (
            StatusCode::OK,
            Json(VaultResolveBatchResponse {
                vault_id,
                results,
                asset_results,
            }),
        )
            .into_response(),
        Err(error) => read_error_response(error),
    }
}

/// `GET /api/v1/vaults/{vault_id}/assets/{*path}` — one Vault's contained
/// assets and imported attachments, which share one containment rule and are
/// not otherwise distinguished on disk. Every check behind it — the Vault gate,
/// path containment, the servable-extension allow-list, the content type, the
/// browse surface, and the response bound — belongs to
/// `VaultReadCore::contained_asset`, which the MCP `get_attachment` tool
/// answers on too; what is left here is the response's own wire shape.
pub async fn vault_scoped_asset_handler(
    State(state): State<AppState>,
    mcp_read: Option<axum::Extension<crate::auth::McpAssetRead>>,
    Path((raw_vault_id, path)): Path<(String, String)>,
) -> Response {
    // Present only when the auth layer admitted this request on the MCP bearer
    // token (#176). That credential's ceiling for attachment bytes is
    // `HATCHDOOR_MCP_MAX_BASE64_BYTES`, which `get_attachment`'s base64 encoding
    // enforces; without this the `download_url` the same tool advertises would
    // be a way around it, up to this route's own far larger bound. A web-token
    // request carries no marker and keeps the route's bound.
    let mcp_max_bytes = mcp_read.map(|axum::Extension(read)| read.max_bytes);
    let vault_id = match parse_vault_id(&raw_vault_id) {
        Ok(vault_id) => vault_id,
        Err(error) => return bad_request(error),
    };
    let lookup_path = path.clone();
    // Directory lookup, canonicalize-and-contain path resolution, and the
    // file read are all blocking filesystem work; do all three in one
    // offloaded trip rather than only the final read.
    let result = VaultReads::new(&state)
        .read(move |core| {
            let asset = match core.contained_asset(vault_id, &lookup_path)? {
                Ok(asset) => asset,
                Err(kind) => return Ok(AssetOutcome::PathError(kind)),
            };
            // The MCP credential's own ceiling, checked before the read rather
            // than after, so an over-limit file is never buffered for a caller
            // that may not have it.
            if mcp_max_bytes.is_some_and(|max_bytes| asset.size_bytes > max_bytes) {
                return Ok(AssetOutcome::PathError(AssetPathError::TooLarge));
            }
            let content_type = asset.content_type;
            // Bounded read: an asset is buffered whole to build the response, so
            // a single request must not turn into an unbounded allocation.
            Ok(match asset.read_bytes() {
                Ok(bytes) => AssetOutcome::Bytes {
                    content_type,
                    bytes,
                },
                Err(AssetReadError::TooLarge) => AssetOutcome::PathError(AssetPathError::TooLarge),
                Err(AssetReadError::Io(error)) => AssetOutcome::Internal(error),
            })
        })
        .await;

    match result {
        Ok(AssetOutcome::Bytes {
            content_type,
            bytes,
        }) => asset_response(content_type, bytes),
        Ok(AssetOutcome::PathError(kind)) => vault_asset_error_response(kind, &path, vault_id),
        Ok(AssetOutcome::Internal(message)) => internal_error(message).into_response(),
        Err(error) => read_error_response(error),
    }
}

enum AssetOutcome {
    PathError(AssetPathError),
    Internal(String),
    Bytes {
        content_type: &'static str,
        bytes: Vec<u8>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_read_error_response_uses_issue_62_http_meaning_buckets() {
        let not_found = vault_read_error_response(VaultReadError {
            code: "vault_not_found".to_string(),
            message: "missing".to_string(),
            vault_id: None,
            retryable: false,
        });
        assert_eq!(not_found.status(), StatusCode::NOT_FOUND);

        let disabled = vault_read_error_response(VaultReadError {
            code: "vault_disabled".to_string(),
            message: "disabled".to_string(),
            vault_id: None,
            retryable: false,
        });
        assert_eq!(disabled.status(), StatusCode::CONFLICT);

        let unavailable = vault_read_error_response(VaultReadError {
            code: "vault_unavailable".to_string(),
            message: "unavailable".to_string(),
            vault_id: None,
            retryable: true,
        });
        assert_eq!(unavailable.status(), StatusCode::SERVICE_UNAVAILABLE);

        let invalid_query = vault_read_error_response(VaultReadError {
            code: "invalid_search_query".to_string(),
            message: "empty".to_string(),
            vault_id: None,
            retryable: false,
        });
        assert_eq!(invalid_query.status(), StatusCode::BAD_REQUEST);

        let invalid_layer = vault_read_error_response(VaultReadError {
            code: "invalid_layer_selection".to_string(),
            message: "absent everywhere".to_string(),
            vault_id: None,
            retryable: false,
        });
        assert_eq!(invalid_layer.status(), StatusCode::BAD_REQUEST);

        let search_unavailable = vault_read_error_response(VaultReadError {
            code: "search_unavailable".to_string(),
            message: "embedder failed".to_string(),
            vault_id: None,
            retryable: true,
        });
        assert_eq!(search_unavailable.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn vault_asset_error_response_reuses_the_shared_vault_api_error_shape() {
        let vault_id = VaultId::generate().expect("generate Vault id");
        let response =
            vault_asset_error_response(AssetPathError::Forbidden, "secret.txt", vault_id);
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}
