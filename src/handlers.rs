use std::path::{Component, Path as FsPath, PathBuf};

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use tracing::{debug, warn};

use crate::api_types::{
    ErrorResponse, NoteLinksResponse, NoteResponse, RefreshResponse, ResolveBatchRequest,
    ResolveBatchResponse, ResolveQuery, ResolveResponse, ResolveTargetResult, SearchQuery,
    SearchResponse,
};
use crate::app_state::{refresh_if_needed, snapshot, AppState};
use crate::vault::Note;

pub(crate) async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

pub(crate) async fn tree_handler(State(state): State<AppState>) -> impl IntoResponse {
    match snapshot(&state).await {
        Ok((_index, tree)) => (StatusCode::OK, Json((*tree).clone())).into_response(),
        Err(err) => err.into_response(),
    }
}

pub(crate) async fn note_handler(
    Path(slug): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let (index, _tree) = match snapshot(&state).await {
        Ok(s) => s,
        Err(err) => return err.into_response(),
    };

    match index.read_note_by_slug(&slug) {
        Ok(Some(note)) => (StatusCode::OK, Json(NoteResponse { note })).into_response(),
        Ok(None) => {
            warn!(slug = %slug, "Note not found");
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("Note not found: {slug}"),
                }),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed reading note {slug}: {e}"),
            }),
        )
            .into_response(),
    }
}

pub(crate) async fn note_download_handler(
    Path(slug): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let (index, _tree) = match snapshot(&state).await {
        Ok(s) => s,
        Err(err) => return err.into_response(),
    };

    let note = match index.read_note_by_slug(&slug) {
        Ok(Some(note)) => note,
        Ok(None) => {
            warn!(slug = %slug, "Note not found for download");
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("Note not found: {slug}"),
                }),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed reading note {slug}: {e}"),
                }),
            )
                .into_response();
        }
    };

    let filename = download_filename_for_note(&note);
    let content_disposition = build_download_content_disposition(&filename);

    let mut response = Response::new(note.content.into());
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&content_disposition)
            .unwrap_or_else(|_| HeaderValue::from_static("attachment; filename=\"note.md\"")),
    );

    response
}

pub(crate) async fn note_links_handler(
    Path(slug): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let (index, _tree) = match snapshot(&state).await {
        Ok(s) => s,
        Err(err) => return err.into_response(),
    };

    match index.note_links(&slug) {
        Some(links) => (StatusCode::OK, Json(NoteLinksResponse { links })).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Note not found: {slug}"),
            }),
        )
            .into_response(),
    }
}

pub(crate) async fn resolve_handler(
    Query(query): Query<ResolveQuery>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let (index, _tree) = match snapshot(&state).await {
        Ok(s) => s,
        Err(err) => return err.into_response(),
    };

    let slug = index
        .resolve_wikilink(&query.target)
        .map(|entry| entry.slug.clone());

    (StatusCode::OK, Json(ResolveResponse { slug })).into_response()
}

pub(crate) async fn resolve_batch_handler(
    State(state): State<AppState>,
    Json(payload): Json<ResolveBatchRequest>,
) -> impl IntoResponse {
    let (index, _tree) = match snapshot(&state).await {
        Ok(s) => s,
        Err(err) => return err.into_response(),
    };

    let results = payload
        .targets
        .into_iter()
        .map(|target| ResolveTargetResult {
            slug: index
                .resolve_wikilink(&target)
                .map(|entry| entry.slug.clone()),
            target,
        })
        .collect();

    (StatusCode::OK, Json(ResolveBatchResponse { results })).into_response()
}

pub(crate) async fn refresh_handler(State(state): State<AppState>) -> impl IntoResponse {
    match refresh_if_needed(&state, true).await {
        Ok(()) => (StatusCode::OK, Json(RefreshResponse { refreshed: true })).into_response(),
        Err(err) => err.into_response(),
    }
}

pub(crate) async fn search_handler(
    Query(query): Query<SearchQuery>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let (index, _tree) = match snapshot(&state).await {
        Ok(s) => s,
        Err(err) => return err.into_response(),
    };

    let limit = query.limit.unwrap_or(25).clamp(1, 100);
    let include_content = query.content.unwrap_or(false);
    let search_query = query.q;
    debug!(
        query_len = search_query.len(),
        include_content,
        limit,
        "Executing search"
    );

    let handle =
        tokio::task::spawn_blocking(move || index.search(&search_query, include_content, limit));

    match handle.await {
        Ok(results) => (StatusCode::OK, Json(SearchResponse { results })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Search task failed: {e}"),
            }),
        )
            .into_response(),
    }
}

pub(crate) async fn spa_index_handler() -> impl IntoResponse {
    match std::fs::read_to_string("frontend/dist/index.html") {
        Ok(html) => (StatusCode::OK, Html(html)).into_response(),
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Html(
                "<h1>Frontend not built</h1><p>Run <code>cd frontend && npm install && npm run build</code>, then restart the server.</p>"
                    .to_string(),
            ),
        )
            .into_response(),
    }
}

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
    if trimmed.is_empty() {
        return None;
    }

    if FsPath::new(trimmed).is_absolute() {
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

fn download_filename_for_note(note: &Note) -> String {
    let from_path = note
        .relative_path
        .split('/')
        .next_back()
        .unwrap_or(note.title.as_str());
    let base = sanitize_download_filename(from_path);
    if base.ends_with(".md") {
        base
    } else {
        format!("{base}.md")
    }
}

fn sanitize_download_filename(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    for ch in input.trim().chars() {
        let allowed = ch.is_ascii_alphanumeric() || matches!(ch, ' ' | '-' | '_' | '.' | '(' | ')');
        if allowed {
            output.push(ch);
        } else if !ch.is_ascii_control() {
            output.push('-');
        }
    }

    let collapsed = output.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim_end_matches('.').trim();
    if trimmed.is_empty() {
        "note".to_string()
    } else {
        trimmed.to_string()
    }
}

fn build_download_content_disposition(filename: &str) -> String {
    let ascii_fallback = filename
        .chars()
        .map(|ch| if ch.is_ascii() { ch } else { '-' })
        .collect::<String>();
    format!(
        "attachment; filename=\"{}\"; filename*=UTF-8''{}",
        ascii_fallback,
        percent_encode_filename(filename)
    )
}

fn percent_encode_filename(input: &str) -> String {
    let mut encoded = String::with_capacity(input.len());
    for byte in input.bytes() {
        let is_safe = matches!(
            byte,
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~'
        );
        if is_safe {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push_str(&format!("{byte:02X}"));
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use tempfile::TempDir;
    use tokio::sync::RwLock;
    use std::sync::Arc;
    use std::time::Duration;

    use crate::app_state::build_cache;

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
    fn content_type_for_path_maps_known_types() {
        assert_eq!(
            content_type_for_path(FsPath::new("diagram.svg")),
            "image/svg+xml"
        );
        assert_eq!(
            content_type_for_path(FsPath::new("photo.jpg")),
            "image/jpeg"
        );
        assert_eq!(
            content_type_for_path(FsPath::new("unknown.xyz")),
            "application/octet-stream"
        );
    }

    #[test]
    fn resolve_asset_path_returns_file_within_vault() {
        let tmp = TempDir::new().expect("temp dir");
        let vault_root = tmp.path().join("vault");
        let notes_dir = vault_root.join("Notes");
        std::fs::create_dir_all(&notes_dir).expect("create dir");
        let image_path = notes_dir.join("diagram.png");
        std::fs::write(&image_path, b"png").expect("write image");

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
        std::fs::create_dir_all(&vault_root).expect("create dir");
        let text_path = vault_root.join("secret.txt");
        std::fs::write(&text_path, b"secret").expect("write text");

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

    #[test]
    fn sanitize_download_filename_replaces_unsafe_chars() {
        assert_eq!(
            sanitize_download_filename("  Prox:mox/Note?.md  "),
            "Prox-mox-Note-.md"
        );
        assert_eq!(sanitize_download_filename(""), "note");
    }

    #[test]
    fn download_filename_for_note_adds_md_extension_when_missing() {
        let note = Note {
            title: "README".to_string(),
            slug: "readme".to_string(),
            relative_path: "Docs/README".to_string(),
            content: "# Readme".to_string(),
        };

        assert_eq!(download_filename_for_note(&note), "README.md");
    }

    #[test]
    fn build_download_content_disposition_includes_utf8_filename() {
        let value = build_download_content_disposition("Homelab Atlas.md");
        assert_eq!(
            value,
            "attachment; filename=\"Homelab Atlas.md\"; filename*=UTF-8''Homelab%20Atlas.md"
        );
    }

    #[tokio::test]
    async fn note_download_handler_returns_markdown_with_headers() {
        let tmp = TempDir::new().expect("temp dir");
        let vault_root = tmp.path().join("vault");
        std::fs::create_dir_all(&vault_root).expect("create vault");
        std::fs::write(vault_root.join("README.md"), "# Home\n").expect("write note");

        let cache = build_cache(&vault_root).expect("build cache");
        let state = AppState {
            vault_path: vault_root,
            refresh_interval: Duration::from_secs(60),
            cache: Arc::new(RwLock::new(cache)),
        };

        let response = note_download_handler(Path("readme".to_string()), State(state))
            .await
            .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .expect("content type")
            .to_str()
            .expect("header string");
        assert_eq!(content_type, "application/octet-stream");
        let content_disposition = response
            .headers()
            .get(header::CONTENT_DISPOSITION)
            .expect("content disposition")
            .to_str()
            .expect("header string");
        assert_eq!(
            content_disposition,
            "attachment; filename=\"README.md\"; filename*=UTF-8''README.md"
        );

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        assert_eq!(body, "# Home\n");
    }
}
