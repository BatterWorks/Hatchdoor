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
use tracing::error;

use crate::app_state::AppState;
use crate::vault_registry::{
    HttpsCredentials, NewVaultDefinition, VaultCommitIdentity, VaultDefinition,
    VaultDefinitionEdit, VaultDefinitionError, VaultId, VaultRegistryError, VaultRegistryRecovery,
    VaultRegistryRecoveryKind, VaultRegistrySnapshot, VaultRegistryState, VaultSource,
};
use crate::vault_runtime::{
    CollectionVaultSnapshot, LocalContentStatus, VaultActivationStatus, VaultCapabilities,
    VaultChangeCategory, VaultCollectionRevisionEvent, VaultGitStatus, VaultRuntimeError,
    VaultSearchStatus, VaultWatcherStatus,
};
use crate::vault_work::{ScheduleResult, VaultWorkKind};

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct VaultSummary {
    pub vault_id: VaultId,
    pub name: String,
    pub enabled: bool,
    /// Absent on a public read-only demo (#109): the source names an absolute
    /// path on the operator's disk, or the remote a Vault tracks, and a demo
    /// visitor is not an operator. Always present on an authenticated read,
    /// where it is the Settings page's own input.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<VaultSource>,
    pub exclude_patterns: Vec<String>,
    pub credential_configured: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archive_folder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_identity: Option<VaultCommitIdentity>,
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
pub struct LegacyMigrationRecoveryInfo {
    pub code: &'static str,
    pub message: String,
}

impl From<&crate::vault_migration::LegacyMigrationRecovery> for LegacyMigrationRecoveryInfo {
    fn from(recovery: &crate::vault_migration::LegacyMigrationRecovery) -> Self {
        Self {
            code: recovery.code(),
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
    /// Present only when the registry itself is fine (empty, revision 0) but
    /// safe automatic legacy import could not prove the deployment and needs
    /// operator recovery (#150). Distinct from `recovery` above: that one
    /// means the persisted registry file itself is unreadable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legacy_migration_recovery: Option<LegacyMigrationRecoveryInfo>,
    /// Instance-wide publication posture: `true` only under
    /// `HATCHDOOR_DEMO_MODE`. Always serialized (never omitted) so the
    /// browser can tell "not a demo" from "did not say".
    pub demo_mode: bool,
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

/// `username` is optional: a caller may supply a token alone, and the
/// registry substitutes a documented fixed placeholder
/// (`vault_registry::HTTPS_CREDENTIALS_USERNAME_PLACEHOLDER`) rather than
/// requiring one (#130).
#[derive(Debug, Deserialize)]
pub struct HttpsCredentialsInput {
    #[serde(default)]
    pub username: Option<String>,
    pub token: String,
}

impl From<HttpsCredentialsInput> for HttpsCredentials {
    fn from(value: HttpsCredentialsInput) -> Self {
        Self {
            username: value.username.unwrap_or_default(),
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
    Replace {
        #[serde(default)]
        username: Option<String>,
        token: String,
    },
}

impl From<HttpsCredentialsPatch> for crate::vault_registry::HttpsCredentialUpdate {
    fn from(value: HttpsCredentialsPatch) -> Self {
        match value {
            HttpsCredentialsPatch::Keep => Self::Keep,
            HttpsCredentialsPatch::Remove => Self::Remove,
            HttpsCredentialsPatch::Replace { username, token } => Self::Replace(HttpsCredentials {
                username: username.unwrap_or_default(),
                token,
            }),
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
    /// Absent means the server-wide `HATCHDOOR_ARCHIVE_PREFIX` applies.
    #[serde(default)]
    pub archive_folder: Option<String>,
    /// Absent means the server-wide author identity applies.
    #[serde(default)]
    pub commit_identity: Option<VaultCommitIdentity>,
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
    /// Absent means the server-wide `HATCHDOOR_ARCHIVE_PREFIX` applies.
    #[serde(default)]
    pub archive_folder: Option<String>,
    /// Absent means the server-wide author identity applies.
    #[serde(default)]
    pub commit_identity: Option<VaultCommitIdentity>,
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
        | VaultDefinitionError::InvalidArchiveFolder
        | VaultDefinitionError::InvalidCommitIdentity
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
            detail: None,
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
        source: Some(definition.source().clone()),
        exclude_patterns: definition.exclude_patterns().to_vec(),
        credential_configured: definition.credential_configured(),
        archive_folder: definition.archive_folder().map(str::to_string),
        commit_identity: definition.commit_identity().cloned(),
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

/// The public-safe projection of one enabled Vault for a read-only demo (#109).
///
/// Keeps everything a visitor browses with — identity, name, and the four
/// independent status fields plus capabilities, so partial/stale/unavailable
/// participation stays honest (#109's fourth criterion, and #116's slot
/// vocabulary) — and withholds everything that describes the operator's
/// deployment rather than the content: the source's absolute path or remote,
/// the exclusion list, the archive folder, the commit author's name and email,
/// and the runtime error details, whose messages embed absolute paths. The
/// browser already falls back to its own sentence for every absent error, which
/// is what #124 asks a demo to show anyway.
///
/// `credential_configured` survives deliberately: #133 designates it the only
/// credential signal, and it names no path, URL, or secret.
fn public_vault_summary(
    definition: &VaultDefinition,
    snapshot: &CollectionVaultSnapshot,
) -> VaultSummary {
    VaultSummary {
        vault_id: snapshot.vault_id,
        name: snapshot.name.clone(),
        enabled: snapshot.enabled,
        source: None,
        exclude_patterns: Vec::new(),
        credential_configured: definition.credential_configured(),
        archive_folder: None,
        commit_identity: None,
        activation: snapshot.activation,
        local_content: snapshot.local_content,
        search: snapshot.search,
        git: snapshot.git,
        watcher: snapshot.watcher,
        capabilities: snapshot.capabilities,
        activation_error: None,
        search_error: None,
        git_error: None,
        watcher_error: None,
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

async fn reconcile_after_commit(
    state: &AppState,
    snapshot: &VaultRegistrySnapshot,
) -> Result<(), String> {
    let vaults = state.vaults.clone();
    let registry = state.vault_registry.clone();
    let vault_work = state.vault_work.clone();
    let managed_git = state.managed_git.clone();
    let snapshot = snapshot.clone();
    let (mutation_boundary, mutation_safe) = tokio::sync::oneshot::channel();
    let _reconciled = tokio::spawn(async move {
        vaults
            .reconcile_and_reconstruct_and_wait_for_mutation_boundary(
                &registry,
                &snapshot,
                &vault_work,
                &managed_git,
                mutation_boundary,
            )
            .await;
    });
    mutation_safe
        .await
        .map_err(|_| "Vault reconciliation task ended before its mutation boundary".to_string())?
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

/// `GET /api/v1/vaults` — discovery. Reachable at zero enabled Vaults and while
/// the registry is in an explicit recovery state; never returns credentials,
/// only `credential_configured`.
///
/// Reachable unauthenticated in demo mode (#109), which is why the projection
/// forks here: a demo publishes only its *enabled* Vaults, through
/// `public_vault_summary`. A disabled definition is an operator's own bookkeeping
/// about an instance a visitor cannot administer, and listing it would name a
/// Vault the demo does not serve.
#[derive(Debug, Deserialize)]
pub struct StartWithNoVaultsRequest {
    /// The one-shot confirmation flag: a bare POST is not enough (#150),
    /// mirroring `vault_migration::start_with_no_vaults`'s own
    /// `confirmed` gate rather than duplicating it here.
    pub confirm: bool,
}

/// `POST /api/v1/vaults/start-with-no-vaults` — the confirmed recovery action
/// offered only when a failed legacy import left `AppState`'s
/// `legacy_migration_recovery` set (#150). Writes an ordinary empty,
/// revision-1 registry and clears that recovery flag; unreachable (and left
/// untouched) once the registry already holds real state, since
/// `start_with_no_vaults` always commits from revision 0.
pub async fn start_with_no_vaults_handler(
    State(state): State<AppState>,
    request: Result<Json<StartWithNoVaultsRequest>, JsonRejection>,
) -> Response {
    let request = match request {
        Ok(Json(request)) => request,
        Err(error) => return json_rejection_response(error),
    };

    let recovery = state
        .legacy_migration_recovery
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let Some(recovery) = recovery else {
        return VaultApiError::new(
            "legacy_migration_recovery_not_pending",
            "There is no failed legacy import waiting for recovery.",
            None,
            false,
        )
        .respond(StatusCode::CONFLICT);
    };
    if !recovery.can_start_with_no_vaults() {
        return VaultApiError::new(
            "legacy_environment_cleanup_required",
            recovery.message(),
            None,
            false,
        )
        .respond(StatusCode::SERVICE_UNAVAILABLE);
    }

    match crate::vault_migration::start_with_no_vaults(&state.vault_registry, request.confirm) {
        Ok(snapshot) => {
            // Reconciles like every other registry-mutating handler here,
            // even though the transition is empty-registry to
            // empty-registry today: it keeps `state.vaults`'s own
            // collection revision (and its `/api/v1/vaults/events` SSE
            // publication) from silently lagging the registry commit,
            // matching what a future consumer of this collection revision
            // expects.
            if let Err(error) = reconcile_after_commit(&state, &snapshot).await {
                return internal_error_response(error, None);
            }
            *state
                .legacy_migration_recovery
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
            mutation_response(&state, &snapshot, None)
        }
        Err(crate::vault_migration::LegacyMigrationError::ConfirmationRequired) => {
            VaultApiError::new(
                "confirmation_required",
                "Starting with no Vaults requires confirm: true.",
                None,
                false,
            )
            .respond(StatusCode::BAD_REQUEST)
        }
        Err(crate::vault_migration::LegacyMigrationError::Registry(error)) => {
            registry_error_response(error, None)
        }
        Err(crate::vault_migration::LegacyMigrationError::Storage(detail)) => {
            internal_error_response(detail, None)
        }
    }
}

pub async fn list_vaults_handler(State(state): State<AppState>) -> Response {
    match state.vault_registry.load() {
        Ok(VaultRegistryState::Ready(registry_snapshot)) => {
            let collection_snapshot = state.vaults.snapshot();
            let demo_mode = state.demo_mode;
            let legacy_migration_recovery = state
                .legacy_migration_recovery
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_ref()
                .map(LegacyMigrationRecoveryInfo::from);
            let vaults = if legacy_migration_recovery.is_some() {
                Vec::new()
            } else {
                registry_snapshot
                    .definitions()
                    .filter(|definition| !demo_mode || definition.enabled())
                    .map(|definition| {
                        let runtime_snapshot = collection_snapshot
                            .vaults
                            .get(&definition.vault_id())
                            .cloned()
                            .unwrap_or_else(|| unreconciled_snapshot(&definition));
                        if demo_mode {
                            public_vault_summary(&definition, &runtime_snapshot)
                        } else {
                            vault_summary(&definition, &runtime_snapshot)
                        }
                    })
                    .collect()
            };
            Json(VaultDiscoveryResponse {
                registry_revision: Some(registry_snapshot.revision()),
                collection_revision: collection_snapshot.collection_revision,
                vaults,
                recovery: None,
                legacy_migration_recovery,
                demo_mode: state.demo_mode,
            })
            .into_response()
        }
        Ok(VaultRegistryState::Recovery(recovery)) => Json(VaultDiscoveryResponse {
            registry_revision: None,
            collection_revision: 0,
            vaults: Vec::new(),
            recovery: Some(RegistryRecoveryInfo::from(&recovery)),
            legacy_migration_recovery: None,
            demo_mode: state.demo_mode,
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
        archive_folder: request.archive_folder,
        commit_identity: request.commit_identity,
    };
    match state
        .vault_registry
        .add(request.expected_registry_revision, definition)
    {
        Ok(snapshot) => {
            let vault_id = snapshot.vault_ids().find(|id| !before_ids.contains(id));
            if let Err(error) = reconcile_after_commit(&state, &snapshot).await {
                return internal_error_response(error, vault_id);
            }
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

    // Captured before `request.https_credentials` moves into `edit` below.
    // `VaultDefinition`'s `PartialEq` only ever compares `credential_configured`
    // (a bool), never the redacted credential value itself — by design, so
    // retention/reconciliation never needs to see a plaintext secret. That
    // means replacing token A with token B leaves `credential_configured`
    // `true` before and after, so nothing downstream of the registry commit
    // can observe "credentials actually changed" from the definition alone;
    // this flag is the one place that genuinely knows a new value was
    // written this request (issue #97's reopening finding 3; also relied on
    // below for issue #98's reopening finding, since the same blind spot
    // means `reconcile()` never emits a collection-revision event for a
    // credential-only change either).
    let credentials_replaced = matches!(
        request.https_credentials,
        HttpsCredentialsPatch::Replace { .. }
    );

    let edit = VaultDefinitionEdit {
        name: request.name,
        source: request.source,
        exclude_patterns: request.exclude_patterns,
        https_credentials: request.https_credentials.into(),
        confirm_identity_change: request.confirm_identity_change,
        archive_folder: request.archive_folder,
        commit_identity: request.commit_identity,
    };
    match state
        .vault_registry
        .edit(request.expected_registry_revision, vault_id, edit)
    {
        Ok(snapshot) => {
            if let Err(error) = reconcile_after_commit(&state, &snapshot).await {
                return internal_error_response(error, Some(vault_id));
            }
            if credentials_replaced {
                request_git_retry_after_credential_replacement(&state, &snapshot, vault_id);
                // Issue #98's reopening finding: `reconcile()` retained the
                // same `VaultControlBlock` above (definition equality can't
                // see a credential-only change), so it never bumped
                // `collection_revision` or emitted an event for this Vault.
                // Notify explicitly so SSE consumers can still invalidate
                // its Git/capability state after a real secret rotation.
                state.vaults.notify_definition_changed(vault_id);
            }
            mutation_response(&state, &snapshot, Some(vault_id))
        }
        Err(error) => registry_error_response(error, Some(vault_id)),
    }
}

/// Request an immediate Git turn for `vault_id` after its edit just wrote new
/// HTTPS credentials (issue #97's reopening finding 3): a prior
/// authentication failure otherwise sits unretried until the normal
/// schedule, since `VaultDefinition` equality — which `reconcile()` uses to
/// decide whether to retain the existing `VaultControlBlock` and skip its
/// Pending-status Git-turn request — cannot see the credential value change,
/// only whether a credential is configured at all (`true` both before and
/// after a Replace-over-Replace).
///
/// `edit_vault_handler` (including its MCP `edit_vault` proxy, which calls
/// this same handler) is the only call site of
/// `VaultRegistryStore::edit`/`VaultDefinitionEdit`, so this handler-level
/// trigger, run once per successful edit after `reconcile_after_commit`
/// completes, covers every path that can write `https_credentials`.
///
/// Source-agnostic: both `ManagedGit` and a remote-backed `ExistingGit`
/// (`PullOnly`/`TwoWay`) Vault are tracked by `ManagedGitScheduler` and
/// carry a `managed_git_poll_interval`, so both retry through
/// `state.managed_git.retry_now`, which self-registers the Vault if it was
/// not already tracked. `Local` and an `ExistingGit` `LocalHistory` Vault
/// carry no interval and are skipped — `Local` never accepts credentials
/// (rejected earlier at commit time) and so never reaches here with
/// `credentials_replaced` true, and `LocalHistory` has no remote to retry.
fn request_git_retry_after_credential_replacement(
    state: &AppState,
    snapshot: &VaultRegistrySnapshot,
    vault_id: VaultId,
) {
    let Some(definition) = snapshot.definition(vault_id) else {
        return;
    };
    if let Some(poll_interval) = definition.source().managed_git_poll_interval() {
        state.managed_git.retry_now(vault_id, poll_interval);
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
            if let Err(error) = reconcile_after_commit(&state, &snapshot).await {
                return internal_error_response(error, Some(vault_id));
            }
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
            if let Err(error) = reconcile_after_commit(&state, &snapshot).await {
                return internal_error_response(error, Some(vault_id));
            }
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
    let Some(poll_interval) = definition.source().managed_git_poll_interval() else {
        return VaultApiError::new(
            "capability_unavailable",
            "Manual Git sync is only available for a Vault with a configured remote",
            Some(vault_id),
            false,
        )
        .respond(StatusCode::CONFLICT);
    };

    let schedule = if retry {
        state.managed_git.retry_now(vault_id, poll_interval)
    } else {
        state.managed_git.sync_now(vault_id, poll_interval)
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

/// `POST /api/v1/vaults/{vault_id}/refresh` — request one Vault's next Index
/// turn. The handler only admits work to the shared FIFO; the runtime worker
/// performs the authoritative Markdown scan and atomic snapshot publication.
pub async fn refresh_vault_handler(
    State(state): State<AppState>,
    Path(raw_id): Path<String>,
) -> Response {
    let vault_id = match parse_vault_id(&raw_id) {
        Ok(vault_id) => vault_id,
        Err(error) => return error.respond(StatusCode::BAD_REQUEST),
    };
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
    if state
        .vaults
        .runtime(vault_id)
        .is_none_or(|runtime| !runtime.snapshot().capabilities.browse)
    {
        return VaultApiError::new(
            "capability_unavailable",
            "Vault refresh requires currently usable local Markdown content",
            Some(vault_id),
            true,
        )
        .respond(StatusCode::CONFLICT);
    }

    match state.vault_work.request(vault_id, VaultWorkKind::Index) {
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
    use crate::cache::SqliteCache;
    use crate::vault_registry::{DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS, VaultGitMode};
    use crate::vault_work::VaultWorkError;

    /// A minimal `AppState` for exercising `handlers/vaults.rs` request
    /// handlers directly, without the legacy single-Vault cache/embedder
    /// machinery those handlers never touch (mirrors `mcp/routes.rs`'s own
    /// `test_state` shape). `startup_sqlite` still needs a real (in-memory,
    /// so cheap) `SqliteCache` because the field is not optional;
    /// `ready_vault` genuinely can be `None` since nothing under test reads
    /// it. Returns the coordinator's worker too — discarded by most callers,
    /// but a test that needs to drain a queued turn (e.g. the one Vault
    /// activation itself requests) needs direct access to it, since nothing
    /// else in this process consumes the coordinator's queue. Also returns
    /// the backing `TempDir` so it outlives the state.
    fn test_state() -> (
        AppState,
        crate::vault_work::VaultWorkWorker,
        tempfile::TempDir,
    ) {
        let directory = tempfile::tempdir().expect("temp dir");
        let (vault_events, _) = tokio::sync::broadcast::channel(64);
        let (mcp_tools_changed, _) = tokio::sync::broadcast::channel(16);
        let (vault_work, worker) = crate::vault_work::VaultWorkCoordinator::new();
        let managed_git =
            std::sync::Arc::new(crate::git::ManagedGitScheduler::new(vault_work.clone()));
        let state = AppState {
            cache_db_path: directory.path().join("cache.sqlite3"),
            vault_registry: crate::vault_registry::VaultRegistryStore::new(
                directory.path().join("state/vaults.json"),
            ),
            vaults: crate::vault_runtime::VaultCollectionRuntime::new(),
            vault_work,
            managed_git,
            legacy_migration_recovery: std::sync::Arc::new(std::sync::RwLock::new(None)),
            startup_sqlite: std::sync::Arc::new(
                SqliteCache::in_memory(384).expect("in-memory cache"),
            ),
            ready_vault: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
            vault_revision: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            vault_events,
            mcp_tools_changed,
            embedder: crate::app_state::test_embedder(),
            runtime_embedder: std::sync::Arc::new(crate::embed::RuntimeEmbedder::new()),
            model_setup: std::sync::Arc::new(crate::model_setup::ModelSetup::new(
                directory.path().join("models"),
            )),
            model_setup_started: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
            startup_git_config: std::sync::Arc::new(None),
            web_auth_enabled: false,
            demo_mode: false,
            vault_write_lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
            git_sync: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
            scan_config_cache: std::sync::Arc::new(std::sync::RwLock::new(None)),
            refresh_lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
            index_status: crate::app_state::IndexStatusTracker::up_to_date(),
            runtime_config: crate::runtime_config::RuntimeConfig::for_tests(),
            startup: crate::startup::StartupTracker::ready(),
        };
        (state, worker, directory)
    }

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
                    archive_folder: None,
                    commit_identity: None,
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

    /// Closes issue #97's reopening finding 3: replacing a Vault's HTTPS
    /// credentials while it sits in an authentication-failure Git status
    /// used to leave it unretried until the normal (non-backoff, "wait for
    /// a configuration change, a manual retry, or a restart") schedule,
    /// because `VaultDefinition`'s `PartialEq` only ever sees
    /// `credential_configured: bool` — `true` both before and after a
    /// Replace-over-Replace — never the actual credential value, so
    /// `reconcile()` retained the same `VaultControlBlock` and never
    /// reached the Git-turn-request logic gated behind a *new* control
    /// block's `Pending` status.
    #[tokio::test]
    async fn credential_replacement_requests_an_immediate_retry_after_an_authentication_failure() {
        let (state, mut worker, _directory) = test_state();

        let create_request = CreateVaultRequest {
            expected_registry_revision: 0,
            name: "Remote notes".to_string(),
            enabled: true,
            source: VaultSource::ManagedGit {
                repository_url: "https://example.test/owner/notes.git".to_string(),
                branch: None,
                vault_subdirectory: None,
                mode: VaultGitMode::PullOnly,
                poll_interval_secs: DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
            },
            exclude_patterns: Vec::new(),
            https_credentials: Some(HttpsCredentialsInput {
                username: Some("git-user".to_string()),
                token: "old-token".to_string(),
            }),
            archive_folder: None,
            commit_identity: None,
        };
        let response = create_vault_handler(State(state.clone()), Ok(Json(create_request))).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let VaultRegistryState::Ready(snapshot) =
            state.vault_registry.load().expect("load registry")
        else {
            panic!("registry entered recovery");
        };
        let vault_id = snapshot.vault_ids().next().expect("one Vault");

        // Drain the one Git turn Vault activation itself requests (the fresh
        // Vault starts `Pending`) and record it as an authentication
        // failure — exactly what a real turn's outcome publication does,
        // without needing a live network call. This leaves the Vault
        // quiescent (no automatic retry pending) in an authentication-
        // failure Git status, matching the finding's precondition.
        let auth_failure = VaultWorkError::new(
            "managed_git_authentication_failed",
            "bad credentials",
            false,
        );
        let outcome = worker
            .run_next(|_| {
                let auth_failure = auth_failure.clone();
                async move { Err::<(), _>(auth_failure) }
            })
            .await
            .expect("Vault activation's initial Git turn");
        assert_eq!(outcome.request.vault_id(), vault_id);
        state
            .managed_git
            .record_outcome(vault_id, &Err(auth_failure.clone()));
        state
            .vaults
            .runtime(vault_id)
            .expect("active runtime")
            .set_git_status(
                VaultGitStatus::Unavailable,
                Some(VaultRuntimeError {
                    code: auth_failure.code().to_string(),
                    message: auth_failure.message().to_string(),
                    retryable: auth_failure.retryable(),
                    detail: None,
                }),
            )
            .expect("publish authentication-failure status");
        assert!(
            !state.vault_work.has_work(vault_id, VaultWorkKind::Git),
            "the Vault must be quiescent before the credential replacement under test"
        );

        // Replace credentials. The redacted definition's `credential_configured`
        // stays `true` before and after, so without this fix nothing would
        // notice the value actually changed.
        let VaultRegistryState::Ready(current) =
            state.vault_registry.load().expect("load registry")
        else {
            panic!("registry entered recovery");
        };
        let edit_request = EditVaultRequest {
            expected_registry_revision: current.revision(),
            name: "Remote notes".to_string(),
            source: VaultSource::ManagedGit {
                repository_url: "https://example.test/owner/notes.git".to_string(),
                branch: None,
                vault_subdirectory: None,
                mode: VaultGitMode::PullOnly,
                poll_interval_secs: DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
            },
            exclude_patterns: Vec::new(),
            https_credentials: HttpsCredentialsPatch::Replace {
                username: Some("git-user".to_string()),
                token: "new-token".to_string(),
            },
            confirm_identity_change: false,
            archive_folder: None,
            commit_identity: None,
        };
        let response = edit_vault_handler(
            State(state.clone()),
            Path(vault_id.to_string()),
            Ok(Json(edit_request)),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);

        assert!(
            state.vault_work.has_work(vault_id, VaultWorkKind::Git),
            "replacing credentials on an authentication-failed Vault must request an immediate retry"
        );
    }

    /// Closes issue #98's reopening finding: replacing an already-configured
    /// credential must still publish a `Definition`-category collection-
    /// revision event, even though `VaultDefinition` equality can't observe
    /// the change and `reconcile()` retains the same `VaultControlBlock`
    /// unchanged (the same blind spot #97's reopening finding 3 fixed for
    /// Git retry scheduling — that fix never touched the revision/event
    /// path SSE consumers rely on).
    #[tokio::test]
    async fn credential_replacement_notifies_definition_change_for_sse_consumers() {
        let (state, _worker, _directory) = test_state();

        let create_request = CreateVaultRequest {
            expected_registry_revision: 0,
            name: "Remote notes".to_string(),
            enabled: true,
            source: VaultSource::ManagedGit {
                repository_url: "https://example.test/owner/notes.git".to_string(),
                branch: None,
                vault_subdirectory: None,
                mode: VaultGitMode::PullOnly,
                poll_interval_secs: DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
            },
            exclude_patterns: Vec::new(),
            https_credentials: Some(HttpsCredentialsInput {
                username: Some("git-user".to_string()),
                token: "old-token".to_string(),
            }),
            archive_folder: None,
            commit_identity: None,
        };
        let response = create_vault_handler(State(state.clone()), Ok(Json(create_request))).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let VaultRegistryState::Ready(snapshot) =
            state.vault_registry.load().expect("load registry")
        else {
            panic!("registry entered recovery");
        };
        let vault_id = snapshot.vault_ids().next().expect("one Vault");

        let mut revisions = state.vaults.subscribe_revisions();
        let before = revisions.borrow().collection_revision;

        let edit_request = EditVaultRequest {
            expected_registry_revision: snapshot.revision(),
            name: "Remote notes".to_string(),
            source: VaultSource::ManagedGit {
                repository_url: "https://example.test/owner/notes.git".to_string(),
                branch: None,
                vault_subdirectory: None,
                mode: VaultGitMode::PullOnly,
                poll_interval_secs: DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
            },
            exclude_patterns: Vec::new(),
            https_credentials: HttpsCredentialsPatch::Replace {
                username: Some("git-user".to_string()),
                token: "new-token".to_string(),
            },
            confirm_identity_change: false,
            archive_folder: None,
            commit_identity: None,
        };
        let response = edit_vault_handler(
            State(state.clone()),
            Path(vault_id.to_string()),
            Ok(Json(edit_request)),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);

        assert!(
            revisions.has_changed().expect("revisions channel open"),
            "a credential-only replacement must still publish a collection-revision event"
        );
        let event = revisions.borrow_and_update().clone();
        assert!(event.collection_revision > before);
        assert_eq!(event.vault_ids, vec![vault_id]);
        assert_eq!(event.category, VaultChangeCategory::Definition);
    }

    /// Closes a Spec-review finding on issue #97's reopening finding 2:
    /// `vault_runtime.rs`'s reconcile loop is the *only* path that pushes an
    /// edited `poll_interval_secs` into the live `ManagedGitScheduler` — a
    /// non-retained (definition-changed) `VaultControlBlock` calls
    /// `managed_git.activate(vault_id, poll_interval)` regardless of Git
    /// status. `managed_task.rs`'s own unit tests (e.g.
    /// `two_vaults_with_different_poll_intervals_get_independent_schedules`,
    /// `reactivating_with_a_changed_interval_updates_it_without_resetting_an_armed_backoff`)
    /// exercise `ManagedGitScheduler::activate` directly, in isolation —
    /// this test instead drives the real `edit_vault_handler` request path
    /// (registry `edit()` + `reconcile_and_reconstruct`) end to end and
    /// observes the *running* scheduler's stored per-Vault state through
    /// its `#[cfg(test)]` accessors, proving: (1) an interval-only edit on
    /// an already-active managed-Git Vault reaches the live scheduler with
    /// no restart or manual reactivation, and (2) doing so leaves an
    /// in-progress backoff untouched — interval update and backoff
    /// preservation are two independent concerns, and this is where that
    /// claim is actually load-bearing (the HTTP edit path), not just at the
    /// scheduler-unit level.
    ///
    /// Also closes a second, independent Spec-review finding on the same
    /// edit path: publishing the Vault's Git status to `Unavailable` (a
    /// real retryable failure) before the edit, and asserting
    /// `VaultWorkCoordinator::has_work` stays `false` afterward, proves the
    /// edit does not *also* force an unwanted immediate real Git turn —
    /// `activation_snapshot` used to reset every non-retained
    /// reconstruction's Git status back to `Pending` unconditionally,
    /// which the active loop treats as "needs an immediate first sync" and
    /// requests a real turn for, silently bypassing whatever backoff
    /// finding 1 had armed. See
    /// `vault_runtime::tests::editing_a_non_identity_field_preserves_the_vaults_actual_prior_git_status`
    /// for the same property proven directly at the `reconcile()` level.
    #[tokio::test]
    async fn editing_only_the_poll_interval_updates_the_live_scheduler_without_disturbing_backoff()
    {
        let (state, mut worker, _directory) = test_state();

        let create_request = CreateVaultRequest {
            expected_registry_revision: 0,
            name: "Remote notes".to_string(),
            enabled: true,
            source: VaultSource::ManagedGit {
                repository_url: "https://example.test/owner/notes.git".to_string(),
                branch: None,
                vault_subdirectory: None,
                mode: VaultGitMode::PullOnly,
                poll_interval_secs: DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
            },
            exclude_patterns: Vec::new(),
            https_credentials: None,
            archive_folder: None,
            commit_identity: None,
        };
        let response = create_vault_handler(State(state.clone()), Ok(Json(create_request))).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let VaultRegistryState::Ready(snapshot) =
            state.vault_registry.load().expect("load registry")
        else {
            panic!("registry entered recovery");
        };
        let vault_id = snapshot.vault_ids().next().expect("one Vault");

        // Arm an in-progress backoff by draining the initial creation-
        // triggered Git turn as a retryable failure — the same technique
        // the credential-replacement test above uses — so the interval-only
        // edit below has something to prove it does *not* disturb. Also
        // publish the Vault's actual Git status as `Unavailable` (exactly
        // what a real turn's outcome-publication does, alongside
        // `record_outcome`), so the edit below has a genuinely non-`Pending`
        // status to prove it does not silently discard.
        let transient = VaultWorkError::new("managed_git_remote_unreachable", "x", true);
        let outcome = worker
            .run_next(|_| {
                let transient = transient.clone();
                async move { Err::<(), _>(transient) }
            })
            .await
            .expect("Vault activation's initial Git turn");
        assert_eq!(outcome.request.vault_id(), vault_id);
        state
            .managed_git
            .record_outcome(vault_id, &Err(transient.clone()));
        state
            .vaults
            .runtime(vault_id)
            .expect("active runtime")
            .set_git_status(
                VaultGitStatus::Unavailable,
                Some(VaultRuntimeError {
                    code: transient.code().to_string(),
                    message: transient.message().to_string(),
                    retryable: transient.retryable(),
                    detail: None,
                }),
            )
            .expect("publish the real transient Git failure");
        let armed_next_attempt = state
            .managed_git
            .next_attempt_for_test(vault_id)
            .expect("Vault tracked by the scheduler after its first turn");
        assert_eq!(
            state.managed_git.poll_interval_for_test(vault_id),
            Some(std::time::Duration::from_secs(
                DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS
            )),
            "the scheduler must start out tracking the Vault's original interval"
        );
        assert!(
            !state.vault_work.has_work(vault_id, VaultWorkKind::Git),
            "the Vault must be quiescent (mid-backoff, no queued turn) before the edit under test"
        );

        // Edit only the poll interval: same source identity, same
        // credentials, no confirmation needed.
        let VaultRegistryState::Ready(current) =
            state.vault_registry.load().expect("load registry")
        else {
            panic!("registry entered recovery");
        };
        let new_interval_secs = DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS * 2;
        let edit_request = EditVaultRequest {
            expected_registry_revision: current.revision(),
            name: "Remote notes".to_string(),
            source: VaultSource::ManagedGit {
                repository_url: "https://example.test/owner/notes.git".to_string(),
                branch: None,
                vault_subdirectory: None,
                mode: VaultGitMode::PullOnly,
                poll_interval_secs: new_interval_secs,
            },
            exclude_patterns: Vec::new(),
            https_credentials: HttpsCredentialsPatch::Keep,
            confirm_identity_change: false,
            archive_folder: None,
            commit_identity: None,
        };
        let response = edit_vault_handler(
            State(state.clone()),
            Path(vault_id.to_string()),
            Ok(Json(edit_request)),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);

        // The edit itself took effect: the reloaded definition carries the
        // new interval.
        let VaultRegistryState::Ready(after_edit) =
            state.vault_registry.load().expect("load registry")
        else {
            panic!("registry entered recovery");
        };
        assert_eq!(
            after_edit
                .definition(vault_id)
                .and_then(|definition| definition.source().managed_git_poll_interval()),
            Some(std::time::Duration::from_secs(new_interval_secs)),
            "the edit must have actually persisted the new interval"
        );

        // The live scheduler's stored interval reflects the edit — reached
        // purely through reconcile()'s non-retained-definition path, no
        // restart or manual reactivation.
        assert_eq!(
            state.managed_git.poll_interval_for_test(vault_id),
            Some(std::time::Duration::from_secs(new_interval_secs)),
            "an interval-only edit must reach the live ManagedGitScheduler"
        );
        // And the in-progress backoff armed above is untouched.
        assert_eq!(
            state.managed_git.next_attempt_for_test(vault_id),
            Some(armed_next_attempt),
            "an interval-only edit must not reset an in-progress backoff"
        );
        // And, the specific coordinator-level effect a forced `Pending`
        // reset would have caused: no immediate real Git turn was queued.
        assert!(
            !state.vault_work.has_work(vault_id, VaultWorkKind::Git),
            "a benign interval-only edit must not force an immediate real Git turn \
             while the Vault is mid-backoff from a real transient failure"
        );
    }

    /// Issue #132: `POST /sync` and `POST /retry` used to refuse every
    /// `ExistingGit` Vault outright — `managed_git_control_handler` checked
    /// `managed_git_poll_interval()`, which returned `None` for every
    /// `ExistingGit` mode. A remote-backed `ExistingGit` Vault (`PullOnly`/
    /// `TwoWay`) is now scheduler-tracked exactly like `ManagedGit`, so both
    /// endpoints must admit it — this is the acceptance criterion
    /// "`An ExistingGit Vault in PullOnly or TwoWay ... accepts POST /sync
    /// and POST /retry`" exercised end to end through the real handlers,
    /// mirroring `vaults_v1_sync_and_retry_require_a_managed_git_source`'s
    /// coverage of the still-refused `Local` case (`src/server.rs`) but for
    /// the newly-admitted source kind.
    #[tokio::test]
    async fn sync_and_retry_admit_an_existing_git_pull_only_vault_and_track_its_schedule() {
        let (state, _worker, directory) = test_state();
        let repository_path = directory.path().join("existing-pull-only-repo");
        std::fs::create_dir_all(&repository_path).expect("create repo directory");
        git2::Repository::init(&repository_path).expect("init git repo");

        let create_request = CreateVaultRequest {
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
        };
        let response = create_vault_handler(State(state.clone()), Ok(Json(create_request))).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let VaultRegistryState::Ready(snapshot) =
            state.vault_registry.load().expect("load registry")
        else {
            panic!("registry entered recovery");
        };
        let vault_id = snapshot.vault_ids().next().expect("one Vault");

        // Activation alone must already have registered the schedule —
        // `reconcile_and_reconstruct`'s activation loop calls
        // `managed_git.activate` for any source with a
        // `managed_git_poll_interval`, generically across source kinds.
        assert_eq!(
            state.managed_git.poll_interval_for_test(vault_id),
            Some(std::time::Duration::from_secs(
                DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS
            )),
            "an ExistingGit PullOnly Vault must be tracked by the scheduler on activation"
        );

        let sync = sync_vault_handler(State(state.clone()), Path(vault_id.to_string())).await;
        assert_eq!(
            sync.status(),
            StatusCode::ACCEPTED,
            "manual sync must no longer be refused for an ExistingGit PullOnly Vault"
        );

        let retry = retry_vault_handler(State(state.clone()), Path(vault_id.to_string())).await;
        assert_eq!(
            retry.status(),
            StatusCode::ACCEPTED,
            "manual retry must no longer be refused for an ExistingGit PullOnly Vault"
        );
    }

    /// Companion to `sync_and_retry_admit_an_existing_git_pull_only_vault_and_track_its_schedule`:
    /// the other half of the acceptance criterion "An `ExistingGit` Vault in
    /// `LocalHistory` ... still refuse[s] both and carr[ies] no schedule" —
    /// `LocalHistory` has no remote to poll and must keep refusing both
    /// endpoints exactly as before this ticket.
    #[tokio::test]
    async fn sync_and_retry_still_refuse_an_existing_git_local_history_vault() {
        let (state, _worker, directory) = test_state();
        let repository_path = directory.path().join("local-history-repo");
        std::fs::create_dir_all(&repository_path).expect("create repo directory");
        git2::Repository::init(&repository_path).expect("init git repo");

        let create_request = CreateVaultRequest {
            expected_registry_revision: 0,
            name: "Local history checkout".to_string(),
            enabled: true,
            source: VaultSource::ExistingGit {
                repository_path,
                repository_url: None,
                branch: None,
                vault_subdirectory: None,
                mode: VaultGitMode::LocalHistory,
                poll_interval_secs: DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
            },
            exclude_patterns: Vec::new(),
            https_credentials: None,
            archive_folder: None,
            commit_identity: None,
        };
        let response = create_vault_handler(State(state.clone()), Ok(Json(create_request))).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let VaultRegistryState::Ready(snapshot) =
            state.vault_registry.load().expect("load registry")
        else {
            panic!("registry entered recovery");
        };
        let vault_id = snapshot.vault_ids().next().expect("one Vault");

        assert_eq!(
            state.managed_git.poll_interval_for_test(vault_id),
            None,
            "a LocalHistory Vault must never be tracked by the scheduler"
        );

        let sync = sync_vault_handler(State(state.clone()), Path(vault_id.to_string())).await;
        assert_eq!(sync.status(), StatusCode::CONFLICT);

        let retry = retry_vault_handler(State(state.clone()), Path(vault_id.to_string())).await;
        assert_eq!(retry.status(), StatusCode::CONFLICT);
    }
}
