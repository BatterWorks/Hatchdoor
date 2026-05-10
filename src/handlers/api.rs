use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use tracing::{debug, warn};

use crate::api_types::{
    ErrorResponse, NoteLinksResponse, NoteResponse, RefreshResponse, ResolveBatchRequest,
    ResolveBatchResponse, ResolveQuery, ResolveResponse, ResolveTargetResult, SearchQuery,
    SearchResponse,
};
use crate::app_state::{refresh_if_needed, sqlite_cache, AppState};

pub(crate) async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

pub(crate) async fn tree_handler(State(state): State<AppState>) -> impl IntoResponse {
    let cache = match sqlite_cache(&state).await {
        Ok(cache) => cache,
        Err(err) => return err.into_response(),
    };

    match cache.explorer_tree() {
        Ok(tree) => (StatusCode::OK, Json(tree)).into_response(),
        Err(error) => internal_error_response(error),
    }
}

pub(crate) async fn note_handler(
    Path(slug): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let cache = match sqlite_cache(&state).await {
        Ok(cache) => cache,
        Err(err) => return err.into_response(),
    };

    match cache.read_note_by_slug(&slug) {
        Ok(Some(note)) => (StatusCode::OK, Json(NoteResponse { note })).into_response(),
        Ok(None) => {
            warn!(slug = %slug, "Note not found");
            note_not_found_response(&slug)
        }
        Err(error) => internal_error_response(format!("Failed reading note {slug}: {error}")),
    }
}

pub(crate) async fn note_links_handler(
    Path(slug): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let cache = match sqlite_cache(&state).await {
        Ok(cache) => cache,
        Err(err) => return err.into_response(),
    };

    match cache.note_links(&slug) {
        Ok(Some(links)) => (StatusCode::OK, Json(NoteLinksResponse { links })).into_response(),
        Ok(None) => note_not_found_response(&slug),
        Err(error) => internal_error_response(error),
    }
}

pub(crate) async fn resolve_handler(
    Query(query): Query<ResolveQuery>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let cache = match sqlite_cache(&state).await {
        Ok(cache) => cache,
        Err(err) => return err.into_response(),
    };

    match cache.resolve_wikilink(&query.target) {
        Ok(slug) => (StatusCode::OK, Json(ResolveResponse { slug })).into_response(),
        Err(error) => internal_error_response(error),
    }
}

pub(crate) async fn resolve_batch_handler(
    State(state): State<AppState>,
    Json(payload): Json<ResolveBatchRequest>,
) -> impl IntoResponse {
    let cache = match sqlite_cache(&state).await {
        Ok(cache) => cache,
        Err(err) => return err.into_response(),
    };

    let mut results = Vec::with_capacity(payload.targets.len());
    for target in payload.targets {
        let slug = match cache.resolve_wikilink(&target) {
            Ok(slug) => slug,
            Err(error) => return internal_error_response(error),
        };
        results.push(ResolveTargetResult { target, slug });
    }

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
    let cache = match sqlite_cache(&state).await {
        Ok(cache) => cache,
        Err(err) => return err.into_response(),
    };

    let limit = query.limit.unwrap_or(25).clamp(1, 100);
    let include_content = query.content.unwrap_or(false);
    let search_query = query.q;
    debug!(
        query_len = search_query.len(),
        include_content,
        limit,
        "Executing SQLite search"
    );

    match cache.search(&search_query, include_content, limit) {
        Ok(results) => (StatusCode::OK, Json(SearchResponse { results })).into_response(),
        Err(error) => internal_error_response(format!("Search failed: {error}")),
    }
}

fn note_not_found_response(slug: &str) -> axum::response::Response {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            error: format!("Note not found: {slug}"),
        }),
    )
        .into_response()
}

fn internal_error_response(error: String) -> axum::response::Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse { error }),
    )
        .into_response()
}
