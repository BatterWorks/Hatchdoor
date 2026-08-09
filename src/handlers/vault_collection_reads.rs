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
//! only scope parsing, query decoding, and response shaping: no
//! collection-read domain logic lives here. Mounted in the same
//! `/api/v1/vaults` router group as `handlers/vaults.rs` and
//! `handlers/vault_content.rs`, sharing their auth posture, `VaultApiError`
//! shape, and rejection-mapping helpers (`query_rejection_response`,
//! `vault_read_error_response`). Every route here is a read, so — like exact
//! reads in `vault_content.rs` — none of them is wrapped in `reject_demo_mutation`
//! (#109): they stay reachable unauthenticated in demo mode, unlike the
//! mutation and Vault-control routes in `vaults.rs`/`vault_write.rs`.

use std::collections::BTreeSet;
use std::str::FromStr;

use axum::Json;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::api_types::RecentlyModifiedQuery;
use crate::app_state::{AppState, run_blocking};
use crate::handlers::vault_content::vault_read_error_response;
use crate::handlers::vaults::{VaultApiError, query_rejection_response};
use crate::search::vault_scoped::{VaultSearchCore, VaultSearchRequest};
use crate::search::{LayerSelection, NoteFilters, SearchMode};
use crate::vault_read::{VaultReadCore, VaultScope};
use crate::vault_registry::VaultId;

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

/// Parses the `{scope}` path segment: the literal `all`, or a canonical Vault
/// ID. Anything else is the structured `invalid_scope` error (`400`), one of
/// the domain error codes issue #62 requires at least.
fn parse_vault_scope(raw: &str) -> Result<VaultScope, VaultApiError> {
    if raw == "all" {
        return Ok(VaultScope::All);
    }
    VaultId::from_str(raw).map(VaultScope::One).map_err(|_| {
        VaultApiError::new(
            "invalid_scope",
            "scope must be the literal 'all' or a canonical Vault ID",
            None,
            false,
        )
    })
}

fn bad_scope(error: VaultApiError) -> Response {
    error.respond(StatusCode::BAD_REQUEST)
}

/// Builds a [`LayerSelection`] from raw, comma-separated HTTP query tokens.
/// Deliberately does not consult any one Vault's known-layer catalog while
/// parsing, unlike [`LayerSelection::parse`] (built for the single-Vault MCP
/// surface, where an unrecognized token degrades to the default surface with
/// a warning): issue #62 applies one layer selector *independently* to every
/// participant, so a name valid in one Vault and absent from another is not a
/// parse-time concern — `VaultSearchCore::search`'s own
/// `invalid_layer_selection` check already covers the only real error case, a
/// named layer absent from every usable participant.
fn parse_layer_selection(raw: Option<&str>) -> LayerSelection {
    let Some(raw) = raw else {
        return LayerSelection::default_surface();
    };
    let tokens: Vec<&str> = raw
        .split(',')
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .collect();
    if tokens.is_empty() {
        return LayerSelection::default_surface();
    }
    if tokens.iter().any(|token| token.eq_ignore_ascii_case("all")) {
        return LayerSelection::All;
    }
    let mut include_default = false;
    let mut layers = BTreeSet::new();
    for token in tokens {
        if token.eq_ignore_ascii_case("default") {
            include_default = true;
        } else {
            layers.insert(token.to_string());
        }
    }
    if !include_default && layers.is_empty() {
        include_default = true;
    }
    LayerSelection::Set {
        include_default,
        layers,
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
    let scope = match parse_vault_scope(&raw_scope) {
        Ok(scope) => scope,
        Err(error) => return bad_scope(error),
    };
    let cache = state.startup_sqlite.clone();
    let vaults = state.vaults.clone();
    let result = run_blocking(move || {
        let core = VaultReadCore::new(&cache, &vaults);
        Ok(core.trees(scope))
    })
    .await;
    match result {
        Ok(Ok(projection)) => (StatusCode::OK, Json(projection)).into_response(),
        Ok(Err(error)) => vault_read_error_response(error),
        Err(error) => error.into_response(),
    }
}

/// `GET /api/v1/vaults/{scope}/stats` — grouped per Vault.
pub async fn vault_scope_stats_handler(
    State(state): State<AppState>,
    Path(raw_scope): Path<String>,
) -> Response {
    let scope = match parse_vault_scope(&raw_scope) {
        Ok(scope) => scope,
        Err(error) => return bad_scope(error),
    };
    let cache = state.startup_sqlite.clone();
    let vaults = state.vaults.clone();
    let result = run_blocking(move || {
        let core = VaultReadCore::new(&cache, &vaults);
        Ok(core.statistics(scope))
    })
    .await;
    match result {
        Ok(Ok(projection)) => (StatusCode::OK, Json(projection)).into_response(),
        Ok(Err(error)) => vault_read_error_response(error),
        Err(error) => error.into_response(),
    }
}

/// `GET /api/v1/vaults/{scope}/graph` — grouped per Vault; edges never cross
/// a Vault boundary (enforced by `VaultReadCore::graphs` itself).
pub async fn vault_scope_graph_handler(
    State(state): State<AppState>,
    Path(raw_scope): Path<String>,
) -> Response {
    let scope = match parse_vault_scope(&raw_scope) {
        Ok(scope) => scope,
        Err(error) => return bad_scope(error),
    };
    let cache = state.startup_sqlite.clone();
    let vaults = state.vaults.clone();
    let result = run_blocking(move || {
        let core = VaultReadCore::new(&cache, &vaults);
        Ok(core.graphs(scope))
    })
    .await;
    match result {
        Ok(Ok(projection)) => (StatusCode::OK, Json(projection)).into_response(),
        Ok(Err(error)) => vault_read_error_response(error),
        Err(error) => error.into_response(),
    }
}

/// `GET /api/v1/vaults/{scope}/recent?limit=..` — flattened across Vaults.
/// Mirrors the legacy `/api/recently-modified` default/clamp (5, 1..25).
pub async fn vault_scope_recent_handler(
    State(state): State<AppState>,
    Path(raw_scope): Path<String>,
    query: Result<Query<RecentlyModifiedQuery>, QueryRejection>,
) -> Response {
    let scope = match parse_vault_scope(&raw_scope) {
        Ok(scope) => scope,
        Err(error) => return bad_scope(error),
    };
    let Query(query) = match query {
        Ok(query) => query,
        Err(error) => return query_rejection_response(error),
    };
    let limit = query.limit.unwrap_or(5).clamp(1, 25);
    let cache = state.startup_sqlite.clone();
    let vaults = state.vaults.clone();
    let result = run_blocking(move || {
        let core = VaultReadCore::new(&cache, &vaults);
        Ok(core.recently_modified(scope, limit))
    })
    .await;
    match result {
        Ok(Ok(projection)) => (StatusCode::OK, Json(projection)).into_response(),
        Ok(Err(error)) => vault_read_error_response(error),
        Err(error) => error.into_response(),
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
    let scope = match parse_vault_scope(&raw_scope) {
        Ok(scope) => scope,
        Err(error) => return bad_scope(error),
    };
    let Query(query) = match query {
        Ok(query) => query,
        Err(error) => return query_rejection_response(error),
    };
    let limit = query.limit.unwrap_or(10).clamp(1, 50);
    let per_note_cap = query.per_note_cap.unwrap_or(2).clamp(1, 10);
    let mode = query.mode.unwrap_or_default();
    let layers = parse_layer_selection(query.layers.as_deref());

    let cache = state.startup_sqlite.clone();
    let vaults = state.vaults.clone();
    let embedder = state.embedder.clone();
    let request = VaultSearchRequest {
        scope,
        query: query.q,
        mode,
        limit,
        per_note_cap,
        filters: NoteFilters::default(),
        include_properties: Vec::new(),
        layers,
    };
    // Query embedding (semantic mode) and SQLite work both run off the async
    // runtime, mirroring the legacy `search_handler`.
    let result = run_blocking(move || {
        let core = VaultSearchCore::new(&cache, &vaults, embedder.as_ref());
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

    #[test]
    fn parse_vault_scope_accepts_all_and_canonical_ids_and_rejects_everything_else() {
        assert_eq!(
            parse_vault_scope("all").expect("all parses"),
            VaultScope::All
        );

        let vault_id = VaultId::generate().expect("generate Vault id");
        assert_eq!(
            parse_vault_scope(&vault_id.to_string()).expect("uuid parses"),
            VaultScope::One(vault_id)
        );

        let error = parse_vault_scope("not-a-scope").expect_err("malformed scope rejected");
        assert_eq!(error.code, "invalid_scope");

        let error = parse_vault_scope("All").expect_err("case-sensitive literal only");
        assert_eq!(error.code, "invalid_scope");
    }

    #[test]
    fn parse_layer_selection_matches_default_all_and_named_token_semantics() {
        assert_eq!(
            parse_layer_selection(None),
            LayerSelection::default_surface()
        );
        assert_eq!(
            parse_layer_selection(Some("")),
            LayerSelection::default_surface()
        );
        assert_eq!(parse_layer_selection(Some("all")), LayerSelection::All);
        assert_eq!(parse_layer_selection(Some("ALL")), LayerSelection::All);

        assert_eq!(
            parse_layer_selection(Some("sources")),
            LayerSelection::Set {
                include_default: false,
                layers: ["sources".to_string()].into_iter().collect(),
            }
        );
        assert_eq!(
            parse_layer_selection(Some("default, sources")),
            LayerSelection::Set {
                include_default: true,
                layers: ["sources".to_string()].into_iter().collect(),
            }
        );
    }
}
