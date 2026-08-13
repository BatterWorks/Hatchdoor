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

use crate::api_types::{ResolveBatchRequest, ResolveQuery, ResolveTargetResult};
use crate::app_state::{AppState, run_blocking};
use crate::handlers::api::MAX_RESOLVE_BATCH;
use crate::handlers::assets::{
    AssetPathError, asset_error_parts, asset_response, content_type_for_path, resolve_asset_path,
};
use crate::handlers::downloads::{NoteExport, build_note_export, download_response};
use crate::handlers::vaults::{
    VaultApiError, internal_error_response, json_rejection_response, parse_vault_id,
    query_rejection_response,
};
use crate::vault_read::{VaultReadCore, VaultReadError};
use crate::vault_registry::VaultId;

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct VaultResolveResponse {
    pub vault_id: VaultId,
    pub slug: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct VaultResolveBatchResponse {
    pub vault_id: VaultId,
    pub results: Vec<ResolveTargetResult>,
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn bad_request(error: VaultApiError) -> Response {
    error.respond(StatusCode::BAD_REQUEST)
}

fn note_not_found_response(vault_id: VaultId, slug: &str) -> Response {
    VaultApiError::new(
        "note_not_found",
        format!("Note not found: {slug}"),
        Some(vault_id),
        false,
    )
    .respond(StatusCode::NOT_FOUND)
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
    let (code, status): (&'static str, StatusCode) = match error.code.as_str() {
        "vault_not_found" => ("vault_not_found", StatusCode::NOT_FOUND),
        "vault_disabled" => ("vault_disabled", StatusCode::CONFLICT),
        "vault_scan_config_invalid" => (
            "vault_scan_config_invalid",
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
        "vault_read_unavailable" => ("vault_read_unavailable", StatusCode::SERVICE_UNAVAILABLE),
        "vault_runtime_not_active" => ("vault_unavailable", StatusCode::SERVICE_UNAVAILABLE),
        "invalid_search_query" => ("invalid_search_query", StatusCode::BAD_REQUEST),
        "invalid_layer_selection" => ("invalid_layer_selection", StatusCode::BAD_REQUEST),
        "search_unavailable" => ("search_unavailable", StatusCode::SERVICE_UNAVAILABLE),
        _ => ("vault_unavailable", StatusCode::SERVICE_UNAVAILABLE),
    };
    VaultApiError::new(code, error.message, error.vault_id, error.retryable).respond(status)
}

/// Maps `resolve_asset_path`'s containment outcome onto the shared
/// `VaultApiError` shape, mirroring `assets.rs::asset_error_response`'s
/// `AssetPathError` -> (status, message) choice for the legacy unscoped route.
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
    let cache = state.startup_sqlite.clone();
    let vaults = state.vaults.clone();
    let lookup_slug = slug.clone();
    let result = run_blocking(move || {
        let core = VaultReadCore::new(&cache, &vaults);
        Ok(core.exact_note(vault_id, &lookup_slug))
    })
    .await;
    match result {
        Ok(Ok(Some(note))) => (StatusCode::OK, Json(note)).into_response(),
        Ok(Ok(None)) => note_not_found_response(vault_id, &slug),
        Ok(Err(error)) => vault_read_error_response(error),
        Err(error) => error.into_response(),
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
    let cache = state.startup_sqlite.clone();
    let vaults = state.vaults.clone();
    let lookup_slug = slug.clone();
    let result = run_blocking(move || {
        let core = VaultReadCore::new(&cache, &vaults);
        Ok(core.exact_note_links(vault_id, &lookup_slug))
    })
    .await;
    match result {
        Ok(Ok(Some(links))) => (StatusCode::OK, Json(links)).into_response(),
        Ok(Ok(None)) => note_not_found_response(vault_id, &slug),
        Ok(Err(error)) => vault_read_error_response(error),
        Err(error) => error.into_response(),
    }
}

enum DownloadOutcome {
    NotFound,
    ReadError(VaultReadError),
    ExportError(String),
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
    let cache = state.startup_sqlite.clone();
    let vaults = state.vaults.clone();
    let lookup_slug = slug.clone();
    let result = run_blocking(move || {
        let core = VaultReadCore::new(&cache, &vaults);
        // Note content and its containing directory must come from the same
        // Vault control-block fetch: a concurrent edit reconciles a
        // *replacement* control block rather than mutating the current one in
        // place, so two independent lookups could otherwise pair this note's
        // content with a different Vault generation's directory.
        let (note, vault_root) = match core.exact_note_for_download(vault_id, &lookup_slug) {
            Ok(Some(found)) => found,
            Ok(None) => return Ok(DownloadOutcome::NotFound),
            Err(error) => return Ok(DownloadOutcome::ReadError(error)),
        };
        Ok(match build_note_export(&vault_root, &note.note) {
            Ok(export) => DownloadOutcome::Export(export),
            Err(message) => DownloadOutcome::ExportError(message),
        })
    })
    .await;

    let outcome = match result {
        Ok(outcome) => outcome,
        Err(error) => return error.into_response(),
    };

    match outcome {
        DownloadOutcome::Export(export) => download_response(export),
        DownloadOutcome::NotFound => note_not_found_response(vault_id, &slug),
        DownloadOutcome::ReadError(error) => vault_read_error_response(error),
        DownloadOutcome::ExportError(message) => internal_error_response(message, Some(vault_id)),
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
    let cache = state.startup_sqlite.clone();
    let vaults = state.vaults.clone();
    let result = run_blocking(move || {
        let core = VaultReadCore::new(&cache, &vaults);
        Ok(core.resolve_wikilink(vault_id, &query.target))
    })
    .await;
    match result {
        Ok(Ok(resolved)) => (
            StatusCode::OK,
            Json(VaultResolveResponse {
                vault_id,
                slug: resolved.map(|resolved| resolved.slug),
            }),
        )
            .into_response(),
        Ok(Err(error)) => vault_read_error_response(error),
        Err(error) => error.into_response(),
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
    if payload.targets.len() > MAX_RESOLVE_BATCH {
        return VaultApiError::new(
            "resolve_batch_too_large",
            format!("Too many targets (max {MAX_RESOLVE_BATCH})"),
            Some(vault_id),
            false,
        )
        .respond(StatusCode::PAYLOAD_TOO_LARGE);
    }

    let cache = state.startup_sqlite.clone();
    let vaults = state.vaults.clone();
    let snapshot = state.runtime_snapshot();
    let control = vaults.runtime(vault_id);
    let archive_prefix = match AppState::vault_archive_prefix(
        control.as_ref().map(|control| control.definition()),
        &snapshot,
    ) {
        Ok(prefix) => prefix,
        Err(error) => return internal_error_response(error, Some(vault_id)),
    };

    let result = run_blocking(move || {
        let core = VaultReadCore::new(&cache, &vaults);
        // One authoritative-index build for the whole batch: `resolve_wikilinks`
        // resolves every target against it, rather than paying a full Vault
        // scan per target the way looping `resolve_wikilink` would.
        let resolved = match core.resolve_wikilinks(vault_id, &payload.targets) {
            Ok(resolved) => resolved,
            Err(error) => return Ok(Err(error)),
        };
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
        Ok(Ok(results))
    })
    .await;

    match result {
        Ok(Ok(results)) => (
            StatusCode::OK,
            Json(VaultResolveBatchResponse { vault_id, results }),
        )
            .into_response(),
        Ok(Err(error)) => vault_read_error_response(error),
        Err(error) => error.into_response(),
    }
}

/// `GET /api/v1/vaults/{vault_id}/assets/{*path}` — the same containment
/// (extension allowlist, traversal rejection, canonicalize + `starts_with`
/// containment) the legacy `/vault-assets/{*path}` route applied (retired in
/// #101), scoped to the requested Vault ID's own directory. Serves both
/// embedded assets and imported attachments, which share one containment rule
/// and are not otherwise distinguished on disk.
pub async fn vault_scoped_asset_handler(
    State(state): State<AppState>,
    Path((raw_vault_id, path)): Path<(String, String)>,
) -> Response {
    let vault_id = match parse_vault_id(&raw_vault_id) {
        Ok(vault_id) => vault_id,
        Err(error) => return bad_request(error),
    };
    let cache = state.startup_sqlite.clone();
    let vaults = state.vaults.clone();
    let lookup_path = path.clone();
    // Directory lookup, canonicalize-and-contain path resolution, and the
    // file read are all blocking filesystem work; do all three in one
    // `run_blocking` trip rather than only the final read.
    let result = run_blocking(move || {
        let core = VaultReadCore::new(&cache, &vaults);
        let vault_root = match core.vault_directory(vault_id) {
            Ok(root) => root,
            Err(error) => return Ok(AssetOutcome::VaultError(error)),
        };
        let asset_path = match resolve_asset_path(&vault_root, &lookup_path) {
            Ok(path) => path,
            Err(kind) => return Ok(AssetOutcome::PathError(kind)),
        };
        let content_type = content_type_for_path(&asset_path);
        std::fs::read(&asset_path)
            .map(|bytes| AssetOutcome::Bytes {
                content_type,
                bytes,
            })
            .map_err(|error| format!("failed reading asset '{}': {error}", asset_path.display()))
    })
    .await;

    match result {
        Ok(AssetOutcome::Bytes {
            content_type,
            bytes,
        }) => asset_response(content_type, bytes),
        Ok(AssetOutcome::VaultError(error)) => vault_read_error_response(error),
        Ok(AssetOutcome::PathError(kind)) => vault_asset_error_response(kind, &path, vault_id),
        Err(error) => error.into_response(),
    }
}

enum AssetOutcome {
    VaultError(VaultReadError),
    PathError(AssetPathError),
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
