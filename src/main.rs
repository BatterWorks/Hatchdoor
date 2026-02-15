mod vault;

use std::env;
use std::net::SocketAddr;
use std::path::{Component, Path as FsPath, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::{Json, Router};
use dotenvy::dotenv;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tracing::{debug, error, info, warn};
use tracing_subscriber::EnvFilter;

use crate::vault::{ExplorerFolder, Note, NoteLinks, SearchHit, VaultIndex};

#[derive(Debug, Clone)]
struct AppConfig {
    vault_path: PathBuf,
    host: String,
    port: u16,
    refresh_seconds: u64,
}

impl AppConfig {
    fn from_env() -> Result<Self, String> {
        let vault_path = env::var("VAULT_PATH").unwrap_or_else(|_| "./vault".to_string());
        let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let port_raw = env::var("PORT").unwrap_or_else(|_| "42824".to_string());
        let refresh_raw = env::var("VAULT_REFRESH_SECONDS").unwrap_or_else(|_| "2".to_string());

        let port = parse_port(&port_raw)?;
        let refresh_seconds = parse_refresh_seconds(&refresh_raw)?;

        Ok(Self {
            vault_path: PathBuf::from(vault_path),
            host,
            port,
            refresh_seconds,
        })
    }

    fn socket_addr(&self) -> Result<SocketAddr, String> {
        format!("{}:{}", self.host, self.port)
            .parse::<SocketAddr>()
            .map_err(|e| format!("invalid bind address: {e}"))
    }
}

fn parse_port(input: &str) -> Result<u16, String> {
    input
        .parse::<u16>()
        .map_err(|e| format!("invalid PORT '{input}': {e}"))
}

fn parse_refresh_seconds(input: &str) -> Result<u64, String> {
    input
        .parse::<u64>()
        .map_err(|e| format!("invalid VAULT_REFRESH_SECONDS '{input}': {e}"))
}

#[derive(Clone)]
struct AppState {
    vault_path: PathBuf,
    refresh_interval: Duration,
    cache: Arc<RwLock<VaultCache>>,
}

struct VaultCache {
    index: Arc<VaultIndex>,
    explorer_tree: Arc<ExplorerFolder>,
    last_refresh: Instant,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

#[derive(Debug, Serialize)]
struct NoteResponse {
    note: Note,
}

#[derive(Debug, Serialize)]
struct NoteLinksResponse {
    links: NoteLinks,
}

#[derive(Debug, Deserialize)]
struct ResolveQuery {
    target: String,
}

#[derive(Debug, Serialize)]
struct ResolveResponse {
    slug: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ResolveBatchRequest {
    targets: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ResolveBatchResponse {
    results: Vec<ResolveTargetResult>,
}

#[derive(Debug, Serialize)]
struct ResolveTargetResult {
    target: String,
    slug: Option<String>,
}

#[derive(Debug, Serialize)]
struct RefreshResponse {
    refreshed: bool,
}

#[derive(Debug, Deserialize)]
struct SearchQuery {
    q: String,
    content: Option<bool>,
    limit: Option<usize>,
}

#[derive(Debug, Serialize)]
struct SearchResponse {
    results: Vec<SearchHit>,
}

#[tokio::main]
async fn main() {
    dotenv().ok();
    init_logging();

    let config = AppConfig::from_env().unwrap_or_else(|e| {
        error!("Configuration error: {e}");
        std::process::exit(1);
    });

    let cache = build_cache(&config.vault_path).unwrap_or_else(|e| {
        error!(
            "Failed to index vault at {}: {e}",
            config.vault_path.display()
        );
        std::process::exit(1);
    });

    let state = AppState {
        vault_path: config.vault_path.clone(),
        refresh_interval: Duration::from_secs(config.refresh_seconds),
        cache: Arc::new(RwLock::new(cache)),
    };

    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/api/tree", get(tree_handler))
        .route("/api/note/{slug}", get(note_handler))
        .route("/api/note/{slug}/links", get(note_links_handler))
        .route("/api/resolve", get(resolve_handler))
        .route("/api/resolve-batch", post(resolve_batch_handler))
        .route("/api/search", get(search_handler))
        .route("/api/refresh", post(refresh_handler))
        .route("/", get(spa_index_handler))
        .route("/n/{slug}", get(spa_index_handler))
        .route("/vault-assets/{*path}", get(vault_asset_handler))
        .route_service(
            "/manifest.webmanifest",
            ServeFile::new("frontend/dist/manifest.webmanifest"),
        )
        .route_service(
            "/registerSW.js",
            ServeFile::new("frontend/dist/registerSW.js"),
        )
        .route_service("/sw.js", ServeFile::new("frontend/dist/sw.js"))
        .nest_service("/assets", ServeDir::new("frontend/dist/assets"))
        .fallback_service(ServeDir::new("frontend/dist"))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().include_headers(false))
                .on_response(DefaultOnResponse::new().include_headers(false)),
        )
        .with_state(state);

    let addr = config.socket_addr().unwrap_or_else(|e| {
        error!("Address error: {e}");
        std::process::exit(1);
    });

    info!(
        host = %config.host,
        port = config.port,
        refresh_seconds = config.refresh_seconds,
        vault_path = %config.vault_path.display(),
        "Hatchdoor starting"
    );
    info!("Hatchdoor listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|e| {
            error!("Failed to bind: {e}");
            std::process::exit(1);
        });

    axum::serve(listener, app).await.unwrap_or_else(|e| {
        error!("Server error: {e}");
        std::process::exit(1);
    });
}

fn init_logging() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("hatchdoor=info,tower_http=info,axum::rejection=warn"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();
}

fn build_cache(vault_path: &PathBuf) -> Result<VaultCache, String> {
    debug!(vault_path = %vault_path.display(), "Building vault cache");
    let index = VaultIndex::build(vault_path).map_err(|e| e.to_string())?;
    let explorer_tree = index.explorer_tree();

    Ok(VaultCache {
        index: Arc::new(index),
        explorer_tree: Arc::new(explorer_tree),
        last_refresh: Instant::now(),
    })
}

async fn snapshot(
    state: &AppState,
) -> Result<(Arc<VaultIndex>, Arc<ExplorerFolder>), (StatusCode, Json<ErrorResponse>)> {
    refresh_if_needed(state, false).await?;
    let guard = state.cache.read().await;
    Ok((guard.index.clone(), guard.explorer_tree.clone()))
}

async fn refresh_if_needed(
    state: &AppState,
    force: bool,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    {
        let guard = state.cache.read().await;
        if !force && guard.last_refresh.elapsed() < state.refresh_interval {
            return Ok(());
        }
    }

    let mut guard = state.cache.write().await;
    if !force && guard.last_refresh.elapsed() < state.refresh_interval {
        return Ok(());
    }

    match build_cache(&state.vault_path) {
        Ok(cache) => {
            if force {
                info!(
                    force_refresh = true,
                    vault_path = %state.vault_path.display(),
                    "Vault cache refreshed"
                );
            } else {
                debug!(
                    force_refresh = false,
                    vault_path = %state.vault_path.display(),
                    "Vault cache refreshed"
                );
            }
            *guard = cache;
            Ok(())
        }
        Err(error) => {
            error!(
                force_refresh = force,
                vault_path = %state.vault_path.display(),
                error = %error,
                "Vault refresh failed"
            );
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Vault refresh failed: {error}"),
                }),
            ))
        }
    }
}

async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

async fn tree_handler(State(state): State<AppState>) -> impl IntoResponse {
    match snapshot(&state).await {
        Ok((_index, tree)) => (StatusCode::OK, Json((*tree).clone())).into_response(),
        Err(err) => err.into_response(),
    }
}

async fn note_handler(
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

async fn note_links_handler(
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

async fn resolve_handler(
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

async fn resolve_batch_handler(
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

async fn refresh_handler(State(state): State<AppState>) -> impl IntoResponse {
    match refresh_if_needed(&state, true).await {
        Ok(()) => (StatusCode::OK, Json(RefreshResponse { refreshed: true })).into_response(),
        Err(err) => err.into_response(),
    }
}

async fn search_handler(
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
        include_content, limit, "Executing search"
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

async fn spa_index_handler() -> impl IntoResponse {
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

async fn vault_asset_handler(
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn parse_port_accepts_valid_u16() {
        assert_eq!(parse_port("42824").expect("valid port"), 42824);
    }

    #[test]
    fn parse_port_rejects_invalid_values() {
        assert!(parse_port("70000").is_err());
        assert!(parse_port("abc").is_err());
    }

    #[test]
    fn parse_refresh_seconds_accepts_valid_u64() {
        assert_eq!(parse_refresh_seconds("2").expect("valid refresh"), 2);
    }

    #[test]
    fn parse_refresh_seconds_rejects_invalid_values() {
        assert!(parse_refresh_seconds("-1").is_err());
        assert!(parse_refresh_seconds("abc").is_err());
    }

    #[test]
    fn socket_addr_builds_expected_address() {
        let cfg = AppConfig {
            vault_path: PathBuf::from("./vault"),
            host: "0.0.0.0".to_string(),
            port: 42824,
            refresh_seconds: 2,
        };

        let addr = cfg.socket_addr().expect("valid addr");
        assert_eq!(addr.to_string(), "0.0.0.0:42824");
    }

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
}
