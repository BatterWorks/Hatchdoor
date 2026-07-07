use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;
use tracing::{debug, warn};

use crate::api_types::{
    ErrorResponse, NoteLinksResponse, NoteResponse, RecentlyModifiedQuery,
    RecentlyModifiedResponse, RefreshResponse, ResolveBatchRequest, ResolveBatchResponse,
    ResolveQuery, ResolveResponse, ResolveTargetResult, SearchQuery, VaultEventResponse,
};

use crate::app_state::{AppState, refresh_coalescing, run_blocking, sqlite_cache};

/// Upper bound on targets accepted by `/api/resolve-batch` (DoS guard).
const MAX_RESOLVE_BATCH: usize = 200;

pub async fn health_handler(State(state): State<AppState>) -> impl IntoResponse {
    let cache = state.cache.read().await.sqlite.clone();
    match run_blocking(move || cache.health_check()).await {
        Ok(()) => (StatusCode::OK, "ok").into_response(),
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, "unavailable").into_response(),
    }
}

pub async fn tree_handler(State(state): State<AppState>) -> impl IntoResponse {
    let cache = match sqlite_cache(&state).await {
        Ok(cache) => cache,
        Err(err) => return err.into_response(),
    };

    match run_blocking(move || cache.explorer_tree()).await {
        Ok(tree) => (StatusCode::OK, Json(tree)).into_response(),
        Err(err) => err.into_response(),
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

    let lookup_slug = slug.clone();
    match run_blocking(move || cache.read_note_by_slug(&lookup_slug)).await {
        Ok(Some(note)) => (StatusCode::OK, Json(NoteResponse { note })).into_response(),
        Ok(None) => {
            warn!(slug = %slug, "Note not found");
            note_not_found_response(&slug)
        }
        Err(err) => err.into_response(),
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

    let lookup_slug = slug.clone();
    match run_blocking(move || cache.note_links(&lookup_slug)).await {
        Ok(Some(links)) => (StatusCode::OK, Json(NoteLinksResponse { links })).into_response(),
        Ok(None) => note_not_found_response(&slug),
        Err(err) => err.into_response(),
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

    match run_blocking(move || cache.resolve_wikilink(&query.target)).await {
        Ok(resolved) => {
            let slug = resolved.map(|(slug, _)| slug);
            (StatusCode::OK, Json(ResolveResponse { slug })).into_response()
        }
        Err(err) => err.into_response(),
    }
}

pub async fn resolve_batch_handler(
    State(state): State<AppState>,
    Json(payload): Json<ResolveBatchRequest>,
) -> impl IntoResponse {
    if payload.targets.len() > MAX_RESOLVE_BATCH {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(ErrorResponse {
                error: format!("Too many targets (max {MAX_RESOLVE_BATCH})"),
            }),
        )
            .into_response();
    }

    let cache = match sqlite_cache(&state).await {
        Ok(cache) => cache,
        Err(err) => return err.into_response(),
    };
    let archive_prefix = state.archive_prefix.clone();

    let result = run_blocking(move || {
        let mut results = Vec::with_capacity(payload.targets.len());
        for target in payload.targets {
            let resolved = cache.resolve_wikilink(&target)?;
            let (slug, archived) = match resolved {
                Some((slug, relative_path)) => {
                    let archived = relative_path.starts_with(&*archive_prefix);
                    (Some(slug), archived)
                }
                None => (None, false),
            };
            results.push(ResolveTargetResult {
                target,
                slug,
                archived,
            });
        }
        Ok(results)
    })
    .await;

    match result {
        Ok(results) => (StatusCode::OK, Json(ResolveBatchResponse { results })).into_response(),
        Err(err) => err.into_response(),
    }
}

pub async fn refresh_handler(State(state): State<AppState>) -> impl IntoResponse {
    // A refresh re-embeds the whole vault; on an unauthenticated public demo
    // that would be a request-loop CPU DoS. The watcher keeps the demo fresh.
    if let Some(response) = crate::handlers::write_api::reject_demo_mode_write(&state) {
        return response;
    }
    match refresh_coalescing(&state).await {
        Ok(()) => (StatusCode::OK, Json(RefreshResponse { refreshed: true })).into_response(),
        Err(err) => err.into_response(),
    }
}

pub async fn vault_events_handler(
    State(state): State<AppState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let current_revision = state.vault_revision.load(Ordering::SeqCst);
    let current_event = tokio_stream::once(Ok(vault_revision_event(current_revision)));
    let revision_on_lag: Arc<AtomicU64> = state.vault_revision.clone();
    let live_events = BroadcastStream::new(state.vault_events.subscribe()).map(move |event| {
        // On broadcast lag, emit the current revision so a slow client
        // always resyncs instead of silently missing the update.
        let revision = match event {
            Ok(revision) => revision,
            Err(_lagged) => revision_on_lag.load(Ordering::SeqCst),
        };
        Ok(vault_revision_event(revision))
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
    match run_blocking(move || cache.recently_modified_notes(limit)).await {
        Ok(notes) => (StatusCode::OK, Json(RecentlyModifiedResponse { notes })).into_response(),
        Err(err) => err.into_response(),
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
    let embedder = state.embedder.clone();

    let limit = query.limit.unwrap_or(10).clamp(1, 50);
    let per_note_cap = query.per_note_cap.unwrap_or(2).clamp(1, 10);
    let mode = query.mode.unwrap_or_default();
    if query.q.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "query cannot be empty".to_string(),
            }),
        )
            .into_response();
    }
    let q_len = query.q.len();
    debug!(
        query_len = q_len,
        ?mode,
        limit,
        per_note_cap,
        "Executing Phase 2 search"
    );

    let req = crate::search::SearchRequest {
        query: query.q,
        mode,
        limit,
        per_note_cap,
    };
    // Query embedding + SQLite work runs off the async runtime.
    let result = run_blocking(move || {
        crate::search::run(cache.as_ref(), embedder.as_ref(), req)
            .map_err(|error| format!("Search failed: {error}"))
    })
    .await;
    match result {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(err) => err.into_response(),
    }
}

pub async fn stats_handler(State(state): State<AppState>) -> impl IntoResponse {
    let cache = match sqlite_cache(&state).await {
        Ok(cache) => cache,
        Err(err) => return err.into_response(),
    };

    match run_blocking(move || cache.vault_stats()).await {
        Ok(stats) => (StatusCode::OK, Json(stats)).into_response(),
        Err(err) => err.into_response(),
    }
}

pub async fn graph_handler(State(state): State<AppState>) -> impl IntoResponse {
    let cache = match sqlite_cache(&state).await {
        Ok(cache) => cache,
        Err(err) => return err.into_response(),
    };

    match run_blocking(move || cache.graph_data()).await {
        Ok(data) => (StatusCode::OK, Json(data)).into_response(),
        Err(err) => err.into_response(),
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
