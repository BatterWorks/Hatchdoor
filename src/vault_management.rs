//! Vault collection management: the shared core behind every change to which
//! Vaults this instance serves, and every projection of that collection.
//!
//! Before #187 this orchestration lived in `handlers/vaults.rs`, and the seven
//! MCP management tools reached it by calling those handler functions with
//! hand-built axum extractors and decoding the HTTP response body. The
//! sequence every Vault definition change runs — commit to the registry,
//! reconcile the live runtime through its foreground mutation boundary, then
//! answer from one collection snapshot — is domain behaviour, not transport
//! shaping, so it belongs here alongside the authenticated and demo
//! projections of a Vault, the credential-replacement Git retry rule, the
//! manual sync/retry/refresh controls, the confirmed start-with-no-Vaults
//! recovery, and the collection wire types.
//!
//! Failures leave here as the transport-neutral [`VaultOperationError`] from
//! #184, exactly as the Vault read and mutation cores report theirs (ADR-19).
//! Each adapter maps that error onto its own wire shape: `handlers/vaults.rs`
//! onto a status code plus the same JSON body, `mcp/tools/read.rs` onto a
//! structured tool error. An instance-side failure is logged and sanitized
//! here rather than in an adapter, so both surfaces report the same message
//! and neither leaks a filesystem path.
//!
//! What stays with the adapters is what only their transport knows:
//! extractors and rejection wording, status codes, the demo-mode refusal
//! middleware in `src/server.rs`, and the collection-wide SSE event stream,
//! which is a transport concern with no MCP counterpart.

use std::collections::BTreeSet;
use std::str::FromStr;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tracing::error;

use crate::app_state::AppState;
use crate::git::GitPollingClock;
use crate::vault_error::VaultOperationError;
use crate::vault_registry::{
    HttpsCredentials, NewVaultDefinition, VaultCommitIdentity, VaultDefinition,
    VaultDefinitionEdit, VaultDefinitionError, VaultId, VaultRegistryError, VaultRegistryRecovery,
    VaultRegistryRecoveryKind, VaultRegistrySnapshot, VaultRegistryState, VaultSource,
};
use crate::vault_runtime::{
    CollectionVaultSnapshot, LocalContentStatus, VaultActivationStatus, VaultCapabilities,
    VaultCollectionRevisionEvent, VaultCollectionSnapshot, VaultGitStatus, VaultRuntimeError,
    VaultSearchStatus, VaultWatcherStatus,
};
use crate::vault_runtime_state::format_timestamp;
use crate::vault_work::{ScheduleResult, VaultWorkKind};

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, JsonSchema, Deserialize)]
pub struct VaultSummary {
    pub vault_id: VaultId,
    pub name: String,
    pub enabled: bool,
    /// Absent on a public read-only demo (#109): the source names an absolute
    /// path on the operator's disk, or the remote a Vault tracks, and a demo
    /// visitor is not an operator. Always present on an authenticated read,
    /// where it is the Settings page's own input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<VaultSource>,
    pub exclude_patterns: Vec<String>,
    pub credential_configured: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_folder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_identity: Option<VaultCommitIdentity>,
    pub activation: VaultActivationStatus,
    pub local_content: LocalContentStatus,
    pub search: VaultSearchStatus,
    pub git: VaultGitStatus,
    /// When this Vault's last interval-arming Git turn finished, RFC 3339
    /// UTC — whether it succeeded or failed. A failed check is still a check,
    /// and `git`/`git_error` already say which it was; naming this "synced"
    /// would report a successful sync for a Vault that has only ever failed
    /// to authenticate. Absent for a Vault with no remote to poll, one that
    /// has not completed a turn yet, or a read that withholds operator
    /// detail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_checked_at: Option<String>,
    /// When this Vault's next scheduled Git turn is due, RFC 3339 UTC.
    /// Present for every Vault with a remote to poll — one that has never
    /// completed a turn is due immediately, not unscheduled — and absent only
    /// for a Vault with no remote, or a read that withholds operator detail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_attempt_at: Option<String>,
    pub watcher: VaultWatcherStatus,
    pub capabilities: VaultCapabilities,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activation_error: Option<VaultRuntimeError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_error: Option<VaultRuntimeError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_error: Option<VaultRuntimeError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watcher_error: Option<VaultRuntimeError>,
}

#[derive(Debug, Serialize, JsonSchema, Deserialize)]
pub struct RegistryRecoveryInfo {
    pub code: String,
    pub kind: String,
    pub message: String,
}

impl From<&VaultRegistryRecovery> for RegistryRecoveryInfo {
    fn from(recovery: &VaultRegistryRecovery) -> Self {
        let kind = match recovery.kind() {
            VaultRegistryRecoveryKind::Corrupt => "corrupt".to_string(),
            VaultRegistryRecoveryKind::UnsupportedSchema { .. } => "unsupported_schema".to_string(),
            VaultRegistryRecoveryKind::FutureSchema { .. } => "future_schema".to_string(),
        };
        Self {
            code: "vault_registry_recovery_required".to_string(),
            kind,
            message: recovery.message().to_string(),
        }
    }
}

#[derive(Debug, Serialize, JsonSchema, Deserialize)]
pub struct LegacyMigrationRecoveryInfo {
    pub code: String,
    pub message: String,
}

impl From<&crate::vault_migration::LegacyMigrationRecovery> for LegacyMigrationRecoveryInfo {
    fn from(recovery: &crate::vault_migration::LegacyMigrationRecovery) -> Self {
        Self {
            code: recovery.code().to_string(),
            message: recovery.message().to_string(),
        }
    }
}

#[derive(Debug, Serialize, JsonSchema, Deserialize)]
pub struct VaultDiscoveryResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry_revision: Option<u64>,
    pub collection_revision: u64,
    pub vaults: Vec<VaultSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<RegistryRecoveryInfo>,
    /// Present only when the registry itself is fine (empty, revision 0) but
    /// safe automatic legacy import could not prove the deployment and needs
    /// operator recovery (#150). Distinct from `recovery` above: that one
    /// means the persisted registry file itself is unreadable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_migration_recovery: Option<LegacyMigrationRecoveryInfo>,
    /// Instance-wide publication posture: `true` only under
    /// `HATCHDOOR_DEMO_MODE`. Always serialized (never omitted) so the
    /// browser can tell "not a demo" from "did not say".
    pub demo_mode: bool,
}

#[derive(Debug, Serialize, JsonSchema, Deserialize)]
pub struct VaultMutationResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault: Option<VaultSummary>,
    pub registry_revision: u64,
    pub collection_revision: u64,
}

#[derive(Debug, Serialize, JsonSchema, Deserialize)]
pub struct VaultScheduleResponse {
    pub vault_id: VaultId,
    pub schedule: String,
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

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Parse a caller-supplied Vault ID. Both surfaces need it before they can
/// name a Vault, and both report the same `invalid_vault_id` refusal, so it
/// is a core concern rather than an axum path-extractor detail;
/// `handlers/vaults.rs` re-exports it for its sibling adapters.
pub(crate) fn parse_vault_id(raw: &str) -> Result<VaultId, VaultOperationError> {
    VaultId::from_str(raw).map_err(|_| {
        VaultOperationError::new(
            "invalid_vault_id",
            "Vault ID must be a canonical UUID v4",
            None,
            false,
        )
    })
}

/// Every error code this core can report.
///
/// The HTTP adapter turns each of these into a status code, and its
/// `every_management_error_code_keeps_its_historical_status` test asserts its
/// table covers exactly this list — so a code added here without a declared
/// status fails the tests rather than silently becoming a `500` through the
/// table's catch-all arm. The list is maintained by hand alongside the
/// constructors below, and exists only to constrain the adapters, so it is
/// compiled for tests alone.
#[cfg(test)]
pub(crate) const MANAGEMENT_ERROR_CODES: &[&str] = &[
    "invalid_vault_id",
    "invalid_vault_definition",
    "confirmation_required",
    "vault_not_found",
    "duplicate_vault_name",
    "vault_path_overlap",
    "identity_change_requires_disabled",
    "identity_change_requires_confirmation",
    "registry_revision_conflict",
    "legacy_migration_recovery_not_pending",
    "vault_disabled",
    "capability_unavailable",
    "vault_registry_recovery_required",
    "legacy_environment_cleanup_required",
    "vault_unavailable",
    "registry_revision_exhausted",
    "internal_error",
];

/// An instance-side failure: logged here with its real detail, reported to
/// every surface with the same sanitized message. The sanitization lives in
/// the core rather than in one adapter so a filesystem path cannot reach one
/// surface merely because that surface skipped the scrubbing.
fn internal_error(detail: impl AsRef<str>, vault_id: Option<VaultId>) -> VaultOperationError {
    error!(detail = %detail.as_ref(), "Vault collection API internal error");
    VaultOperationError::new("internal_error", "Internal server error", vault_id, false)
}

fn recovery_error(recovery: &VaultRegistryRecovery) -> VaultOperationError {
    VaultOperationError::new(
        "vault_registry_recovery_required",
        recovery.message().to_string(),
        None,
        true,
    )
}

fn definition_error(error: VaultDefinitionError, vault_id: Option<VaultId>) -> VaultOperationError {
    let message = error.to_string();
    let code = match error {
        VaultDefinitionError::VaultNotFound => "vault_not_found",
        // Both depend on other current registry records, not on the shape of
        // this request in isolation, so they are state conflicts rather than
        // malformed input — the same request would succeed against a
        // different existing registry state.
        VaultDefinitionError::DuplicateName => "duplicate_vault_name",
        VaultDefinitionError::PathOverlap => "vault_path_overlap",
        VaultDefinitionError::IdentityChangeRequiresDisabled => "identity_change_requires_disabled",
        VaultDefinitionError::IdentityChangeRequiresConfirmation => {
            "identity_change_requires_confirmation"
        }
        VaultDefinitionError::InvalidName
        | VaultDefinitionError::InvalidExclusionPattern
        | VaultDefinitionError::InvalidArchiveFolder
        | VaultDefinitionError::InvalidCommitIdentity
        | VaultDefinitionError::InvalidSource(_) => "invalid_vault_definition",
    };
    VaultOperationError::new(code, message, vault_id, false)
}

fn registry_error(error: VaultRegistryError, vault_id: Option<VaultId>) -> VaultOperationError {
    match error {
        VaultRegistryError::RevisionConflict { expected, actual } => VaultOperationError::new(
            "registry_revision_conflict",
            format!("expected registry revision {expected}, current revision is {actual}"),
            vault_id,
            true,
        ),
        VaultRegistryError::RecoveryRequired => VaultOperationError::new(
            "vault_registry_recovery_required",
            "The Vault registry requires operator recovery and cannot be written until it is restored.",
            vault_id,
            true,
        ),
        VaultRegistryError::RevisionExhausted => {
            let message = error.to_string();
            VaultOperationError::new("registry_revision_exhausted", message, vault_id, false)
        }
        VaultRegistryError::LockPoisoned => {
            internal_error("Vault registry write lock poisoned", vault_id)
        }
        VaultRegistryError::InvalidDefinition(error) => definition_error(error, vault_id),
        VaultRegistryError::Storage(detail) => internal_error(detail, vault_id),
    }
}

fn vault_not_found(vault_id: VaultId) -> VaultOperationError {
    VaultOperationError::new(
        "vault_not_found",
        "Vault definition was not found",
        Some(vault_id),
        false,
    )
}

fn vault_disabled(vault_id: VaultId) -> VaultOperationError {
    VaultOperationError::new("vault_disabled", "Vault is disabled", Some(vault_id), false)
}

fn vault_unavailable(vault_id: VaultId) -> VaultOperationError {
    VaultOperationError::new(
        "vault_unavailable",
        "Vault is not currently accepting background work",
        Some(vault_id),
        true,
    )
}

// ---------------------------------------------------------------------------
// Projections
// ---------------------------------------------------------------------------

/// The status projection every registry-known Vault should have once
/// reconciled. A missing entry means this request observed a registry commit
/// the runtime has not reconciled yet — every mutation below commits and
/// reconciles within the same call, so this is defensive rather than an
/// expected path, and is reported as a retryable Vault-scoped status error
/// rather than a panic.
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

fn vault_summary(
    definition: &VaultDefinition,
    snapshot: &CollectionVaultSnapshot,
    clock: Option<GitPollingClock>,
) -> VaultSummary {
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
        last_checked_at: clock
            .and_then(|clock| clock.last_checked_at)
            .map(format_timestamp),
        next_attempt_at: clock.map(|clock| format_timestamp(clock.next_attempt_at)),
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
        last_checked_at: None,
        next_attempt_at: None,
        watcher: snapshot.watcher,
        capabilities: snapshot.capabilities,
        activation_error: None,
        search_error: None,
        git_error: None,
        watcher_error: None,
    }
}

fn vault_summary_for(
    collection_snapshot: &VaultCollectionSnapshot,
    registry_snapshot: &VaultRegistrySnapshot,
    vault_id: VaultId,
    clock: Option<GitPollingClock>,
) -> Option<VaultSummary> {
    let definition = registry_snapshot.definition(vault_id)?;
    let runtime_snapshot = collection_snapshot
        .vaults
        .get(&vault_id)
        .cloned()
        .unwrap_or_else(|| unreconciled_snapshot(&definition));
    Some(vault_summary(&definition, &runtime_snapshot, clock))
}

// ---------------------------------------------------------------------------
// The core
// ---------------------------------------------------------------------------

/// The Vault collection management core. Cheap to construct per call, like
/// [`crate::vault_read::VaultReadCore`] and
/// [`crate::vault_mutation::VaultMutationCore`]; it borrows the composed
/// runtime because a definition change touches the registry, the live
/// collection runtime, the work coordinator, the managed-Git scheduler, and
/// the pending legacy-import recovery flag together, and reconciling them in
/// that order within one call is the behaviour this module exists to own.
pub struct VaultCollectionManagement<'a> {
    state: &'a AppState,
}

impl<'a> VaultCollectionManagement<'a> {
    pub fn new(state: &'a AppState) -> Self {
        Self { state }
    }

    /// Discovery: every Vault this instance serves, with the collection
    /// revision the statuses were read at. Reachable at zero enabled Vaults
    /// and while the registry itself needs operator recovery, which it
    /// reports as an explicit `recovery` object rather than as an error.
    ///
    /// The projection forks on demo mode (#109) because the same read is
    /// unauthenticated there: a demo lists only *enabled* definitions and
    /// builds each through [`public_vault_summary`]. A disabled definition is
    /// an operator's own bookkeeping about an instance a visitor cannot
    /// administer, and listing it would name a Vault the demo does not serve.
    pub fn list(&self) -> Result<VaultDiscoveryResponse, VaultOperationError> {
        match self.state.vault_registry.load() {
            Ok(VaultRegistryState::Ready(registry_snapshot)) => {
                let collection_snapshot = self.state.vaults.snapshot();
                let demo_mode = self.state.demo_mode;
                let legacy_migration_recovery = self
                    .state
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
                                vault_summary(
                                    &definition,
                                    &runtime_snapshot,
                                    self.state.managed_git.polling_clock(definition.vault_id()),
                                )
                            }
                        })
                        .collect()
                };
                Ok(VaultDiscoveryResponse {
                    registry_revision: Some(registry_snapshot.revision()),
                    collection_revision: collection_snapshot.collection_revision,
                    vaults,
                    recovery: None,
                    legacy_migration_recovery,
                    demo_mode,
                })
            }
            Ok(VaultRegistryState::Recovery(recovery)) => Ok(VaultDiscoveryResponse {
                registry_revision: None,
                collection_revision: 0,
                vaults: Vec::new(),
                recovery: Some(RegistryRecoveryInfo::from(&recovery)),
                legacy_migration_recovery: None,
                demo_mode: self.state.demo_mode,
            }),
            Err(error) => Err(internal_error(error.to_string(), None)),
        }
    }

    /// Create a new Vault definition, seeding it when it qualifies.
    pub async fn create(
        &self,
        request: CreateVaultRequest,
    ) -> Result<VaultMutationResponse, VaultOperationError> {
        // `VaultRegistryStore::add` generates the new Vault's ID internally
        // but returns only the resulting snapshot, so the created ID is
        // recovered by diffing the ID sets before and after. This is sound
        // only because a successful `add` commits from exactly the revision
        // this `load` observed (its own internal compare-and-swap rejects any
        // commit that raced in between as a `RevisionConflict`, which never
        // reaches this diff) — a future change to that CAS contract would
        // need to preserve this guarantee or expose the generated ID
        // directly.
        let before_ids: BTreeSet<VaultId> = match self.state.vault_registry.load() {
            Ok(VaultRegistryState::Ready(snapshot)) => snapshot.vault_ids().collect(),
            Ok(VaultRegistryState::Recovery(recovery)) => return Err(recovery_error(&recovery)),
            Err(error) => return Err(internal_error(error.to_string(), None)),
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
        let snapshot = self
            .state
            .vault_registry
            .add(request.expected_registry_revision, definition)
            .map_err(|error| registry_error(error, None))?;

        let vault_id = snapshot.vault_ids().find(|id| !before_ids.contains(id));
        // Seed from the *committed* definition, not the request: the registry
        // canonicalizes the path and normalizes the exclude patterns, and the
        // emptiness decision has to be made against what was actually stored.
        // Runs before `reconcile_after_commit` activates the Vault and queues
        // its first Index turn, so the starter notes are in that first index
        // rather than arriving later as a watcher event.
        if let Some(definition) = vault_id.and_then(|vault_id| snapshot.definition(vault_id)) {
            seed_new_vault_or_log(&definition);
        }
        self.reconcile_after_commit(&snapshot)
            .await
            .map_err(|error| internal_error(error, vault_id))?;
        Ok(self.mutation_response(&snapshot, vault_id))
    }

    /// Edit an existing Vault definition.
    pub async fn edit(
        &self,
        vault_id: VaultId,
        request: EditVaultRequest,
    ) -> Result<VaultMutationResponse, VaultOperationError> {
        // Captured before `request.https_credentials` moves into `edit`
        // below. `VaultDefinition`'s `PartialEq` only ever compares
        // `credential_configured` (a bool), never the redacted credential
        // value itself — by design, so retention/reconciliation never needs
        // to see a plaintext secret. That means replacing token A with token
        // B leaves `credential_configured` `true` before and after, so
        // nothing downstream of the registry commit can observe "credentials
        // actually changed" from the definition alone; this flag is the one
        // place that genuinely knows a new value was written this call
        // (issue #97's reopening finding 3; also relied on below for issue
        // #98's reopening finding, since the same blind spot means
        // `reconcile()` never emits a collection-revision event for a
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
        let snapshot = self
            .state
            .vault_registry
            .edit(request.expected_registry_revision, vault_id, edit)
            .map_err(|error| registry_error(error, Some(vault_id)))?;

        self.reconcile_after_commit(&snapshot)
            .await
            .map_err(|error| internal_error(error, Some(vault_id)))?;
        if credentials_replaced {
            self.request_git_retry_after_credential_replacement(&snapshot, vault_id);
            // Issue #98's reopening finding: `reconcile()` retained the same
            // `VaultControlBlock` above (definition equality can't see a
            // credential-only change), so it never bumped
            // `collection_revision` or emitted an event for this Vault.
            // Notify explicitly so SSE consumers can still invalidate its
            // Git/capability state after a real secret rotation.
            self.state.vaults.notify_definition_changed(vault_id);
        }
        Ok(self.mutation_response(&snapshot, Some(vault_id)))
    }

    /// Enable or disable an existing Vault definition.
    pub async fn set_enabled(
        &self,
        vault_id: VaultId,
        expected_registry_revision: u64,
        enabled: bool,
    ) -> Result<VaultMutationResponse, VaultOperationError> {
        let result = if enabled {
            self.state
                .vault_registry
                .enable(expected_registry_revision, vault_id)
        } else {
            self.state
                .vault_registry
                .disable(expected_registry_revision, vault_id)
        };
        let snapshot = result.map_err(|error| registry_error(error, Some(vault_id)))?;
        self.reconcile_after_commit(&snapshot)
            .await
            .map_err(|error| internal_error(error, Some(vault_id)))?;
        Ok(self.mutation_response(&snapshot, Some(vault_id)))
    }

    /// Disconnect a Vault. Deletes no files, checkouts, Git history, or
    /// credentials outside this registry record.
    pub async fn disconnect(
        &self,
        vault_id: VaultId,
        expected_registry_revision: u64,
    ) -> Result<VaultMutationResponse, VaultOperationError> {
        let snapshot = self
            .state
            .vault_registry
            .disconnect(expected_registry_revision, vault_id)
            .map_err(|error| registry_error(error, Some(vault_id)))?;
        self.reconcile_after_commit(&snapshot)
            .await
            .map_err(|error| internal_error(error, Some(vault_id)))?;
        Ok(self.mutation_response(&snapshot, None))
    }

    /// Request an immediate managed-Git turn for one Vault, bypassing its
    /// daily schedule.
    pub fn sync(&self, vault_id: VaultId) -> Result<VaultScheduleResponse, VaultOperationError> {
        self.managed_git_control(vault_id, false)
    }

    /// The same admitted operation as [`Self::sync`], kept as a distinctly
    /// named entry point (mirrors `ManagedGitScheduler::retry_now`).
    pub fn retry(&self, vault_id: VaultId) -> Result<VaultScheduleResponse, VaultOperationError> {
        self.managed_git_control(vault_id, true)
    }

    fn managed_git_control(
        &self,
        vault_id: VaultId,
        retry: bool,
    ) -> Result<VaultScheduleResponse, VaultOperationError> {
        let definition = self.enabled_definition(vault_id)?;
        let Some(poll_interval) = definition.source().managed_git_poll_interval() else {
            return Err(VaultOperationError::new(
                "capability_unavailable",
                "Manual Git sync is only available for a Vault with a configured remote",
                Some(vault_id),
                false,
            ));
        };

        let schedule = if retry {
            self.state.managed_git.retry_now(vault_id, poll_interval)
        } else {
            self.state.managed_git.sync_now(vault_id, poll_interval)
        };
        schedule_response(vault_id, schedule)
    }

    /// Request one Vault's next Index turn. This only admits work to the
    /// shared FIFO; the runtime worker performs the authoritative Markdown
    /// scan and atomic snapshot publication.
    pub fn refresh(&self, vault_id: VaultId) -> Result<VaultScheduleResponse, VaultOperationError> {
        self.enabled_definition(vault_id)?;
        if self
            .state
            .vaults
            .runtime(vault_id)
            .is_none_or(|runtime| !runtime.snapshot().capabilities.browse)
        {
            return Err(VaultOperationError::new(
                "capability_unavailable",
                "Vault refresh requires currently usable local Markdown content",
                Some(vault_id),
                true,
            ));
        }
        schedule_response(
            vault_id,
            self.state
                .vault_work
                .request(vault_id, VaultWorkKind::Index),
        )
    }

    /// The confirmed recovery action offered only when a failed legacy import
    /// left `AppState`'s `legacy_migration_recovery` set (#150). Writes an
    /// ordinary empty, revision-1 registry and clears that recovery flag;
    /// unreachable (and left untouched) once the registry already holds real
    /// state, since `start_with_no_vaults` always commits from revision 0.
    pub async fn start_with_no_vaults(
        &self,
        confirm: bool,
    ) -> Result<VaultMutationResponse, VaultOperationError> {
        let recovery = self
            .state
            .legacy_migration_recovery
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let Some(recovery) = recovery else {
            return Err(VaultOperationError::new(
                "legacy_migration_recovery_not_pending",
                "There is no failed legacy import waiting for recovery.",
                None,
                false,
            ));
        };
        if !recovery.can_start_with_no_vaults() {
            return Err(VaultOperationError::new(
                "legacy_environment_cleanup_required",
                recovery.message(),
                None,
                false,
            ));
        }

        let snapshot =
            crate::vault_migration::start_with_no_vaults(&self.state.vault_registry, confirm)
                .map_err(|error| match error {
                    crate::vault_migration::LegacyMigrationError::ConfirmationRequired => {
                        VaultOperationError::new(
                            "confirmation_required",
                            "Starting with no Vaults requires confirm: true.",
                            None,
                            false,
                        )
                    }
                    crate::vault_migration::LegacyMigrationError::Registry(error) => {
                        registry_error(error, None)
                    }
                    crate::vault_migration::LegacyMigrationError::Storage(detail) => {
                        internal_error(detail, None)
                    }
                })?;

        // Reconciles like every other registry-mutating operation here, even
        // though the transition is empty-registry to empty-registry today: it
        // keeps `state.vaults`'s own collection revision (and its
        // `/api/v1/vaults/events` SSE publication) from silently lagging the
        // registry commit, matching what a future consumer of this collection
        // revision expects.
        self.reconcile_after_commit(&snapshot)
            .await
            .map_err(|error| internal_error(error, None))?;
        *self
            .state
            .legacy_migration_recovery
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        Ok(self.mutation_response(&snapshot, None))
    }

    /// The collection-revision channel the HTTP adapter's SSE route publishes
    /// from. It lives here because ADR-19 forbids an adapter reaching past a
    /// core into the runtime; the `Event` framing, keep-alive, and stream
    /// itself stay in the adapter, which is the only surface that has one.
    pub fn subscribe_revisions(
        &self,
    ) -> tokio::sync::watch::Receiver<VaultCollectionRevisionEvent> {
        self.state.vaults.subscribe_revisions()
    }

    /// The Vault-control precondition both manual Git controls and the Index
    /// refresh share: the Vault has to be in the registry and enabled before
    /// any per-source capability question is worth asking.
    fn enabled_definition(
        &self,
        vault_id: VaultId,
    ) -> Result<VaultDefinition, VaultOperationError> {
        let registry_snapshot = match self.state.vault_registry.load() {
            Ok(VaultRegistryState::Ready(snapshot)) => snapshot,
            Ok(VaultRegistryState::Recovery(recovery)) => return Err(recovery_error(&recovery)),
            Err(error) => return Err(internal_error(error.to_string(), Some(vault_id))),
        };
        let definition = registry_snapshot
            .definition(vault_id)
            .ok_or_else(|| vault_not_found(vault_id))?;
        if !definition.enabled() {
            return Err(vault_disabled(vault_id));
        }
        Ok(definition)
    }

    async fn reconcile_after_commit(&self, snapshot: &VaultRegistrySnapshot) -> Result<(), String> {
        let vaults = self.state.vaults.clone();
        let registry = self.state.vault_registry.clone();
        let vault_work = self.state.vault_work.clone();
        let managed_git = self.state.managed_git.clone();
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
        mutation_safe.await.map_err(|_| {
            "Vault reconciliation task ended before its mutation boundary".to_string()
        })?
    }

    fn mutation_response(
        &self,
        snapshot: &VaultRegistrySnapshot,
        vault_id: Option<VaultId>,
    ) -> VaultMutationResponse {
        // One snapshot for both the reported `collection_revision` and the
        // returned Vault's status, so the two can never disagree about which
        // collection state they describe.
        let collection_snapshot = self.state.vaults.snapshot();
        let vault = vault_id.and_then(|vault_id| {
            vault_summary_for(
                &collection_snapshot,
                snapshot,
                vault_id,
                self.state.managed_git.polling_clock(vault_id),
            )
        });
        VaultMutationResponse {
            vault,
            registry_revision: snapshot.revision(),
            collection_revision: collection_snapshot.collection_revision,
        }
    }

    /// Request an immediate Git turn for `vault_id` after its edit just wrote
    /// new HTTPS credentials (issue #97's reopening finding 3): a prior
    /// authentication failure otherwise sits unretried until the normal
    /// schedule, since `VaultDefinition` equality — which `reconcile()` uses
    /// to decide whether to retain the existing `VaultControlBlock` and skip
    /// its Pending-status Git-turn request — cannot see the credential value
    /// change, only whether a credential is configured at all (`true` both
    /// before and after a Replace-over-Replace).
    ///
    /// [`Self::edit`] is the only call site of
    /// `VaultRegistryStore::edit`/`VaultDefinitionEdit` — both the HTTP route
    /// and the MCP `edit_vault` tool reach it — so this trigger, run once per
    /// successful edit after `reconcile_after_commit` completes, covers every
    /// path that can write `https_credentials`.
    ///
    /// Source-agnostic: both `ManagedGit` and a remote-backed `ExistingGit`
    /// (`PullOnly`/`TwoWay`) Vault are tracked by `ManagedGitScheduler` and
    /// carry a `managed_git_poll_interval`, so both retry through
    /// `managed_git.retry_now`, which self-registers the Vault if it was not
    /// already tracked. `Local` and an `ExistingGit` `LocalHistory` Vault
    /// carry no interval and are skipped — `Local` never accepts credentials
    /// (rejected earlier at commit time) and so never reaches here with
    /// `credentials_replaced` true, and `LocalHistory` has no remote to
    /// retry.
    fn request_git_retry_after_credential_replacement(
        &self,
        snapshot: &VaultRegistrySnapshot,
        vault_id: VaultId,
    ) {
        let Some(definition) = snapshot.definition(vault_id) else {
            return;
        };
        if let Some(poll_interval) = definition.source().managed_git_poll_interval() {
            self.state.managed_git.retry_now(vault_id, poll_interval);
        }
    }
}

fn schedule_response(
    vault_id: VaultId,
    schedule: ScheduleResult,
) -> Result<VaultScheduleResponse, VaultOperationError> {
    match schedule {
        ScheduleResult::Queued => Ok(VaultScheduleResponse {
            vault_id,
            schedule: "queued".to_string(),
        }),
        ScheduleResult::Coalesced => Ok(VaultScheduleResponse {
            vault_id,
            schedule: "coalesced".to_string(),
        }),
        ScheduleResult::Rejected => Err(vault_unavailable(vault_id)),
    }
}

/// Seed a newly created Vault, logging rather than failing the call when that
/// does not work out.
///
/// Which Vaults qualify is `vault::seed_new_vault`'s decision, shared with the
/// legacy import path so the rule cannot drift between the two ways a Vault
/// definition comes into existence. All this adds is the operator-facing log
/// line: the Vault is already committed to the registry by the time this runs,
/// and refusing the whole creation because the welcome notes could not be
/// written would be a worse outcome than an empty Vault.
fn seed_new_vault_or_log(definition: &VaultDefinition) {
    match crate::vault::seed_new_vault(definition.source(), definition.exclude_patterns()) {
        Ok(true) => tracing::info!(
            vault_id = %definition.vault_id(),
            "Seeded new Vault with Hatchdoor starter notes"
        ),
        Ok(false) => {}
        Err(error) => error!(
            vault_id = %definition.vault_id(),
            %error,
            "could not seed the new Vault with starter notes"
        ),
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use crate::app_state::AppState;
    use crate::cache::SqliteCache;

    /// A minimal `AppState` for exercising Vault collection management
    /// directly (mirrors `mcp/routes.rs`'s own `test_state` shape).
    /// `startup_sqlite` still needs a real (in-memory, so cheap)
    /// `SqliteCache` because the field is not optional; the tests register
    /// whatever Vaults they need through the registry.
    ///
    /// Returns the coordinator's worker too — discarded by most callers, but
    /// a test that needs to drain a queued turn (e.g. the one Vault
    /// activation itself requests) needs direct access to it, since nothing
    /// else in this process consumes the coordinator's queue. Also returns
    /// the backing `TempDir` so it outlives the state.
    ///
    /// Shared with `handlers/vaults.rs`'s adapter tests, which drive the same
    /// composed runtime through the HTTP entry points.
    pub(crate) fn test_state() -> (
        AppState,
        crate::vault_work::VaultWorkWorker,
        tempfile::TempDir,
    ) {
        let directory = tempfile::tempdir().expect("temp dir");
        let (mcp_tools_changed, _) = tokio::sync::broadcast::channel(16);
        let (vault_work, worker) = crate::vault_work::VaultWorkCoordinator::new();
        let registry_path = directory.path().join("state/vaults.json");
        let managed_git = std::sync::Arc::new(crate::git::ManagedGitScheduler::with_state_store(
            vault_work.clone(),
            std::sync::Arc::new(
                crate::vault_runtime_state::VaultRuntimeStateStore::beside_registry(&registry_path),
            ),
        ));
        let state = AppState {
            vault_registry: crate::vault_registry::VaultRegistryStore::new(registry_path),
            vaults: crate::vault_runtime::VaultCollectionRuntime::new(),
            vault_work,
            managed_git,
            legacy_migration_recovery: std::sync::Arc::new(std::sync::RwLock::new(None)),
            startup_sqlite: std::sync::Arc::new(
                SqliteCache::in_memory(384).expect("in-memory cache"),
            ),
            mcp_tools_changed,
            embedder: crate::app_state::test_embedder(),
            runtime_embedder: std::sync::Arc::new(crate::embed::RuntimeEmbedder::new()),
            model_setup: std::sync::Arc::new(crate::model_setup::ModelSetup::new(
                directory.path().join("models"),
            )),
            model_setup_started: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
            web_auth_enabled: false,
            demo_mode: false,
            runtime_config: crate::runtime_config::RuntimeConfig::for_tests(),
            startup: crate::startup::StartupTracker::ready(),
        };
        (state, worker, directory)
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::test_state;
    use super::*;
    use crate::vault_registry::{DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS, VaultGitMode};
    use crate::vault_runtime::VaultChangeCategory;
    use crate::vault_work::VaultWorkError;

    fn managed_git_source(poll_interval_secs: u64) -> VaultSource {
        VaultSource::ManagedGit {
            repository_url: "https://example.test/owner/notes.git".to_string(),
            branch: None,
            vault_subdirectory: None,
            mode: VaultGitMode::PullOnly,
            poll_interval_secs,
        }
    }

    fn create_request(name: &str, source: VaultSource) -> CreateVaultRequest {
        CreateVaultRequest {
            expected_registry_revision: 0,
            name: name.to_string(),
            enabled: true,
            source,
            exclude_patterns: Vec::new(),
            https_credentials: None,
            archive_folder: None,
            commit_identity: None,
        }
    }

    fn ready_snapshot(state: &AppState) -> VaultRegistrySnapshot {
        let VaultRegistryState::Ready(snapshot) =
            state.vault_registry.load().expect("load registry")
        else {
            panic!("registry entered recovery");
        };
        snapshot
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

    /// #109's projection rule, asserted where it now lives: a demo publishes
    /// only enabled Vaults, and withholds everything that describes the
    /// operator's deployment rather than the content. `credential_configured`
    /// survives deliberately (#133 designates it the only credential signal).
    #[tokio::test]
    async fn demo_discovery_publishes_only_enabled_vaults_through_the_public_projection() {
        let (mut state, _worker, directory) = test_state();
        let published_path = directory.path().join("published");
        let hidden_path = directory.path().join("hidden");
        std::fs::create_dir_all(&published_path).expect("vault dir");
        std::fs::create_dir_all(&hidden_path).expect("vault dir");

        let core = VaultCollectionManagement::new(&state);
        core.create(CreateVaultRequest {
            exclude_patterns: vec!["Private/**".to_string()],
            archive_folder: Some("Archive".to_string()),
            commit_identity: Some(VaultCommitIdentity {
                name: "Operator".to_string(),
                email: "operator@example.test".to_string(),
            }),
            ..create_request(
                "Published",
                VaultSource::Local {
                    path: published_path,
                },
            )
        })
        .await
        .expect("create the published Vault");

        let revision = ready_snapshot(&state).revision();
        core.create(CreateVaultRequest {
            expected_registry_revision: revision,
            enabled: false,
            ..create_request("Hidden", VaultSource::Local { path: hidden_path })
        })
        .await
        .expect("create the disabled Vault");

        let authenticated = VaultCollectionManagement::new(&state)
            .list()
            .expect("authenticated discovery");
        assert_eq!(authenticated.vaults.len(), 2);
        assert!(!authenticated.demo_mode);
        let published = authenticated
            .vaults
            .iter()
            .find(|vault| vault.name == "Published")
            .expect("the published Vault");
        assert!(published.source.is_some());
        assert_eq!(published.exclude_patterns, vec!["Private/**".to_string()]);
        // The registry canonicalizes the stored folder, so the projection
        // reports what was committed rather than what was requested.
        assert_eq!(published.archive_folder.as_deref(), Some("Archive/"));
        assert!(published.commit_identity.is_some());

        state.demo_mode = true;
        let demo = VaultCollectionManagement::new(&state)
            .list()
            .expect("demo discovery");
        assert!(demo.demo_mode);
        assert_eq!(
            demo.vaults.len(),
            1,
            "a demo must not name a Vault it does not serve"
        );
        let published = &demo.vaults[0];
        assert_eq!(published.name, "Published");
        assert!(published.source.is_none());
        assert!(published.exclude_patterns.is_empty());
        assert!(published.archive_folder.is_none());
        assert!(published.commit_identity.is_none());
        assert!(published.activation_error.is_none());
        assert!(published.search_error.is_none());
        assert!(published.git_error.is_none());
        assert!(published.watcher_error.is_none());
        assert!(!published.credential_configured);
        // Capabilities stay honest in demo mode (#133): the browser branches
        // on the instance-level flag, not on a rewritten per-Vault capability.
        assert_eq!(
            published.capabilities,
            authenticated
                .vaults
                .iter()
                .find(|vault| vault.name == "Published")
                .expect("the published Vault")
                .capabilities
        );
    }

    /// #150's recovery action, asserted where it now lives: it requires a
    /// pending failed import and an explicit confirmation, and clears the
    /// flag once the empty registry is committed and reconciled.
    #[tokio::test]
    async fn start_with_no_vaults_requires_a_pending_import_and_an_explicit_confirmation() {
        let (state, _worker, _directory) = test_state();

        let not_pending = VaultCollectionManagement::new(&state)
            .start_with_no_vaults(true)
            .await
            .expect_err("no failed import is pending");
        assert_eq!(not_pending.code, "legacy_migration_recovery_not_pending");

        // A pending import that still needs the operator to clean their
        // environment cannot be resolved by starting empty at all.
        *state
            .legacy_migration_recovery
            .write()
            .expect("recovery lock") = Some(
            crate::vault_migration::LegacyMigrationRecovery::environment_cleanup(
                "remove the legacy environment variables first",
            ),
        );
        let needs_cleanup = VaultCollectionManagement::new(&state)
            .start_with_no_vaults(true)
            .await
            .expect_err("the environment still needs cleaning");
        assert_eq!(needs_cleanup.code, "legacy_environment_cleanup_required");

        *state
            .legacy_migration_recovery
            .write()
            .expect("recovery lock") =
            Some(crate::vault_migration::LegacyMigrationRecovery::for_test(
                "automatic import could not prove the legacy deployment",
            ));

        let unconfirmed = VaultCollectionManagement::new(&state)
            .start_with_no_vaults(false)
            .await
            .expect_err("a bare call is not enough");
        assert_eq!(unconfirmed.code, "confirmation_required");
        assert!(
            state
                .legacy_migration_recovery
                .read()
                .expect("recovery lock")
                .is_some(),
            "a refused recovery must leave the flag pending"
        );

        let response = VaultCollectionManagement::new(&state)
            .start_with_no_vaults(true)
            .await
            .expect("confirmed recovery");
        assert_eq!(response.registry_revision, 1);
        assert!(response.vault.is_none());
        assert!(
            state
                .legacy_migration_recovery
                .read()
                .expect("recovery lock")
                .is_none(),
            "a successful recovery must clear the pending flag"
        );

        // Discovery stops withholding the collection once the flag is clear.
        let discovery = VaultCollectionManagement::new(&state)
            .list()
            .expect("discovery");
        assert!(discovery.legacy_migration_recovery.is_none());
        assert_eq!(discovery.registry_revision, Some(1));
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

        VaultCollectionManagement::new(&state)
            .create(CreateVaultRequest {
                https_credentials: Some(HttpsCredentialsInput {
                    username: Some("git-user".to_string()),
                    token: "old-token".to_string(),
                }),
                ..create_request(
                    "Remote notes",
                    managed_git_source(DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS),
                )
            })
            .await
            .expect("create the Vault");
        let snapshot = ready_snapshot(&state);
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
        let current = ready_snapshot(&state);
        VaultCollectionManagement::new(&state)
            .edit(
                vault_id,
                EditVaultRequest {
                    expected_registry_revision: current.revision(),
                    name: "Remote notes".to_string(),
                    source: managed_git_source(DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS),
                    exclude_patterns: Vec::new(),
                    https_credentials: HttpsCredentialsPatch::Replace {
                        username: Some("git-user".to_string()),
                        token: "new-token".to_string(),
                    },
                    confirm_identity_change: false,
                    archive_folder: None,
                    commit_identity: None,
                },
            )
            .await
            .expect("edit the Vault");

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

        VaultCollectionManagement::new(&state)
            .create(CreateVaultRequest {
                https_credentials: Some(HttpsCredentialsInput {
                    username: Some("git-user".to_string()),
                    token: "old-token".to_string(),
                }),
                ..create_request(
                    "Remote notes",
                    managed_git_source(DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS),
                )
            })
            .await
            .expect("create the Vault");
        let snapshot = ready_snapshot(&state);
        let vault_id = snapshot.vault_ids().next().expect("one Vault");

        let mut revisions = state.vaults.subscribe_revisions();
        let before = revisions.borrow().collection_revision;

        VaultCollectionManagement::new(&state)
            .edit(
                vault_id,
                EditVaultRequest {
                    expected_registry_revision: snapshot.revision(),
                    name: "Remote notes".to_string(),
                    source: managed_git_source(DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS),
                    exclude_patterns: Vec::new(),
                    https_credentials: HttpsCredentialsPatch::Replace {
                        username: Some("git-user".to_string()),
                        token: "new-token".to_string(),
                    },
                    confirm_identity_change: false,
                    archive_folder: None,
                    commit_identity: None,
                },
            )
            .await
            .expect("edit the Vault");

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
    /// this test instead drives the real edit path (registry `edit()` +
    /// `reconcile_and_reconstruct`) end to end and observes the *running*
    /// scheduler's stored per-Vault state through its `#[cfg(test)]`
    /// accessors, proving: (1) an interval-only edit on an already-active
    /// managed-Git Vault reaches the live scheduler with no restart or
    /// manual reactivation, and (2) doing so leaves an in-progress backoff
    /// untouched — interval update and backoff preservation are two
    /// independent concerns, and this is where that claim is actually
    /// load-bearing (the real edit path), not just at the scheduler-unit
    /// level.
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

        VaultCollectionManagement::new(&state)
            .create(create_request(
                "Remote notes",
                managed_git_source(DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS),
            ))
            .await
            .expect("create the Vault");
        let vault_id = ready_snapshot(&state)
            .vault_ids()
            .next()
            .expect("one Vault");

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
        let current = ready_snapshot(&state);
        let new_interval_secs = DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS * 2;
        VaultCollectionManagement::new(&state)
            .edit(
                vault_id,
                EditVaultRequest {
                    expected_registry_revision: current.revision(),
                    name: "Remote notes".to_string(),
                    source: managed_git_source(new_interval_secs),
                    exclude_patterns: Vec::new(),
                    https_credentials: HttpsCredentialsPatch::Keep,
                    confirm_identity_change: false,
                    archive_folder: None,
                    commit_identity: None,
                },
            )
            .await
            .expect("edit the Vault");

        // The edit itself took effect: the reloaded definition carries the
        // new interval.
        assert_eq!(
            ready_snapshot(&state)
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

    /// Issue #132: sync and retry used to refuse every `ExistingGit` Vault
    /// outright — the control checked `managed_git_poll_interval()`, which
    /// returned `None` for every `ExistingGit` mode. A remote-backed
    /// `ExistingGit` Vault (`PullOnly`/`TwoWay`) is now scheduler-tracked
    /// exactly like `ManagedGit`, so both controls must admit it — this is
    /// the acceptance criterion "`An ExistingGit Vault in PullOnly or TwoWay
    /// ... accepts POST /sync and POST /retry`", mirroring
    /// `vaults_v1_sync_and_retry_require_a_managed_git_source`'s coverage of
    /// the still-refused `Local` case (`src/server.rs`) but for the
    /// newly-admitted source kind.
    #[tokio::test]
    async fn sync_and_retry_admit_an_existing_git_pull_only_vault_and_track_its_schedule() {
        let (state, _worker, directory) = test_state();
        let repository_path = directory.path().join("existing-pull-only-repo");
        std::fs::create_dir_all(&repository_path).expect("create repo directory");
        git2::Repository::init(&repository_path).expect("init git repo");

        VaultCollectionManagement::new(&state)
            .create(create_request(
                "Existing checkout",
                VaultSource::ExistingGit {
                    repository_path,
                    repository_url: Some("https://example.test/owner/notes.git".to_string()),
                    branch: None,
                    vault_subdirectory: None,
                    mode: VaultGitMode::PullOnly,
                    poll_interval_secs: DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
                },
            ))
            .await
            .expect("create the Vault");
        let vault_id = ready_snapshot(&state)
            .vault_ids()
            .next()
            .expect("one Vault");

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

        let core = VaultCollectionManagement::new(&state);
        core.sync(vault_id)
            .expect("manual sync must no longer be refused for an ExistingGit PullOnly Vault");
        core.retry(vault_id)
            .expect("manual retry must no longer be refused for an ExistingGit PullOnly Vault");
    }

    /// Companion to `sync_and_retry_admit_an_existing_git_pull_only_vault_and_track_its_schedule`:
    /// the other half of the acceptance criterion "An `ExistingGit` Vault in
    /// `LocalHistory` ... still refuse[s] both and carr[ies] no schedule" —
    /// `LocalHistory` has no remote to poll and must keep refusing both
    /// controls exactly as before.
    #[tokio::test]
    async fn sync_and_retry_still_refuse_an_existing_git_local_history_vault() {
        let (state, _worker, directory) = test_state();
        let repository_path = directory.path().join("local-history-repo");
        std::fs::create_dir_all(&repository_path).expect("create repo directory");
        git2::Repository::init(&repository_path).expect("init git repo");

        VaultCollectionManagement::new(&state)
            .create(create_request(
                "Local history checkout",
                VaultSource::ExistingGit {
                    repository_path,
                    repository_url: None,
                    branch: None,
                    vault_subdirectory: None,
                    mode: VaultGitMode::LocalHistory,
                    poll_interval_secs: DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
                },
            ))
            .await
            .expect("create the Vault");
        let vault_id = ready_snapshot(&state)
            .vault_ids()
            .next()
            .expect("one Vault");

        assert_eq!(
            state.managed_git.poll_interval_for_test(vault_id),
            None,
            "a LocalHistory Vault must never be tracked by the scheduler"
        );

        let core = VaultCollectionManagement::new(&state);
        assert_eq!(
            core.sync(vault_id).expect_err("sync refused").code,
            "capability_unavailable"
        );
        assert_eq!(
            core.retry(vault_id).expect_err("retry refused").code,
            "capability_unavailable"
        );
    }

    /// The starter Vault is a documented first-run behaviour: a brand-new
    /// `Local` Vault pointed at an empty directory opens on the welcome notes
    /// rather than on nothing at all.
    #[tokio::test]
    async fn creating_a_local_vault_on_an_empty_directory_seeds_the_starter_vault() {
        let (state, _worker, directory) = test_state();
        let path = directory.path().join("fresh-notes");
        std::fs::create_dir_all(&path).expect("vault dir");

        VaultCollectionManagement::new(&state)
            .create(create_request(
                "Fresh notes",
                VaultSource::Local { path: path.clone() },
            ))
            .await
            .expect("create the Vault");

        assert!(
            path.join("README.md").is_file(),
            "an empty new Local Vault must receive the starter notes"
        );

        // "before its first Index turn": that turn is still sitting in the
        // coordinator, unrun, with the starter notes already on disk — so the
        // index it builds will contain them rather than discovering them later
        // through a watcher event.
        let vault_id = ready_snapshot(&state)
            .vault_ids()
            .next()
            .expect("one Vault");
        assert!(
            state.vault_work.has_work(vault_id, VaultWorkKind::Index),
            "the Vault's first Index turn must still be pending when seeding has finished"
        );
    }

    /// Seeding must never touch a directory that already holds the operator's
    /// own Markdown, and a Vault whose notes were all deleted is never
    /// re-seeded: only creation seeds, and creation happens once.
    #[tokio::test]
    async fn creating_a_local_vault_on_a_directory_with_markdown_does_not_seed() {
        let (state, _worker, directory) = test_state();
        let path = directory.path().join("existing-notes");
        std::fs::create_dir_all(&path).expect("vault dir");
        std::fs::write(path.join("Home.md"), "home").expect("write note");

        VaultCollectionManagement::new(&state)
            .create(create_request(
                "Existing notes",
                VaultSource::Local { path: path.clone() },
            ))
            .await
            .expect("create the Vault");

        assert!(
            !path.join("README.md").exists(),
            "a Vault that already holds Markdown must be left exactly as it was"
        );
        assert_eq!(
            std::fs::read_to_string(path.join("Home.md")).expect("existing note"),
            "home"
        );
    }

    /// A trashed note is not content: the emptiness decision uses the Vault's
    /// own exclude matcher, which excludes the trash folders by default, so a
    /// directory holding only trash still gets the starter Vault.
    #[tokio::test]
    async fn a_directory_holding_only_trashed_notes_is_still_seeded() {
        let (state, _worker, directory) = test_state();
        let path = directory.path().join("trash-only");
        std::fs::create_dir_all(path.join(".hatchdoor-trash")).expect("trash dir");
        std::fs::write(path.join(".hatchdoor-trash/Gone.md"), "gone").expect("write trashed note");

        VaultCollectionManagement::new(&state)
            .create(create_request(
                "Trash only",
                VaultSource::Local { path: path.clone() },
            ))
            .await
            .expect("create the Vault");

        assert!(
            path.join("README.md").is_file(),
            "the trash folder does not count as Vault content"
        );
    }

    /// Writing starter notes into a Git working tree would manufacture a
    /// commit the operator never asked for, so a Git-backed source is never
    /// seeded whatever its directory holds.
    #[tokio::test]
    async fn creating_a_git_backed_vault_never_seeds() {
        let (state, _worker, directory) = test_state();
        let path = directory.path().join("cloned-notes");
        std::fs::create_dir_all(&path).expect("vault dir");

        VaultCollectionManagement::new(&state)
            .create(create_request(
                "Cloned notes",
                managed_git_source(DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS),
            ))
            .await
            .expect("create the Vault");

        assert!(
            !path.join("README.md").exists(),
            "a Git-backed Vault's content belongs to its repository"
        );
    }

    /// Disabling and re-enabling an emptied Vault must not resurrect the
    /// starter notes: only creation seeds.
    #[tokio::test]
    async fn re_enabling_an_emptied_vault_does_not_re_seed_it() {
        let (state, _worker, directory) = test_state();
        let path = directory.path().join("emptied-notes");
        std::fs::create_dir_all(&path).expect("vault dir");
        std::fs::write(path.join("Home.md"), "home").expect("write note");

        VaultCollectionManagement::new(&state)
            .create(create_request(
                "Emptied notes",
                VaultSource::Local { path: path.clone() },
            ))
            .await
            .expect("create the Vault");
        let snapshot = ready_snapshot(&state);
        let vault_id = snapshot.vault_ids().next().expect("one Vault");

        // The operator deletes every note, then cycles the Vault.
        std::fs::remove_file(path.join("Home.md")).expect("delete the only note");
        VaultCollectionManagement::new(&state)
            .set_enabled(vault_id, snapshot.revision(), false)
            .await
            .expect("disable the Vault");
        let after_disable = ready_snapshot(&state);
        VaultCollectionManagement::new(&state)
            .set_enabled(vault_id, after_disable.revision(), true)
            .await
            .expect("re-enable the Vault");

        assert!(
            !path.join("README.md").exists(),
            "an intentionally emptied Vault must never be re-seeded"
        );
    }
    /// Closes the observability half of the durable-schedule change: a
    /// managed-Git Vault's summary must say when it last completed a Git turn
    /// and when its next one is due. Without these, the only way to answer
    /// "is this Vault still polling?" is to read the server's logs or stat
    /// the checkout's `FETCH_HEAD` — which is how the gap was found.
    #[tokio::test]
    async fn a_managed_git_vaults_summary_reports_its_last_and_next_git_turn() {
        let (state, _worker, _directory) = test_state();
        VaultCollectionManagement::new(&state)
            .create(create_request(
                "Remote notes",
                VaultSource::ManagedGit {
                    repository_url: "https://example.test/vault.git".to_string(),
                    branch: Some("main".to_string()),
                    vault_subdirectory: None,
                    mode: VaultGitMode::PullOnly,
                    poll_interval_secs: DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
                },
            ))
            .await
            .expect("create the Vault");
        let vault_id = ready_snapshot(&state)
            .vault_ids()
            .next()
            .expect("one Vault");
        state
            .managed_git
            .record_outcome(vault_id, &Ok(crate::git::ManagedGitOutcome::UpToDate));

        let listed = VaultCollectionManagement::new(&state)
            .list()
            .expect("list the collection");
        let summary = listed
            .vaults
            .iter()
            .find(|summary| summary.vault_id == vault_id)
            .expect("the managed Vault");

        let last_checked_at = summary
            .last_checked_at
            .as_deref()
            .expect("a completed turn must be reported");
        let next_attempt_at = summary
            .next_attempt_at
            .as_deref()
            .expect("a tracked Vault must report its next turn");
        let parse = |raw: &str| {
            chrono::DateTime::parse_from_rfc3339(raw)
                .expect("an RFC 3339 timestamp")
                .timestamp()
        };
        let gap = parse(next_attempt_at) - parse(last_checked_at);
        let configured = i64::try_from(DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS).expect("interval");
        assert!(
            (gap - configured).abs() <= 2,
            "the next turn must fall one poll interval after the last one: {last_checked_at} -> {next_attempt_at}"
        );
    }

    /// Shortening a Vault's poll interval must bring its *already armed* next
    /// turn forward, not merely apply to the turn after it. Found in
    /// production: a Vault whose last turn armed a 24h deadline was edited
    /// down to hourly, and then sat for the rest of that 24h without a single
    /// poll, because `activate` updated the stored interval in place and left
    /// the deadline alone. The operator's whole reason for shortening the
    /// interval is the *next* check, so a change they can watch not happening
    /// for a day reads as a broken scheduler.
    ///
    /// Observed through the same `list()` summary the settings page reads,
    /// rather than the scheduler's test accessors, because the gap between
    /// `last_checked_at` and `next_attempt_at` is exactly what an operator
    /// checks after making this edit.
    #[tokio::test]
    async fn shortening_the_poll_interval_brings_the_armed_next_turn_forward() {
        let (state, _worker, _directory) = test_state();
        VaultCollectionManagement::new(&state)
            .create(create_request(
                "Remote notes",
                managed_git_source(DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS),
            ))
            .await
            .expect("create the Vault");
        let vault_id = ready_snapshot(&state)
            .vault_ids()
            .next()
            .expect("one Vault");
        // One successful turn, arming the long interval the edit below
        // shortens. Without it the Vault is due immediately anyway and the
        // property under test cannot fail.
        state
            .managed_git
            .record_outcome(vault_id, &Ok(crate::git::ManagedGitOutcome::UpToDate));

        let shortened_secs = 3600;
        assert!(
            shortened_secs < DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
            "this test only means anything if the edit shortens the interval"
        );
        let current = ready_snapshot(&state);
        VaultCollectionManagement::new(&state)
            .edit(
                vault_id,
                EditVaultRequest {
                    expected_registry_revision: current.revision(),
                    name: "Remote notes".to_string(),
                    source: managed_git_source(shortened_secs),
                    exclude_patterns: Vec::new(),
                    https_credentials: HttpsCredentialsPatch::Keep,
                    confirm_identity_change: false,
                    archive_folder: None,
                    commit_identity: None,
                },
            )
            .await
            .expect("edit the Vault");

        let listed = VaultCollectionManagement::new(&state)
            .list()
            .expect("list the collection");
        let summary = listed
            .vaults
            .iter()
            .find(|summary| summary.vault_id == vault_id)
            .expect("the managed Vault");
        let parse = |raw: &str| {
            chrono::DateTime::parse_from_rfc3339(raw)
                .expect("an RFC 3339 timestamp")
                .timestamp()
        };
        let last_checked_at = summary
            .last_checked_at
            .as_deref()
            .expect("the completed turn must still be reported");
        let next_attempt_at = summary
            .next_attempt_at
            .as_deref()
            .expect("a tracked Vault must report its next turn");
        let gap = parse(next_attempt_at) - parse(last_checked_at);
        let shortened = i64::try_from(shortened_secs).expect("interval");
        assert!(
            (gap - shortened).abs() <= 2,
            "the next turn must fall one *new* interval after the last one, \
             not one old interval: {last_checked_at} -> {next_attempt_at}"
        );
    }

    /// A Vault that has never completed a turn is due *now*, not unscheduled:
    /// `next_attempt_at` is what tells an operator the Vault is waiting on a
    /// check rather than forgotten, so it must be present from the moment the
    /// Vault is tracked. `last_checked_at` is the only one of the pair that
    /// waits for a completed turn.
    #[tokio::test]
    async fn a_managed_git_vault_reports_its_next_turn_before_its_first_one_completes() {
        let (state, _worker, _directory) = test_state();
        VaultCollectionManagement::new(&state)
            .create(create_request(
                "Remote notes",
                VaultSource::ManagedGit {
                    repository_url: "https://example.test/vault.git".to_string(),
                    branch: Some("main".to_string()),
                    vault_subdirectory: None,
                    mode: VaultGitMode::PullOnly,
                    poll_interval_secs: DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
                },
            ))
            .await
            .expect("create the Vault");

        let listed = VaultCollectionManagement::new(&state)
            .list()
            .expect("list the collection");
        let summary = listed.vaults.first().expect("the managed Vault");
        assert_eq!(
            summary.last_checked_at, None,
            "no turn has completed, so there is nothing to report as checked"
        );
        assert!(
            summary.next_attempt_at.is_some(),
            "a tracked Vault always has a next turn, even before its first one"
        );
    }

    /// A `Local` Vault has no remote to poll, so it carries no schedule to
    /// report — the fields stay absent rather than inventing a turn that
    /// never happens.
    #[tokio::test]
    async fn a_local_vaults_summary_reports_no_git_schedule() {
        let (state, _worker, directory) = test_state();
        let vault_path = directory.path().join("local-notes");
        std::fs::create_dir_all(&vault_path).expect("create the Vault directory");
        VaultCollectionManagement::new(&state)
            .create(create_request(
                "Local notes",
                VaultSource::Local { path: vault_path },
            ))
            .await
            .expect("create the Vault");

        let listed = VaultCollectionManagement::new(&state)
            .list()
            .expect("list the collection");
        let summary = listed.vaults.first().expect("the Local Vault");
        assert_eq!(summary.last_checked_at, None);
        assert_eq!(summary.next_attempt_at, None);
    }
}
