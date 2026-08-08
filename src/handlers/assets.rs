use std::path::{Component, Path as FsPath, PathBuf};

use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

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

pub(crate) fn resolve_asset_path(
    vault_root: &FsPath,
    raw_path: &str,
) -> Result<PathBuf, AssetPathError> {
    let relative = sanitize_asset_path(raw_path).ok_or(AssetPathError::BadRequest)?;
    if !is_allowed_asset_extension(&relative) {
        return Err(AssetPathError::Forbidden);
    }

    let root = std::fs::canonicalize(vault_root).map_err(|_| AssetPathError::Internal)?;
    let candidate = vault_root.join(relative);
    let resolved = match std::fs::canonicalize(candidate) {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(AssetPathError::NotFound);
        }
        Err(_) => return Err(AssetPathError::Internal),
    };

    if !resolved.starts_with(&root) {
        return Err(AssetPathError::Forbidden);
    }
    if !resolved.is_file() {
        return Err(AssetPathError::NotFound);
    }

    Ok(resolved)
}

fn sanitize_asset_path(raw_path: &str) -> Option<PathBuf> {
    let mut sanitized = PathBuf::new();
    let trimmed = raw_path.trim();
    if trimmed.is_empty() || FsPath::new(trimmed).is_absolute() {
        return None;
    }

    for component in FsPath::new(trimmed).components() {
        match component {
            Component::Normal(segment) => sanitized.push(segment),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir => return None,
            _ => return None,
        }
    }

    if sanitized.as_os_str().is_empty() {
        return None;
    }

    Some(sanitized)
}

fn is_allowed_asset_extension(path: &FsPath) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "avif" | "bmp" | "pdf"
            )
        })
        .unwrap_or(false)
}

pub(crate) fn content_type_for_path(path: &FsPath) -> &'static str {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("avif") => "image/avif",
        Some("bmp") => "image/bmp",
        Some("pdf") => "application/pdf",
        _ => "application/octet-stream",
    }
}

/// One `AssetPathError` -> (structured code, HTTP status, human message)
/// mapping, shared by this route's `ErrorResponse{error}` shape and
/// `handlers/vault_content.rs`'s Vault-scoped `VaultApiError{code, ...}`
/// shape, so the two wire shapes cannot silently diverge on the same
/// underlying containment outcome.
pub(crate) fn asset_error_parts(
    kind: AssetPathError,
    requested_path: &str,
) -> (&'static str, StatusCode, String) {
    match kind {
        AssetPathError::BadRequest => (
            "invalid_asset_path",
            StatusCode::BAD_REQUEST,
            format!("Invalid asset path: {requested_path}"),
        ),
        AssetPathError::Forbidden => (
            "asset_access_denied",
            StatusCode::FORBIDDEN,
            format!("Asset access denied: {requested_path}"),
        ),
        AssetPathError::NotFound => (
            "asset_not_found",
            StatusCode::NOT_FOUND,
            format!("Asset not found: {requested_path}"),
        ),
        AssetPathError::Internal => (
            "internal_error",
            StatusCode::INTERNAL_SERVER_ERROR,
            "Asset resolution failed".to_string(),
        ),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AssetPathError {
    BadRequest,
    Forbidden,
    NotFound,
    Internal,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn sanitize_asset_path_rejects_invalid_paths() {
        assert!(sanitize_asset_path("").is_none());
        assert!(sanitize_asset_path("../secrets.png").is_none());
        assert!(sanitize_asset_path("/abs/path.png").is_none());
        assert!(sanitize_asset_path("folder/../../escape.png").is_none());
    }

    #[test]
    fn sanitize_asset_path_normalizes_valid_path() {
        let path = sanitize_asset_path("./images/diagram.png").expect("valid path");
        assert_eq!(path, PathBuf::from("images/diagram.png"));
    }

    #[test]
    fn is_allowed_asset_extension_filters_by_embeddable_asset_types() {
        assert!(is_allowed_asset_extension(FsPath::new("diagram.png")));
        assert!(is_allowed_asset_extension(FsPath::new("photo.JPEG")));
        assert!(is_allowed_asset_extension(FsPath::new("manual.PDF")));
        assert!(!is_allowed_asset_extension(FsPath::new("notes.md")));
        assert!(!is_allowed_asset_extension(FsPath::new("noext")));
    }

    #[test]
    fn content_type_for_path_serves_pdf_attachments_inline() {
        assert_eq!(
            content_type_for_path(FsPath::new("Attachments/manual.pdf")),
            "application/pdf"
        );
    }

    #[test]
    fn resolve_asset_path_returns_file_within_vault() {
        let tmp = TempDir::new().expect("temp dir");
        let vault_root = tmp.path().join("vault");
        let notes_dir = vault_root.join("Notes");
        fs::create_dir_all(&notes_dir).expect("create dir");
        let image_path = notes_dir.join("diagram.png");
        fs::write(&image_path, b"png").expect("write image");

        let resolved =
            resolve_asset_path(&vault_root, "Notes/diagram.png").expect("path should resolve");

        assert_eq!(
            resolved,
            std::fs::canonicalize(image_path).expect("canonical image path")
        );
    }

    #[test]
    fn resolve_asset_path_serves_assets_under_noise_paths() {
        // Noise patterns must never gate /vault-assets/ serving: an image
        // embedded in a demoted or otherwise noise-matched folder still renders.
        // A user HATCHDOOR_EXCLUDE glob silently breaking an embedded image would
        // be a nasty surprise, so the asset route deliberately ignores exclusion.
        let tmp = TempDir::new().expect("temp dir");
        let vault_root = tmp.path().join("vault");
        let noise_dir = vault_root.join(".trash");
        fs::create_dir_all(&noise_dir).expect("create noise dir");
        let image_path = noise_dir.join("diagram.png");
        fs::write(&image_path, b"png").expect("write image");

        let resolved = resolve_asset_path(&vault_root, ".trash/diagram.png")
            .expect("a noise-path asset must still resolve and serve");
        assert_eq!(
            resolved,
            std::fs::canonicalize(image_path).expect("canonical image path")
        );
    }

    #[test]
    fn resolve_asset_path_blocks_traversal_and_non_images() {
        let tmp = TempDir::new().expect("temp dir");
        let vault_root = tmp.path().join("vault");
        fs::create_dir_all(&vault_root).expect("create dir");
        fs::write(vault_root.join("secret.txt"), b"secret").expect("write text");

        assert_eq!(
            resolve_asset_path(&vault_root, "../outside.png"),
            Err(AssetPathError::BadRequest)
        );
        assert_eq!(
            resolve_asset_path(&vault_root, "secret.txt"),
            Err(AssetPathError::Forbidden)
        );
        assert_eq!(
            resolve_asset_path(&vault_root, "missing.png"),
            Err(AssetPathError::NotFound)
        );
    }
}
