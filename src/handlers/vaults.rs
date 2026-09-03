//! `/api/v1/vaults` — the HTTP adapter over Vault collection management.
//!
//! Since #187 every route here is HTTP shaping and nothing else: parse the
//! path, query, and body, hand them to the Vault collection management core
//! ([`crate::vault_management`]), and turn the typed response or the
//! structured error into a status code and a JSON body. The registry commit,
//! the runtime reconciliation through the foreground mutation boundary, the
//! authenticated and demo projections, the starter-Vault seeding, the
//! credential-replacement Git retry, the manual sync/retry/refresh controls,
//! and the confirmed start-with-no-Vaults recovery all live there, shared with
//! the MCP management tools, which no longer proxy these handlers (ADR-19).
//!
//! Two things stay here because they are transport and have no MCP
//! counterpart: the collection-wide SSE invalidation stream, and the shared
//! `403 demo_read_only` refusal body that `src/server.rs`'s
//! `reject_demo_mutation` middleware returns before a Vault-control route
//! runs (#109).
//!
//! This is the first HTTP surface over the Vault collection registry and
//! runtime: it carries no instance-wide readiness gate, so collection
//! management, including connecting the very first Vault, stays reachable at
//! zero enabled Vaults and while the persisted registry itself is in an
//! explicit recovery state. Discovery and the event stream are pure reads and
//! stay reachable unauthenticated in demo mode.
//!
//! Exact Vault-scoped content reads and their contained resources are a
//! sibling adapter, `handlers/vault_content.rs`, mounted in the same router
//! group and reusing `VaultApiError` and the rejection-mapping helpers below.
//! One-or-all collection reads and search are `handlers/vault_collection_reads.rs`
//! (#100); Markdown mutations, attachment upload, and write-capabilities are
//! `handlers/vault_write.rs` (#101).

use std::convert::Infallible;

use axum::Json;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::WatchStream;
use tracing::error;

use crate::app_state::AppState;
use crate::vault_error::VaultOperationError;
use crate::vault_management::{
    CreateVaultRequest, EditVaultRequest, VaultCollectionManagement, VaultMutationResponse,
    VaultScheduleResponse,
};
use crate::vault_registry::VaultId;
use crate::vault_runtime::{VaultChangeCategory, VaultCollectionRevisionEvent};

/// Shared by the sibling `/api/v1/vaults/...` adapters, which parse the same
/// Vault ID out of their own paths and report the same refusal. The parsing
/// itself belongs to the collection management core, since the MCP surface
/// needs it too.
pub(crate) use crate::vault_management::parse_vault_id;

// ---------------------------------------------------------------------------
// HTTP-only request types
// ---------------------------------------------------------------------------

/// The revision precondition every Vault-control route carries in its query
/// string. MCP passes the same `expected_registry_revision` as a plain tool
/// argument, so the core takes the number rather than this extractor.
#[derive(Debug, Deserialize)]
pub struct RevisionQuery {
    pub expected_registry_revision: u64,
}

#[derive(Debug, Deserialize)]
pub struct StartWithNoVaultsRequest {
    /// The one-shot confirmation flag: a bare POST is not enough (#150),
    /// mirroring `vault_migration::start_with_no_vaults`'s own
    /// `confirmed` gate rather than duplicating it here.
    pub confirm: bool,
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// The HTTP surface's historical name for the structured error every
/// Vault-qualified core now returns. The type itself moved to
/// `crate::vault_error` (ADR-19): it is transport-neutral, and MCP already
/// re-serialised this exact shape into its own tool errors. The alias remains
/// as the spelling the sibling `/api/v1/vaults/...` adapters use.
pub type VaultApiError = VaultOperationError;

impl VaultOperationError {
    /// The HTTP adapter's half of the mapping ADR-19 describes: a core's
    /// structured error becomes a status code plus this same JSON body. It
    /// lives here rather than beside the type because it is axum-shaped.
    pub(crate) fn respond(self, status: StatusCode) -> Response {
        (status, Json(self)).into_response()
    }
}

/// Preserves the rejection's real status — e.g. `413` for a body over the
/// length limit, `415` for the wrong content type, `422` for well-formed JSON
/// missing a required field — rather than flattening every kind to `400`, so
/// clients/proxies keying off status codes are not misled. Matches
/// `vault_write.rs`'s `write_payload`, which exists for the same reason on
/// note-mutation routes.
pub(crate) fn json_rejection_response(error: JsonRejection) -> Response {
    let status = error.status();
    VaultApiError::new("invalid_request_body", error.body_text(), None, false).respond(status)
}

pub(crate) fn query_rejection_response(error: QueryRejection) -> Response {
    VaultApiError::new("invalid_request_query", error.body_text(), None, false)
        .respond(StatusCode::BAD_REQUEST)
}

pub(crate) fn internal_error_response(
    detail: impl AsRef<str>,
    vault_id: Option<VaultId>,
) -> Response {
    error!(detail = %detail.as_ref(), "Vault collection API internal error");
    VaultApiError::new("internal_error", "Internal server error", vault_id, false)
        .respond(StatusCode::INTERNAL_SERVER_ERROR)
}

/// Shared `403` refusal for every mutation and Vault-control route when demo
/// mode publishes the whole enabled Vault collection as public read-only
/// (#109). Reused directly — rather than duplicated per adapter — so
/// collection management here and content mutations/attachment upload in
/// `vault_write.rs` report the same stable `demo_read_only` code and message.
/// Carries no `vault_id`: the refusal is a global instance posture, not a
/// per-Vault condition, and applies uniformly before any per-Vault check
/// (existence, capability, and so on) runs.
pub(crate) fn demo_read_only_response() -> Response {
    VaultApiError::new(
        "demo_read_only",
        "This is a public read-only demo instance; mutations and Vault-control operations are disabled.",
        None,
        false,
    )
    .respond(StatusCode::FORBIDDEN)
}

/// The whole of this adapter's error contract: one structured core error
/// becomes a status code plus the same `{code, message, vault_id?,
/// retryable}` body this surface has always returned. The core sanitizes and
/// logs an `internal_error` itself, so nothing here re-reports it.
fn management_error_response(error: VaultOperationError) -> Response {
    let status = match error.code.as_str() {
        "invalid_vault_id" | "invalid_vault_definition" | "confirmation_required" => {
            StatusCode::BAD_REQUEST
        }
        "vault_not_found" => StatusCode::NOT_FOUND,
        // Every conflict below depends on registry state rather than on the
        // shape of this request in isolation.
        "duplicate_vault_name"
        | "vault_path_overlap"
        | "identity_change_requires_disabled"
        | "identity_change_requires_confirmation"
        | "registry_revision_conflict"
        | "legacy_migration_recovery_not_pending"
        | "vault_disabled"
        | "capability_unavailable" => StatusCode::CONFLICT,
        // Retry-after-operator-action, or retry-after-the-runtime-settles.
        "vault_registry_recovery_required"
        | "legacy_environment_cleanup_required"
        | "vault_unavailable" => StatusCode::SERVICE_UNAVAILABLE,
        // `internal_error` and `registry_revision_exhausted`.
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    error.respond(status)
}

fn mutation_response(status: StatusCode, response: VaultMutationResponse) -> Response {
    (status, Json(response)).into_response()
}

fn schedule_response(response: VaultScheduleResponse) -> Response {
    (StatusCode::ACCEPTED, Json(response)).into_response()
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/v1/vaults` — discovery. Reachable at zero enabled Vaults and
/// while the registry is in an explicit recovery state; never returns
/// credentials, only `credential_configured`. Reachable unauthenticated in
/// demo mode (#109), where the core answers with its public projection.
pub async fn list_vaults_handler(State(state): State<AppState>) -> Response {
    match VaultCollectionManagement::new(&state).list() {
        Ok(response) => Json(response).into_response(),
        Err(error) => management_error_response(error),
    }
}

/// `POST /api/v1/vaults/start-with-no-vaults` — the confirmed recovery action
/// offered only when a failed legacy import left `AppState`'s
/// `legacy_migration_recovery` set (#150).
pub async fn start_with_no_vaults_handler(
    State(state): State<AppState>,
    request: Result<Json<StartWithNoVaultsRequest>, JsonRejection>,
) -> Response {
    let request = match request {
        Ok(Json(request)) => request,
        Err(error) => return json_rejection_response(error),
    };
    match VaultCollectionManagement::new(&state)
        .start_with_no_vaults(request.confirm)
        .await
    {
        Ok(response) => mutation_response(StatusCode::OK, response),
        Err(error) => management_error_response(error),
    }
}

/// `POST /api/v1/vaults` — create a new Vault definition.
pub async fn create_vault_handler(
    State(state): State<AppState>,
    request: Result<Json<CreateVaultRequest>, JsonRejection>,
) -> Response {
    let request = match request {
        Ok(Json(request)) => request,
        Err(error) => return json_rejection_response(error),
    };
    match VaultCollectionManagement::new(&state).create(request).await {
        Ok(response) => mutation_response(StatusCode::CREATED, response),
        Err(error) => management_error_response(error),
    }
}

/// `PATCH /api/v1/vaults/{vault_id}` — edit an existing Vault definition.
pub async fn edit_vault_handler(
    State(state): State<AppState>,
    Path(raw_id): Path<String>,
    request: Result<Json<EditVaultRequest>, JsonRejection>,
) -> Response {
    let vault_id = match parse_vault_id(&raw_id) {
        Ok(vault_id) => vault_id,
        Err(error) => return management_error_response(error),
    };
    let request = match request {
        Ok(Json(request)) => request,
        Err(error) => return json_rejection_response(error),
    };
    match VaultCollectionManagement::new(&state)
        .edit(vault_id, request)
        .await
    {
        Ok(response) => mutation_response(StatusCode::OK, response),
        Err(error) => management_error_response(error),
    }
}

async fn set_enabled_handler(
    state: AppState,
    raw_id: String,
    query: Result<Query<RevisionQuery>, QueryRejection>,
    enabled: bool,
) -> Response {
    let vault_id = match parse_vault_id(&raw_id) {
        Ok(vault_id) => vault_id,
        Err(error) => return management_error_response(error),
    };
    let Query(query) = match query {
        Ok(query) => query,
        Err(error) => return query_rejection_response(error),
    };
    match VaultCollectionManagement::new(&state)
        .set_enabled(vault_id, query.expected_registry_revision, enabled)
        .await
    {
        Ok(response) => mutation_response(StatusCode::OK, response),
        Err(error) => management_error_response(error),
    }
}

/// `POST /api/v1/vaults/{vault_id}/enable`
pub async fn enable_vault_handler(
    State(state): State<AppState>,
    Path(raw_id): Path<String>,
    query: Result<Query<RevisionQuery>, QueryRejection>,
) -> Response {
    set_enabled_handler(state, raw_id, query, true).await
}

/// `POST /api/v1/vaults/{vault_id}/disable`
pub async fn disable_vault_handler(
    State(state): State<AppState>,
    Path(raw_id): Path<String>,
    query: Result<Query<RevisionQuery>, QueryRejection>,
) -> Response {
    set_enabled_handler(state, raw_id, query, false).await
}

/// `DELETE /api/v1/vaults/{vault_id}` — disconnect. Deletes no files, checkouts,
/// Git history, or credentials outside this registry record.
pub async fn disconnect_vault_handler(
    State(state): State<AppState>,
    Path(raw_id): Path<String>,
    query: Result<Query<RevisionQuery>, QueryRejection>,
) -> Response {
    let vault_id = match parse_vault_id(&raw_id) {
        Ok(vault_id) => vault_id,
        Err(error) => return management_error_response(error),
    };
    let Query(query) = match query {
        Ok(query) => query,
        Err(error) => return query_rejection_response(error),
    };
    match VaultCollectionManagement::new(&state)
        .disconnect(vault_id, query.expected_registry_revision)
        .await
    {
        Ok(response) => mutation_response(StatusCode::OK, response),
        Err(error) => management_error_response(error),
    }
}

/// `POST /api/v1/vaults/{vault_id}/sync` — request an immediate managed-Git
/// turn for one Vault, bypassing its daily schedule.
pub async fn sync_vault_handler(
    State(state): State<AppState>,
    Path(raw_id): Path<String>,
) -> Response {
    match parse_vault_id(&raw_id)
        .and_then(|vault_id| VaultCollectionManagement::new(&state).sync(vault_id))
    {
        Ok(response) => schedule_response(response),
        Err(error) => management_error_response(error),
    }
}

/// `POST /api/v1/vaults/{vault_id}/retry` — same admitted operation as sync,
/// kept as a distinctly named entry point (mirrors
/// `ManagedGitScheduler::retry_now`).
pub async fn retry_vault_handler(
    State(state): State<AppState>,
    Path(raw_id): Path<String>,
) -> Response {
    match parse_vault_id(&raw_id)
        .and_then(|vault_id| VaultCollectionManagement::new(&state).retry(vault_id))
    {
        Ok(response) => schedule_response(response),
        Err(error) => management_error_response(error),
    }
}

/// `POST /api/v1/vaults/{vault_id}/refresh` — request one Vault's next Index
/// turn. The route only admits work to the shared FIFO; the runtime worker
/// performs the authoritative Markdown scan and atomic snapshot publication.
pub async fn refresh_vault_handler(
    State(state): State<AppState>,
    Path(raw_id): Path<String>,
) -> Response {
    match parse_vault_id(&raw_id)
        .and_then(|vault_id| VaultCollectionManagement::new(&state).refresh(vault_id))
    {
        Ok(response) => schedule_response(response),
        Err(error) => management_error_response(error),
    }
}

// ---------------------------------------------------------------------------
// Collection-wide event stream
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct VaultCollectionEventPayload {
    collection_revision: u64,
    vault_ids: Vec<VaultId>,
    category: VaultChangeCategory,
}

fn collection_revision_event(event: &VaultCollectionRevisionEvent) -> Event {
    let payload = VaultCollectionEventPayload {
        collection_revision: event.collection_revision,
        vault_ids: event.vault_ids.clone(),
        category: event.category,
    };
    let data = serde_json::to_string(&payload)
        .unwrap_or_else(|_| format!(r#"{{"collection_revision":{}}}"#, event.collection_revision));
    Event::default()
        .event("vault-collection-revision")
        .id(event.collection_revision.to_string())
        .data(data)
}

/// `GET /api/v1/vaults/events` (SSE) — one authenticated collection-wide
/// invalidation stream. Carries `collection_revision`, the affected Vault
/// IDs, and a broad change category; carries no Note content. A subscriber
/// that misses an intermediate advance (the channel keeps only the latest
/// value) still learns the current revision and should refetch broadly.
pub async fn vault_collection_events_handler(
    State(state): State<AppState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let stream = WatchStream::new(VaultCollectionManagement::new(&state).subscribe_revisions())
        .map(|event| Ok(collection_revision_event(&event)));
    Sse::new(stream).keep_alive(KeepAlive::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault_management::MANAGEMENT_ERROR_CODES;
    use crate::vault_management::test_support::test_state;
    use crate::vault_registry::{
        DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS, VaultGitMode, VaultRegistryState, VaultSource,
    };
    use axum::body::to_bytes;
    use std::collections::BTreeSet;

    /// The mapping table above is this adapter's entire error contract, so it
    /// is asserted directly rather than only through the handful of routes
    /// that happen to raise a given code. The expected statuses are checked
    /// against the core's own `MANAGEMENT_ERROR_CODES`, so a code the core
    /// gains without a declared status here fails this test instead of
    /// silently falling into the table's catch-all `500` arm.
    #[test]
    fn every_management_error_code_keeps_its_historical_status() {
        let expected: Vec<(&str, StatusCode)> = vec![
            ("invalid_vault_id", StatusCode::BAD_REQUEST),
            ("invalid_vault_definition", StatusCode::BAD_REQUEST),
            ("confirmation_required", StatusCode::BAD_REQUEST),
            ("vault_not_found", StatusCode::NOT_FOUND),
            ("duplicate_vault_name", StatusCode::CONFLICT),
            ("vault_path_overlap", StatusCode::CONFLICT),
            ("identity_change_requires_disabled", StatusCode::CONFLICT),
            (
                "identity_change_requires_confirmation",
                StatusCode::CONFLICT,
            ),
            ("registry_revision_conflict", StatusCode::CONFLICT),
            (
                "legacy_migration_recovery_not_pending",
                StatusCode::CONFLICT,
            ),
            ("vault_disabled", StatusCode::CONFLICT),
            ("capability_unavailable", StatusCode::CONFLICT),
            (
                "vault_registry_recovery_required",
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            (
                "legacy_environment_cleanup_required",
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            ("vault_unavailable", StatusCode::SERVICE_UNAVAILABLE),
            (
                "registry_revision_exhausted",
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            ("internal_error", StatusCode::INTERNAL_SERVER_ERROR),
        ];

        let declared: BTreeSet<&str> = MANAGEMENT_ERROR_CODES.iter().copied().collect();
        let covered: BTreeSet<&str> = expected.iter().map(|(code, _)| *code).collect();
        assert_eq!(
            covered, declared,
            "every code the collection core reports needs a declared status here; \
             an undeclared one would silently become a 500"
        );

        for (code, status) in expected {
            let response =
                management_error_response(VaultApiError::new(code, "message", None, false));
            assert_eq!(response.status(), status, "status for {code}");
        }
    }

    /// A malformed Vault ID never reaches the core, and still refuses with
    /// the same `400` every route here has always returned.
    #[tokio::test]
    async fn a_malformed_vault_id_is_refused_before_the_core_runs() {
        let (state, _worker, _directory) = test_state();
        let response =
            sync_vault_handler(State(state.clone()), Path("not-a-uuid".to_string())).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    /// The `200` discovery body, asserted at the route rather than only at
    /// the core: the acceptance criterion is byte-identical *responses*, and
    /// this is the route the browser polls on every collection change.
    #[tokio::test]
    async fn discovery_answers_200_with_the_full_collection_body() {
        let (state, _worker, directory) = test_state();
        let path = directory.path().join("notes");
        std::fs::create_dir_all(&path).expect("vault dir");
        VaultCollectionManagement::new(&state)
            .create(CreateVaultRequest {
                expected_registry_revision: 0,
                name: "Notes".to_string(),
                enabled: true,
                source: VaultSource::Local { path },
                exclude_patterns: Vec::new(),
                https_credentials: None,
                archive_folder: None,
                commit_identity: None,
            })
            .await
            .expect("create the Vault");

        let response = list_vaults_handler(State(state.clone())).await;
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("discovery body");
        let body: serde_json::Value = serde_json::from_slice(&bytes).expect("discovery JSON");

        assert_eq!(body["registry_revision"], 1);
        assert_eq!(body["demo_mode"], false);
        assert!(body["collection_revision"].is_number());
        assert!(body.get("recovery").is_none());
        assert!(body.get("legacy_migration_recovery").is_none());
        let vaults = body["vaults"].as_array().expect("vaults array");
        assert_eq!(vaults.len(), 1);
        assert_eq!(vaults[0]["name"], "Notes");
        assert_eq!(vaults[0]["enabled"], true);
        assert_eq!(vaults[0]["credential_configured"], false);
        // An authenticated read always carries the operator's own input.
        assert!(vaults[0]["source"].is_object());
        assert!(vaults[0]["capabilities"].is_object());
    }

    /// The two statuses this adapter adds on top of the core's typed
    /// responses: `201` for a creation, `202` for admitted background work.
    #[tokio::test]
    async fn creation_answers_201_and_an_admitted_control_answers_202() {
        let (state, _worker, directory) = test_state();
        let repository_path = directory.path().join("existing-pull-only-repo");
        std::fs::create_dir_all(&repository_path).expect("create repo directory");
        git2::Repository::init(&repository_path).expect("init git repo");

        let response = create_vault_handler(
            State(state.clone()),
            Ok(Json(CreateVaultRequest {
                expected_registry_revision: 0,
                name: "Existing checkout".to_string(),
                enabled: true,
                source: VaultSource::ExistingGit {
                    repository_path,
                    repository_url: Some("https://example.test/owner/notes.git".to_string()),
                    branch: None,
                    vault_subdirectory: None,
                    mode: VaultGitMode::PullOnly,
                    poll_interval_secs: DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
                },
                exclude_patterns: Vec::new(),
                https_credentials: None,
                archive_folder: None,
                commit_identity: None,
            })),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED);

        let VaultRegistryState::Ready(snapshot) =
            state.vault_registry.load().expect("load registry")
        else {
            panic!("registry entered recovery");
        };
        let vault_id = snapshot.vault_ids().next().expect("one Vault");

        let sync = sync_vault_handler(State(state.clone()), Path(vault_id.to_string())).await;
        assert_eq!(sync.status(), StatusCode::ACCEPTED);
    }
}
