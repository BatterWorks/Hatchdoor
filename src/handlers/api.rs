use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use std::convert::Infallible;
use std::sync::atomic::Ordering;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;
use tracing::{debug, warn};

use crate::api_types::{
    ErrorResponse, NoteLinksResponse, NoteResponse, RecentlyModifiedQuery,
    RecentlyModifiedResponse, RefreshResponse, ResolveBatchRequest, ResolveBatchResponse,
    ResolveQuery, ResolveResponse, ResolveTargetResult, SearchQuery, SearchResponse,
    VaultEventResponse,
};
use crate::app_state::{AppState, refresh_if_needed, sqlite_cache};

pub async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

pub async fn tree_handler(State(state): State<AppState>) -> impl IntoResponse {
    let cache = match sqlite_cache(&state).await {
        Ok(cache) => cache,
        Err(err) => return err.into_response(),
    };

    match cache.explorer_tree() {
        Ok(tree) => (StatusCode::OK, Json(tree)).into_response(),
        Err(error) => internal_error_response(error),
    }
}

pub async fn note_handler(
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

pub async fn note_links_handler(
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

pub async fn resolve_handler(
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

pub async fn resolve_batch_handler(
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

pub async fn refresh_handler(State(state): State<AppState>) -> impl IntoResponse {
    match refresh_if_needed(&state).await {
        Ok(()) => (StatusCode::OK, Json(RefreshResponse { refreshed: true })).into_response(),
        Err(err) => err.into_response(),
    }
}

pub async fn vault_events_handler(
    State(state): State<AppState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let current_revision = state.vault_revision.load(Ordering::SeqCst);
    let current_event = tokio_stream::once(Ok(vault_revision_event(current_revision)));
    let live_events =
        BroadcastStream::new(state.vault_events.subscribe()).filter_map(|event| match event {
            Ok(revision) => Some(Ok(vault_revision_event(revision))),
            Err(_) => None,
        });

    let stream = current_event.chain(live_events);
    Sse::new(stream).keep_alive(KeepAlive::default())
}

fn vault_revision_event(revision: u64) -> Event {
    let payload = serde_json::to_string(&VaultEventResponse { revision })
        .unwrap_or_else(|_| format!(r#"{{"revision":{revision}}}"#));
    Event::default()
        .event("vault-revision")
        .id(revision.to_string())
        .data(payload)
}

pub async fn recently_modified_handler(
    Query(query): Query<RecentlyModifiedQuery>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let cache = match sqlite_cache(&state).await {
        Ok(cache) => cache,
        Err(err) => return err.into_response(),
    };

    let limit = query.limit.unwrap_or(5).clamp(1, 25);
    match cache.recently_modified_notes(limit) {
        Ok(notes) => (StatusCode::OK, Json(RecentlyModifiedResponse { notes })).into_response(),
        Err(error) => internal_error_response(error),
    }
}

pub async fn search_handler(
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
        include_content, limit, "Executing SQLite search"
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
