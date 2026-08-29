//! HTTP wire shaping for the Vault asset route in `handlers/vault_content.rs`:
//! the success response's headers, and the status each contained-resource
//! failure carries.
//!
//! The policy itself — path containment, the servable-extension allow-list, the
//! content-type table, the size bound, and the route's own URL shape — is not
//! here. It belongs to the read core (`vault_read::assets`, reached through
//! `VaultReadCore::contained_asset`), so the MCP `get_attachment` tool answers
//! on exactly the same rules rather than importing this adapter (#188, ADR-19).

use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::vault_read::AssetPathError;

/// Shared response shape for a resolved, in-bounds asset/attachment file,
/// used by the Vault-scoped route in `handlers/vault_content.rs`.
pub(crate) fn asset_response(content_type: &'static str, bytes: Vec<u8>) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    // Cacheable in the browser (assets re-render on every note view) but never
    // in shared caches: authenticated deployments carry ?access_token= in the
    // asset URL, which must not be stored by a proxy.
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=3600"),
    );
    if content_type == "image/svg+xml" {
        // SVGs can carry scripts that execute on direct navigation. Sandbox the
        // document and force a download on navigation; <img> embedding (which
        // never executes scripts) is unaffected by either header.
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("sandbox"),
        );
        headers.insert(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_static("attachment"),
        );
    }

    (StatusCode::OK, headers, bytes).into_response()
}

/// The HTTP status one contained-resource failure carries. The `code` and the
/// message come from the core's own [`AssetPathError`], so this route's
/// `VaultApiError{code, ...}` body and the MCP tool error cannot silently
/// diverge on the same underlying containment outcome.
pub(crate) fn asset_error_parts(
    kind: AssetPathError,
    requested_path: &str,
) -> (&'static str, StatusCode, String) {
    let status = match kind {
        AssetPathError::BadRequest => StatusCode::BAD_REQUEST,
        AssetPathError::Forbidden => StatusCode::FORBIDDEN,
        AssetPathError::NotFound => StatusCode::NOT_FOUND,
        AssetPathError::TooLarge => StatusCode::PAYLOAD_TOO_LARGE,
        AssetPathError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (kind.code(), status, kind.message(requested_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_error_parts_pairs_each_core_code_with_its_http_meaning() {
        assert_eq!(
            asset_error_parts(AssetPathError::NotFound, "a.png"),
            (
                "asset_not_found",
                StatusCode::NOT_FOUND,
                "Asset not found: a.png".to_string()
            )
        );
        assert_eq!(
            asset_error_parts(AssetPathError::Forbidden, "secret.txt").1,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            asset_error_parts(AssetPathError::TooLarge, "big.pdf").1,
            StatusCode::PAYLOAD_TOO_LARGE
        );
    }

    #[test]
    fn svg_responses_are_sandboxed_and_forced_to_download_on_navigation() {
        let response = asset_response("image/svg+xml", b"<svg/>".to_vec());
        assert_eq!(
            response.headers().get(header::CONTENT_SECURITY_POLICY),
            Some(&HeaderValue::from_static("sandbox"))
        );
        assert_eq!(
            response.headers().get(header::CONTENT_DISPOSITION),
            Some(&HeaderValue::from_static("attachment"))
        );
    }
}
