//! `/api/v1/vaults/{scope}/...` — one-or-all collection reads and search:
//! tree, recent Notes, statistics, graph, and search. `{scope}` is one
//! immutable Vault ID or the literal `all`, per
//! `docs/migrations/vault-scoped-clients.md`.
//!
//! This is an HTTP adapter over already-implemented, already-unit-tested
//! shared-core methods — [`crate::vault_read::VaultReadCore::{trees,
//! statistics, graphs, recently_modified}`] and
//! [`crate::search::vault_scoped::VaultSearchCore::search`] — which already
//! implement every one-or-all/partial/zero-Vault/grouped-vs-flattened
//! behavior issue #62 and the wire-contract spec require. This file owns
//! only query decoding and response shaping: no collection-read domain logic
//! lives here, and not even the `{scope}` grammar — `VaultScope::parse`,
//! `BrowseSurface::layer_selection`, and the limit clamps are the core's, so a
//! scope or selector one surface accepts is never one the MCP tools refuse. A
//! scope the core rejects becomes the structured `invalid_scope` error (`400`). Mounted in the same
//! `/api/v1/vaults` router group as `handlers/vaults.rs` and
//! `handlers/vault_content.rs`, sharing their auth posture, `VaultApiError`
//! shape, and rejection-mapping helpers (`query_rejection_response`,
//! `vault_read_error_response`). Every route here is a read, so — like exact
//! reads in `vault_content.rs` — none of them is wrapped in `reject_demo_mutation`
//! (#109): they stay reachable unauthenticated in demo mode, unlike the
//! mutation and Vault-control routes in `vaults.rs`/`vault_write.rs`.

use axum::Json;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::api_types::RecentlyModifiedQuery;
use crate::app_state::{AppState, internal_error, run_blocking};
use crate::handlers::vault_content::vault_read_error_response;
use crate::handlers::vaults::query_rejection_response;
use crate::search::vault_scoped::{VaultSearchCore, VaultSearchRequest};
use crate::search::{NoteFilters, SearchMode};
use crate::vault_read::{
    BrowseSurface, OffloadedReadError, VaultReads, VaultScope, clamp_recent_limit,
    clamp_search_limit, clamp_search_per_note_cap,
};

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

/// Endpoint-local query shape; `layers` is a comma-separated list of tokens
/// (`"all"`, `"default"`, or exact layer names) rather than a repeated query
/// key, avoiding any ambiguity in array-shaped query-string decoding.
#[derive(Debug, Deserialize)]
pub struct VaultScopeSearchQuery {
    pub q: String,
    #[serde(default)]
    pub mode: Option<SearchMode>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub per_note_cap: Option<usize>,
    #[serde(default)]
    pub layers: Option<String>,
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Maps one offloaded read's failure onto a response: the Vault's own
/// structured failure through the shared bucket map, and a blocking task that
/// never completed through the same opaque `500` every other instance-side
/// fault reports.
fn read_error_response(error: OffloadedReadError) -> Response {
    match error {
        OffloadedReadError::Read(error) => vault_read_error_response(error),
        OffloadedReadError::Failed(message) => internal_error(message).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/v1/vaults/{scope}/tree` — grouped per Vault.
pub async fn vault_scope_tree_handler(
    State(state): State<AppState>,
    Path(raw_scope): Path<String>,
) -> Response {
    let scope = match VaultScope::parse(&raw_scope) {
        Ok(scope) => scope,
        Err(error) => return vault_read_error_response(error),
    };
    match VaultReads::new(&state)
        .read(move |core| core.trees(scope))
        .await
    {
        Ok(projection) => (StatusCode::OK, Json(projection)).into_response(),
        Err(error) => read_error_response(error),
    }
}

/// `GET /api/v1/vaults/{scope}/stats` — grouped per Vault.
pub async fn vault_scope_stats_handler(
    State(state): State<AppState>,
    Path(raw_scope): Path<String>,
) -> Response {
    let scope = match VaultScope::parse(&raw_scope) {
        Ok(scope) => scope,
        Err(error) => return vault_read_error_response(error),
    };
    match VaultReads::new(&state)
        .read(move |core| core.statistics(scope))
        .await
    {
        Ok(projection) => (StatusCode::OK, Json(projection)).into_response(),
        Err(error) => read_error_response(error),
    }
}

/// `GET /api/v1/vaults/{scope}/graph` — grouped per Vault; edges never cross
/// a Vault boundary (enforced by `VaultReadCore::graphs` itself).
pub async fn vault_scope_graph_handler(
    State(state): State<AppState>,
    Path(raw_scope): Path<String>,
) -> Response {
    let scope = match VaultScope::parse(&raw_scope) {
        Ok(scope) => scope,
        Err(error) => return vault_read_error_response(error),
    };
    match VaultReads::new(&state)
        .read(move |core| core.graphs(scope))
        .await
    {
        Ok(projection) => (StatusCode::OK, Json(projection)).into_response(),
        Err(error) => read_error_response(error),
    }
}

/// `GET /api/v1/vaults/{scope}/recent?limit=..` — flattened across Vaults.
/// Mirrors the legacy `/api/recently-modified` default/clamp (5, 1..25).
pub async fn vault_scope_recent_handler(
    State(state): State<AppState>,
    Path(raw_scope): Path<String>,
    query: Result<Query<RecentlyModifiedQuery>, QueryRejection>,
) -> Response {
    let scope = match VaultScope::parse(&raw_scope) {
        Ok(scope) => scope,
        Err(error) => return vault_read_error_response(error),
    };
    let Query(query) = match query {
        Ok(query) => query,
        Err(error) => return query_rejection_response(error),
    };
    let limit = clamp_recent_limit(query.limit);
    match VaultReads::new(&state)
        .read(move |core| core.recently_modified(scope, limit))
        .await
    {
        Ok(projection) => (StatusCode::OK, Json(projection)).into_response(),
        Err(error) => read_error_response(error),
    }
}

/// `GET /api/v1/vaults/{scope}/search?q=..&mode=..&limit=..&per_note_cap=..&layers=..`
/// — one global ranking across every usable participant, flattened across
/// Vaults. Mirrors the legacy `/api/search` defaults/clamps (limit 10 max
/// 50; per_note_cap 2, 1..10). Never exposes `NoteFilters`/
/// `include_properties` over this route, matching the legacy web search
/// route's posture — those remain MCP/eval-only.
pub async fn vault_scope_search_handler(
    State(state): State<AppState>,
    Path(raw_scope): Path<String>,
    query: Result<Query<VaultScopeSearchQuery>, QueryRejection>,
) -> Response {
    let scope = match VaultScope::parse(&raw_scope) {
        Ok(scope) => scope,
        Err(error) => return vault_read_error_response(error),
    };
    let Query(query) = match query {
        Ok(query) => query,
        Err(error) => return query_rejection_response(error),
    };
    let cache = state.startup_sqlite.clone();
    let vaults = state.vaults.clone();
    let surface = BrowseSurface::for_demo_mode(state.demo_mode);
    let embedder = state.embedder.clone();
    let request = VaultSearchRequest {
        scope,
        query: query.q,
        mode: query.mode.unwrap_or_default(),
        limit: clamp_search_limit(query.limit),
        per_note_cap: clamp_search_per_note_cap(query.per_note_cap),
        filters: NoteFilters::default(),
        include_properties: Vec::new(),
        layers: surface.layer_selection(query.layers.as_deref()),
    };
    // Query embedding (semantic mode) and SQLite work both run off the async
    // runtime, mirroring the legacy `search_handler`.
    let result = run_blocking(move || {
        let core = VaultSearchCore::new(&cache, &vaults, embedder.as_ref()).on_surface(surface);
        Ok(core.search(request))
    })
    .await;
    match result {
        Ok(Ok(projection)) => (StatusCode::OK, Json(projection)).into_response(),
        Ok(Err(error)) => vault_read_error_response(error),
        Err(error) => error.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The adapter's own mapping test: a scope the core refuses becomes this
    /// route's `400`. What counts as a valid scope is the core's decision and
    /// is tested there.
    #[test]
    fn a_malformed_scope_segment_is_a_structured_400() {
        let error = VaultScope::parse("not-a-scope").expect_err("malformed scope rejected");
        assert_eq!(error.code, "invalid_scope");
        assert_eq!(
            vault_read_error_response(error).status(),
            StatusCode::BAD_REQUEST
        );
    }
}
