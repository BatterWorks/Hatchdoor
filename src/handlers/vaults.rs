//! `/api/v1/vaults` — authenticated Vault discovery, collection management,
//! status, and the collection-wide invalidation event stream.
//!
//! This is the first HTTP surface over the Vault collection registry and
//! runtime: it is deliberately independent of `require_vault_ready` (a
//! legacy single-configured-Vault gate) so collection management, including
//! connecting the very first Vault, stays reachable at zero enabled Vaults
//! and while the persisted registry itself is in an explicit recovery state.
//! Exact Vault-scoped content reads and their contained resources are a
//! sibling adapter, `handlers/vault_content.rs`, mounted in the same router
//! group and reusing `VaultApiError` and the rejection-mapping helpers below.
//! One-or-all collection reads and search are `handlers/vault_collection_reads.rs`
//! (#100); Markdown mutations, attachment upload, and write-capabilities are
//! `handlers/vault_write.rs` (#101), which retired the entire legacy unscoped
//! API in the same change. MCP discovery is #103.
//!
//! Discovery and the event stream are pure reads and stay reachable
//! unauthenticated in demo mode; collection management (create/edit/enable/
//! disable/disconnect) and manual Git sync/retry are Vault-control
//! operations, so `src/server.rs` wraps each of their routes in
//! `reject_demo_mutation` (#109), which calls this file's
//! `demo_read_only_response` to refuse with the shared `403 demo_read_only`
//! error before any registry mutation runs.

use std::convert::Infallible;
use std::str::FromStr;

use axum::Json;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::WatchStream;
use tracing::{error, warn};

use crate::app_state::AppState;
use crate::vault_registry::{
    HttpsCredentials, NewVaultDefinition, VaultDefinition, VaultDefinitionEdit,
    VaultDefinitionError, VaultId, VaultRegistryError, VaultRegistryRecovery,
    VaultRegistryRecoveryKind, VaultRegistrySnapshot, VaultRegistryState, VaultSource,
};
use crate::vault_runtime::{
    CollectionVaultSnapshot, LocalContentStatus, VaultActivationStatus, VaultCapabilities,
    VaultChangeCategory, VaultCollectionRevisionEvent, VaultGitStatus, VaultRuntimeError,
    VaultSearchStatus, VaultWatcherStatus,
};
use crate::vault_work::ScheduleResult;

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct VaultSummary {
    pub vault_id: VaultId,
    pub name: String,
    pub enabled: bool,
    pub source: VaultSource,
    pub exclude_patterns: Vec<String>,
    pub credential_configured: bool,
    pub activation: VaultActivationStatus,
    pub local_content: LocalContentStatus,
    pub search: VaultSearchStatus,
    pub git: VaultGitStatus,
    pub watcher: VaultWatcherStatus,
    pub capabilities: VaultCapabilities,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activation_error: Option<VaultRuntimeError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_error: Option<VaultRuntimeError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_error: Option<VaultRuntimeError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub watcher_error: Option<VaultRuntimeError>,
}

#[derive(Debug, Serialize)]
pub struct RegistryRecoveryInfo {
    pub code: &'static str,
    pub kind: &'static str,
    pub message: String,
}

impl From<&VaultRegistryRecovery> for RegistryRecoveryInfo {
    fn from(recovery: &VaultRegistryRecovery) -> Self {
        let kind = match recovery.kind() {
            VaultRegistryRecoveryKind::Corrupt => "corrupt",
            VaultRegistryRecoveryKind::UnsupportedSchema { .. } => "unsupported_schema",
            VaultRegistryRecoveryKind::FutureSchema { .. } => "future_schema",
        };
        Self {
            code: "vault_registry_recovery_required",
            kind,
            message: recovery.message().to_string(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct VaultDiscoveryResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registry_revision: Option<u64>,
    pub collection_revision: u64,
    pub vaults: Vec<VaultSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery: Option<RegistryRecoveryInfo>,
}

#[derive(Debug, Serialize)]
pub struct VaultMutationResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vault: Option<VaultSummary>,
    pub registry_revision: u64,
    pub collection_revision: u64,
}

#[derive(Debug, Serialize)]
pub struct VaultScheduleResponse {
    pub vault_id: VaultId,
    pub schedule: &'static str,
}

#[derive(Debug, Serialize)]
pub struct VaultApiError {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vault_id: Option<VaultId>,
    pub retryable: bool,
}

impl VaultApiError {
    pub(crate) fn new(
        code: &'static str,
        message: impl Into<String>,
        vault_id: Option<VaultId>,
        retryable: bool,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            vault_id,
            retryable,
        }
    }

    pub(crate) fn respond(self, status: StatusCode) -> Response {
        (status, Json(self)).into_response()
    }
}

#[derive(Debug, Deserialize)]
pub struct HttpsCredentialsInput {
    pub username: String,
    pub token: String,
}

impl From<HttpsCredentialsInput> for HttpsCredentials {
    fn from(value: HttpsCredentialsInput) -> Self {
        Self {
            username: value.username,
            token: value.token,
        }
    }
}

/// Explicit three-state credential update, mirroring
/// `vault_registry::HttpsCredentialUpdate`. An explicit tag (rather than a
/// nullable field) keeps "leave the stored credential alone" distinguishable
/// from "clear it" without relying on JSON-null-vs-absent ambiguity.
#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum HttpsCredentialsPatch {
    Keep,
    Remove,
    Replace { username: String, token: String },
}

impl From<HttpsCredentialsPatch> for crate::vault_registry::HttpsCredentialUpdate {
    fn from(value: HttpsCredentialsPatch) -> Self {
        match value {
            HttpsCredentialsPatch::Keep => Self::Keep,
            HttpsCredentialsPatch::Remove => Self::Remove,
            HttpsCredentialsPatch::Replace { username, token } => {
                Self::Replace(HttpsCredentials { username, token })
            }
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_keep_credentials() -> HttpsCredentialsPatch {
    HttpsCredentialsPatch::Keep
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateVaultRequest {
    pub expected_registry_revision: u64,
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub source: VaultSource,
    #[serde(default)]
    pub exclude_patterns: Vec<String>,
    #[serde(default)]
    pub https_credentials: Option<HttpsCredentialsInput>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditVaultRequest {
    pub expected_registry_revision: u64,
    pub name: String,
    pub source: VaultSource,
    #[serde(default)]
    pub exclude_patterns: Vec<String>,
    #[serde(default = "default_keep_credentials")]
    pub https_credentials: HttpsCredentialsPatch,
    #[serde(default)]
    pub confirm_identity_change: bool,
}

#[derive(Debug, Deserialize)]
pub struct RevisionQuery {
    pub expected_registry_revision: u64,
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

pub(crate) fn parse_vault_id(raw: &str) -> Result<VaultId, VaultApiError> {
    VaultId::from_str(raw).map_err(|_| {
        VaultApiError::new(
            "invalid_vault_id",
            "Vault ID must be a canonical UUID v4",
            None,
            false,
        )
    })
}

pub(crate) fn json_rejection_response(error: JsonRejection) -> Response {
    VaultApiError::new("invalid_request_body", error.body_text(), None, false)
        .respond(StatusCode::BAD_REQUEST)
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

fn recovery_response(recovery: &VaultRegistryRecovery) -> Response {
    VaultApiError::new(
        "vault_registry_recovery_required",
        recovery.message().to_string(),
        None,
        true,
    )
    .respond(StatusCode::SERVICE_UNAVAILABLE)
}

fn definition_error_response(error: VaultDefinitionError, vault_id: Option<VaultId>) -> Response {
    let message = error.to_string();
    let (code, status) = match error {
        VaultDefinitionError::VaultNotFound => ("vault_not_found", StatusCode::NOT_FOUND),
        // Both depend on other current registry records, not on the shape of
        // this request in isolation, so they are state conflicts (409) rather
        // than malformed input (400) — the same request body would succeed
        // against a different existing registry state.
        VaultDefinitionError::DuplicateName => ("duplicate_vault_name", StatusCode::CONFLICT),
        VaultDefinitionError::PathOverlap => ("vault_path_overlap", StatusCode::CONFLICT),
        VaultDefinitionError::IdentityChangeRequiresDisabled => {
            ("identity_change_requires_disabled", StatusCode::CONFLICT)
        }
        VaultDefinitionError::IdentityChangeRequiresConfirmation => (
            "identity_change_requires_confirmation",
            StatusCode::CONFLICT,
        ),
        VaultDefinitionError::InvalidName
        | VaultDefinitionError::InvalidExclusionPattern
        | VaultDefinitionError::InvalidSource(_) => {
            ("invalid_vault_definition", StatusCode::BAD_REQUEST)
        }
    };
    VaultApiError::new(code, message, vault_id, false).respond(status)
}

fn registry_error_response(error: VaultRegistryError, vault_id: Option<VaultId>) -> Response {
    match error {
        VaultRegistryError::RevisionConflict { expected, actual } => VaultApiError::new(
            "registry_revision_conflict",
            format!("expected registry revision {expected}, current revision is {actual}"),
            vault_id,
            true,
        )
        .respond(StatusCode::CONFLICT),
        VaultRegistryError::RecoveryRequired => VaultApiError::new(
            "vault_registry_recovery_required",
            "The Vault registry requires operator recovery and cannot be written until it is restored.",
            vault_id,
            true,
        )
        .respond(StatusCode::SERVICE_UNAVAILABLE),
        VaultRegistryError::RevisionExhausted => {
            let message = error.to_string();
            VaultApiError::new("registry_revision_exhausted", message, vault_id, false)
                .respond(StatusCode::INTERNAL_SERVER_ERROR)
        }
        VaultRegistryError::LockPoisoned => internal_error_response("Vault registry write lock poisoned", vault_id),
        VaultRegistryError::InvalidDefinition(definition_error) => {
            definition_error_response(definition_error, vault_id)
        }
        VaultRegistryError::Storage(detail) => internal_error_response(detail, vault_id),
    }
}

/// The status projection every registry-known Vault should have once
/// reconciled. A missing entry means this request observed a registry commit
/// the runtime has not reconciled yet — every mutation handler below commits
/// and reconciles within the same request, so this is defensive rather than
/// an expected path, and is reported as a retryable Vault-scoped error rather
/// than a panic.
fn unreconciled_snapshot(definition: &VaultDefinition) -> CollectionVaultSnapshot {
    CollectionVaultSnapshot {
        vault_id: definition.vault_id(),
        name: definition.name().to_string(),
        enabled: definition.enabled(),
        activation: VaultActivationStatus::Unavailable,
        local_content: LocalContentStatus::Unavailable,
        search: VaultSearchStatus::Unavailable,
        git: VaultGitStatus::Disabled,
        watcher: VaultWatcherStatus::Disabled,
        capabilities: VaultCapabilities::default(),
        activation_error: Some(VaultRuntimeError {
            code: "vault_runtime_not_reconciled".to_string(),
            message: "This Vault's registry commit has not been reconciled into the live \
                      collection yet; retry shortly."
                .to_string(),
            retryable: true,
        }),
        search_error: None,
        git_error: None,
        watcher_error: None,
    }
}

fn vault_summary(definition: &VaultDefinition, snapshot: &CollectionVaultSnapshot) -> VaultSummary {
    VaultSummary {
        vault_id: snapshot.vault_id,
        name: snapshot.name.clone(),
        enabled: snapshot.enabled,
        source: definition.source().clone(),
        exclude_patterns: definition.exclude_patterns().to_vec(),
        credential_configured: definition.credential_configured(),
        activation: snapshot.activation,
        local_content: snapshot.local_content,
        search: snapshot.search,
        git: snapshot.git,
        watcher: snapshot.watcher,
        capabilities: snapshot.capabilities,
        activation_error: snapshot.activation_error.clone(),
        search_error: snapshot.search_error.clone(),
        git_error: snapshot.git_error.clone(),
        watcher_error: snapshot.watcher_error.clone(),
    }
}

fn vault_summary_for(
    collection_snapshot: &crate::vault_runtime::VaultCollectionSnapshot,
    registry_snapshot: &VaultRegistrySnapshot,
    vault_id: VaultId,
) -> Option<VaultSummary> {
    let definition = registry_snapshot.definition(vault_id)?;
    let runtime_snapshot = collection_snapshot
        .vaults
        .get(&vault_id)
        .cloned()
        .unwrap_or_else(|| unreconciled_snapshot(&definition));
    Some(vault_summary(&definition, &runtime_snapshot))
}

/// Bound on how long a mutation response waits for the live runtime to catch
/// up with the registry commit it just made. Retiring a Vault (disable,
/// disconnect, or an identity-bearing edit) waits for that Vault's already
/// in-flight background Git/Index turn to reach its safe boundary rather than
/// force-cancelling it — for a large managed-Git clone that can take minutes.
/// The registry commit this reconciles is already durable by the time this
/// runs, and the affected Vault's control-block swap happens synchronously
/// inside `reconcile_and_reconstruct` before its first await, so a timeout
/// here only means this one response is built slightly ahead of full
/// background-work bookkeeping — not that the response is wrong. Spawning
/// (rather than racing the call directly) keeps reconciliation running to
/// completion in the background even after a timeout: `/api/v1/vaults` and
/// the event stream reflect it once it finishes.
const RECONCILE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

async fn reconcile_after_commit(state: &AppState, snapshot: &VaultRegistrySnapshot) {
    let vaults = state.vaults.clone();
    let registry = state.vault_registry.clone();
    let vault_work = state.vault_work.clone();
    let managed_git = state.managed_git.clone();
    let snapshot = snapshot.clone();
    let reconciled = tokio::spawn(async move {
        vaults
            .reconcile_and_reconstruct(&registry, &snapshot, &vault_work, &managed_git)
            .await;
    });
    if tokio::time::timeout(RECONCILE_TIMEOUT, reconciled)
        .await
        .is_err()
    {
        warn!(
            "Vault collection reconciliation is still catching up on an in-flight background \
             turn after a registry mutation; it continues in the background and this response \
             reflects the registry commit, not yet the fully reconciled runtime status"
        );
    }
}

fn mutation_response(
    state: &AppState,
    snapshot: &VaultRegistrySnapshot,
    vault_id: Option<VaultId>,
) -> Response {
    // One snapshot for both the reported `collection_revision` and the
    // returned Vault's status, so the two can never disagree about which
    // collection state they describe.
    let collection_snapshot = state.vaults.snapshot();
    let vault =
        vault_id.and_then(|vault_id| vault_summary_for(&collection_snapshot, snapshot, vault_id));
    Json(VaultMutationResponse {
        vault,
        registry_revision: snapshot.revision(),
        collection_revision: collection_snapshot.collection_revision,
    })
    .into_response()
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/v1/vaults` — authenticated discovery. Reachable at zero enabled
/// Vaults and while the registry is in an explicit recovery state; never
/// returns credentials, only `credential_configured`.
pub async fn list_vaults_handler(State(state): State<AppState>) -> Response {
    match state.vault_registry.load() {
        Ok(VaultRegistryState::Ready(registry_snapshot)) => {
            let collection_snapshot = state.vaults.snapshot();
            let vaults = registry_snapshot
                .definitions()
                .map(|definition| {
                    let runtime_snapshot = collection_snapshot
                        .vaults
                        .get(&definition.vault_id())
                        .cloned()
                        .unwrap_or_else(|| unreconciled_snapshot(&definition));
                    vault_summary(&definition, &runtime_snapshot)
                })
                .collect();
            Json(VaultDiscoveryResponse {
                registry_revision: Some(registry_snapshot.revision()),
                collection_revision: collection_snapshot.collection_revision,
                vaults,
                recovery: None,
            })
            .into_response()
        }
        Ok(VaultRegistryState::Recovery(recovery)) => Json(VaultDiscoveryResponse {
            registry_revision: None,
            collection_revision: 0,
            vaults: Vec::new(),
            recovery: Some(RegistryRecoveryInfo::from(&recovery)),
        })
        .into_response(),
        Err(error) => internal_error_response(error.to_string(), None),
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

    // `VaultRegistryStore::add` generates the new Vault's ID internally but
    // returns only the resulting snapshot, so the created ID is recovered by
    // diffing the ID sets before and after. This is sound only because a
    // successful `add` commits from exactly the revision this `load` observed
    // (its own internal compare-and-swap rejects any commit that raced in
    // between as a `RevisionConflict`, which never reaches this diff) — a
    // future change to that CAS contract would need to preserve this
    // guarantee or expose the generated ID directly.
    let before_ids: std::collections::BTreeSet<VaultId> = match state.vault_registry.load() {
        Ok(VaultRegistryState::Ready(snapshot)) => snapshot.vault_ids().collect(),
        Ok(VaultRegistryState::Recovery(recovery)) => return recovery_response(&recovery),
        Err(error) => return internal_error_response(error.to_string(), None),
    };

    let definition = NewVaultDefinition {
        name: request.name,
        enabled: request.enabled,
        source: request.source,
        exclude_patterns: request.exclude_patterns,
        https_credentials: request.https_credentials.map(Into::into),
    };
    match state
        .vault_registry
        .add(request.expected_registry_revision, definition)
    {
        Ok(snapshot) => {
            let vault_id = snapshot.vault_ids().find(|id| !before_ids.contains(id));
            reconcile_after_commit(&state, &snapshot).await;
            (
                StatusCode::CREATED,
                mutation_response(&state, &snapshot, vault_id),
            )
                .into_response()
        }
        Err(error) => registry_error_response(error, None),
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
        Err(error) => return error.respond(StatusCode::BAD_REQUEST),
    };
    let request = match request {
        Ok(Json(request)) => request,
        Err(error) => return json_rejection_response(error),
    };

    let edit = VaultDefinitionEdit {
        name: request.name,
        source: request.source,
        exclude_patterns: request.exclude_patterns,
        https_credentials: request.https_credentials.into(),
        confirm_identity_change: request.confirm_identity_change,
    };
    match state
        .vault_registry
        .edit(request.expected_registry_revision, vault_id, edit)
    {
        Ok(snapshot) => {
            reconcile_after_commit(&state, &snapshot).await;
            mutation_response(&state, &snapshot, Some(vault_id))
        }
        Err(error) => registry_error_response(error, Some(vault_id)),
    }
}

async fn set_enabled_handler(
    state: AppState,
    vault_id: VaultId,
    query: RevisionQuery,
    enabled: bool,
) -> Response {
    let result = if enabled {
        state
            .vault_registry
            .enable(query.expected_registry_revision, vault_id)
    } else {
        state
            .vault_registry
            .disable(query.expected_registry_revision, vault_id)
    };
    match result {
        Ok(snapshot) => {
            reconcile_after_commit(&state, &snapshot).await;
            mutation_response(&state, &snapshot, Some(vault_id))
        }
        Err(error) => registry_error_response(error, Some(vault_id)),
    }
}

/// `POST /api/v1/vaults/{vault_id}/enable`
pub async fn enable_vault_handler(
    State(state): State<AppState>,
    Path(raw_id): Path<String>,
    query: Result<Query<RevisionQuery>, QueryRejection>,
) -> Response {
    let vault_id = match parse_vault_id(&raw_id) {
        Ok(vault_id) => vault_id,
        Err(error) => return error.respond(StatusCode::BAD_REQUEST),
    };
    let Query(query) = match query {
        Ok(query) => query,
        Err(error) => return query_rejection_response(error),
    };
    set_enabled_handler(state, vault_id, query, true).await
}

/// `POST /api/v1/vaults/{vault_id}/disable`
pub async fn disable_vault_handler(
    State(state): State<AppState>,
    Path(raw_id): Path<String>,
    query: Result<Query<RevisionQuery>, QueryRejection>,
) -> Response {
    let vault_id = match parse_vault_id(&raw_id) {
        Ok(vault_id) => vault_id,
        Err(error) => return error.respond(StatusCode::BAD_REQUEST),
    };
    let Query(query) = match query {
        Ok(query) => query,
        Err(error) => return query_rejection_response(error),
    };
    set_enabled_handler(state, vault_id, query, false).await
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
        Err(error) => return error.respond(StatusCode::BAD_REQUEST),
    };
    let Query(query) = match query {
        Ok(query) => query,
        Err(error) => return query_rejection_response(error),
    };
    match state
        .vault_registry
        .disconnect(query.expected_registry_revision, vault_id)
    {
        Ok(snapshot) => {
            reconcile_after_commit(&state, &snapshot).await;
            mutation_response(&state, &snapshot, None)
        }
        Err(error) => registry_error_response(error, Some(vault_id)),
    }
}

async fn managed_git_control_handler(state: AppState, vault_id: VaultId, retry: bool) -> Response {
    let registry_snapshot = match state.vault_registry.load() {
        Ok(VaultRegistryState::Ready(snapshot)) => snapshot,
        Ok(VaultRegistryState::Recovery(recovery)) => return recovery_response(&recovery),
        Err(error) => return internal_error_response(error.to_string(), Some(vault_id)),
    };
    let Some(definition) = registry_snapshot.definition(vault_id) else {
        return VaultApiError::new(
            "vault_not_found",
            "Vault definition was not found",
            Some(vault_id),
            false,
        )
        .respond(StatusCode::NOT_FOUND);
    };
    if !definition.enabled() {
        return VaultApiError::new("vault_disabled", "Vault is disabled", Some(vault_id), false)
            .respond(StatusCode::CONFLICT);
    }
    if !matches!(definition.source(), VaultSource::ManagedGit { .. }) {
        return VaultApiError::new(
            "capability_unavailable",
            "Manual Git sync is only available for managed-Git Vaults",
            Some(vault_id),
            false,
        )
        .respond(StatusCode::CONFLICT);
    }

    let schedule = if retry {
        state.managed_git.retry_now(vault_id)
    } else {
        state.managed_git.sync_now(vault_id)
    };
    match schedule {
        ScheduleResult::Queued => (
            StatusCode::ACCEPTED,
            Json(VaultScheduleResponse {
                vault_id,
                schedule: "queued",
            }),
        )
            .into_response(),
        ScheduleResult::Coalesced => (
            StatusCode::ACCEPTED,
            Json(VaultScheduleResponse {
                vault_id,
                schedule: "coalesced",
            }),
        )
            .into_response(),
        ScheduleResult::Rejected => VaultApiError::new(
            "vault_unavailable",
            "Vault is not currently accepting background work",
            Some(vault_id),
            true,
        )
        .respond(StatusCode::SERVICE_UNAVAILABLE),
    }
}

/// `POST /api/v1/vaults/{vault_id}/sync` — request an immediate managed-Git
/// turn for one Vault, bypassing its daily schedule.
pub async fn sync_vault_handler(
    State(state): State<AppState>,
    Path(raw_id): Path<String>,
) -> Response {
    match parse_vault_id(&raw_id) {
        Ok(vault_id) => managed_git_control_handler(state, vault_id, false).await,
        Err(error) => error.respond(StatusCode::BAD_REQUEST),
    }
}

/// `POST /api/v1/vaults/{vault_id}/retry` — same admitted operation as sync,
/// kept as a distinctly named entry point (mirrors
/// `ManagedGitScheduler::retry_now`).
pub async fn retry_vault_handler(
    State(state): State<AppState>,
    Path(raw_id): Path<String>,
) -> Response {
    match parse_vault_id(&raw_id) {
        Ok(vault_id) => managed_git_control_handler(state, vault_id, true).await,
        Err(error) => error.respond(StatusCode::BAD_REQUEST),
    }
}

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
/// value) still learns the current revision and should refetch broadly,
/// mirroring the existing single-Vault `/api/vault-events` stream's
/// lag-resync behavior.
pub async fn vault_collection_events_handler(
    State(state): State<AppState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let stream = WatchStream::new(state.vaults.subscribe_revisions())
        .map(|event| Ok(collection_revision_event(&event)));
    Sse::new(stream).keep_alive(KeepAlive::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unreconciled_snapshot_reports_a_retryable_status_error() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("vault");
        std::fs::create_dir_all(&path).expect("vault dir");
        let store = crate::vault_registry::VaultRegistryStore::new(
            directory.path().join("state/vaults.json"),
        );
        let snapshot = store
            .add(
                0,
                NewVaultDefinition {
                    name: "Test".to_string(),
                    enabled: true,
                    source: VaultSource::Local { path },
                    exclude_patterns: Vec::new(),
                    https_credentials: None,
                },
            )
            .expect("add vault");
        let definition = snapshot.definitions().next().expect("one definition");
        let fallback = unreconciled_snapshot(&definition);
        assert_eq!(fallback.activation, VaultActivationStatus::Unavailable);
        assert!(
            fallback
                .activation_error
                .as_ref()
                .is_some_and(|error| error.retryable)
        );
    }

    #[test]
    fn https_credentials_patch_keep_round_trips_to_registry_update() {
        let update: crate::vault_registry::HttpsCredentialUpdate =
            HttpsCredentialsPatch::Keep.into();
        assert!(matches!(
            update,
            crate::vault_registry::HttpsCredentialUpdate::Keep
        ));
    }
}
