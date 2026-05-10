use std::path::{Component, Path as FsPath, PathBuf};

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;

use crate::api_types::ErrorResponse;
use crate::app_state::AppState;

pub(crate) async fn vault_asset_handler(
    Path(path): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let asset_path = match resolve_asset_path(&state.vault_path, &path) {
        Ok(path) => path,
        Err(kind) => {
            return asset_error_response(kind, &path);
        }
    };

    let bytes = match std::fs::read(&asset_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed reading asset '{}': {error}", asset_path.display()),
                }),
            )
                .into_response();
        }
    };

    let content_type = content_type_for_path(&asset_path);
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, content_type)],
        bytes,
    )
        .into_response()
}

fn resolve_asset_path(vault_root: &FsPath, raw_path: &str) -> Result<PathBuf, AssetPathError> {
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
                "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "avif" | "bmp"
            )
        })
        .unwrap_or(false)
}

fn content_type_for_path(path: &FsPath) -> &'static str {
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
        _ => "application/octet-stream",
    }
}

fn asset_error_response(kind: AssetPathError, requested_path: &str) -> axum::response::Response {
    let (status, message) = match kind {
        AssetPathError::BadRequest => (
            StatusCode::BAD_REQUEST,
            format!("Invalid asset path: {requested_path}"),
        ),
        AssetPathError::Forbidden => (
            StatusCode::FORBIDDEN,
            format!("Asset access denied: {requested_path}"),
        ),
        AssetPathError::NotFound => (
            StatusCode::NOT_FOUND,
            format!("Asset not found: {requested_path}"),
        ),
        AssetPathError::Internal => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Asset resolution failed".to_string(),
        ),
    };

    (status, Json(ErrorResponse { error: message })).into_response()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AssetPathError {
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
    fn is_allowed_asset_extension_filters_by_image_types() {
        assert!(is_allowed_asset_extension(FsPath::new("diagram.png")));
        assert!(is_allowed_asset_extension(FsPath::new("photo.JPEG")));
        assert!(!is_allowed_asset_extension(FsPath::new("notes.md")));
        assert!(!is_allowed_asset_extension(FsPath::new("noext")));
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
