//! HTTP adapter for the live server-settings surface.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::Extension;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use crate::api_types::ErrorResponse;
use crate::app_state::{AppState, request_collection_reindex};
use crate::runtime_config::{ConfigSnapshot, SettingSource};

/// A deliberately generous ceiling: multipart files are buffered while they
/// are written, so accepting arbitrarily large limits would let one upload
/// exhaust the process.
pub const MAX_IN_MEMORY_UPLOAD_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Serialize)]
pub struct SettingsResponse {
    pub settings: Vec<SettingResponse>,
    /// The last MCP client that called a tool, or `None` when no agent has
    /// connected yet (#426). Read-only: no setting changes it.
    pub last_agent: Option<crate::instance_state::AgentConnection>,
    /// The opt-in check for a newer release (ADR-39): whether it is on, when
    /// it last ran, and the release the update banner offers, if any.
    /// Read-only; `HATCHDOOR_UPDATE_CHECK_ENABLED` above turns it on.
    pub update_check: crate::update_check::UpdateCheckStatus,
    /// The opt-in usage report (ADR-45): whether it is on, the install ID
    /// while it is, and the exact report the next send would carry.
    /// Read-only; `HATCHDOOR_USAGE_REPORT_ENABLED` above turns it on.
    pub usage_report: crate::usage_report::UsageReportStatus,
}

#[derive(Debug, Serialize)]
pub struct SettingResponse {
    pub key: &'static str,
    pub value: Option<String>,
    pub configured: Option<bool>,
    pub source: &'static str,
    pub locked: Option<&'static str>,
    #[serde(rename = "class")]
    pub class: &'static str,
    pub kind: &'static str,
}

/// The consequences a save may need explicit consent for. `reindex` is the
/// only one left: #183 retired `git_init` and `git_downgrade` with the
/// instance-wide Versioning console that explained them. `confirm` stays a
/// list because the wire shape is unchanged, but one save can no longer need
/// two consents, so nothing accumulates across `409`s any more (issue #57).
const KNOWN_CONSEQUENCES: &[&str] = &["reindex"];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchSettingsRequest {
    pub updates: BTreeMap<String, String>,
    #[serde(default)]
    pub confirm: Vec<String>,
}

impl PatchSettingsRequest {
    fn confirmed(&self, consequence: &str) -> bool {
        self.confirm.iter().any(|value| value == consequence)
    }
}

#[derive(Debug, Serialize)]
struct FieldError {
    /// `None` for a refusal that belongs to no single setting (issue #55):
    /// rendered by the page as a form-level message near the section actions
    /// instead of being silently dropped.
    key: Option<String>,
    message: String,
}

impl FieldError {
    fn on(key: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            key: Some(key.into()),
            message: message.into(),
        }
    }

    fn general(message: impl Into<String>) -> Self {
        Self {
            key: None,
            message: message.into(),
        }
    }
}

#[derive(Debug, Serialize)]
struct SettingsValidationError {
    error: String,
    fields: Vec<FieldError>,
}

/// A refusal from the atomic validate-then-persist decision (see
/// `RuntimeConfig::validate_and_save`). Nothing is written to disk when this
/// is returned.
enum PatchRefusal {
    /// One or more fields are individually invalid.
    Validation(Vec<FieldError>),
    /// The save is otherwise valid but has a consequence the caller has not
    /// yet accepted. The server sends only the machine-readable consequence;
    /// the page owns the words describing it (issue #55, #61).
    Confirmation { kind: &'static str },
    /// An unexpected internal failure (e.g. the settings file could not be
    /// written).
    Internal(String),
}

impl From<String> for PatchRefusal {
    fn from(value: String) -> Self {
        PatchRefusal::Internal(value)
    }
}

impl PatchRefusal {
    fn into_response(self) -> Response {
        match self {
            PatchRefusal::Validation(fields) => validation_response(fields),
            PatchRefusal::Confirmation { kind } => confirmation_required(kind),
            PatchRefusal::Internal(message) => {
                crate::app_state::internal_error(message).into_response()
            }
        }
    }
}

/// What to do once a validated save has been persisted: request a reindex,
/// announce a changed MCP tool catalogue, and/or install a newly spawned
/// versioning task.
struct PatchPlan {
    reindex_changed: bool,
    /// The save flips `HATCHDOOR_MCP_WRITE_ENABLED`, which is the only setting
    /// that adds or removes tools from the advertised MCP catalogue.
    mcp_write_toggled: bool,
}

const SETTINGS: &[(&str, &str, &str)] = &[
    ("HATCHDOOR_ARCHIVE_PREFIX", "instant", "text"),
    ("HATCHDOOR_EXCLUDE", "reindex", "text"),
    ("HATCHDOOR_EMBED_LAYERS", "reindex", "switch"),
    ("HATCHDOOR_MCP_ENABLED", "instant", "switch"),
    ("HATCHDOOR_MCP_WRITE_ENABLED", "instant", "switch"),
    ("HATCHDOOR_MCP_RATE_LIMITS_ENABLED", "instant", "switch"),
    ("HATCHDOOR_MCP_BEARER_TOKEN", "instant", "secret"),
    ("HATCHDOOR_MCP_ALLOWED_ORIGINS", "instant", "text"),
    ("HATCHDOOR_PUBLIC_URL", "instant", "text"),
    ("HATCHDOOR_MAX_ATTACHMENT_BYTES", "instant", "number"),
    ("HATCHDOOR_MCP_MAX_BASE64_BYTES", "instant", "number"),
    ("HATCHDOOR_UPDATE_CHECK_ENABLED", "instant", "switch"),
    ("HATCHDOOR_USAGE_REPORT_ENABLED", "instant", "switch"),
    // The `HATCHDOOR_GIT_*` keys below, and `HATCHDOOR_EXCLUDE` above, stay in
    // the schema although #185 deleted the instance-wide lane whose behaviour
    // they drove and #427 removed the import that consumed them; startup
    // still purges the retired Git-lane keys from stored settings and refuses
    // an install that stores any of them without a registry
    // (`vault_migration.rs`). Two startup checks still parse them — the demo-mode
    // posture refusal and `HATCHDOOR_EXCLUDE`'s pattern validation, both in
    // `server.rs` — but nothing reads them per operation. The two author keys
    // are the exception: the collection lane's Git turns read them per turn as
    // the commit-identity fallback for a Vault without its own (#181).
    ("HATCHDOOR_GIT_SYNC_ENABLED", "instant", "mode"),
    ("HATCHDOOR_GIT_HTTPS_USERNAME", "instant", "text"),
    ("HATCHDOOR_GIT_HTTPS_TOKEN", "instant", "secret"),
    ("HATCHDOOR_GIT_DEBOUNCE_SECONDS", "instant", "number"),
    ("HATCHDOOR_GIT_AUTHOR_NAME", "instant", "text"),
    ("HATCHDOOR_GIT_AUTHOR_EMAIL", "instant", "text"),
    ("HATCHDOOR_GIT_BRANCH", "instant", "text"),
];

pub async fn get_settings_handler(State(state): State<AppState>) -> impl IntoResponse {
    let snapshot = state.runtime_snapshot();
    Json(settings_response(
        &snapshot,
        state.demo_mode,
        state.agent_connections.latest(),
        update_check_status(&state, &snapshot),
        crate::usage_report::status(&state, &snapshot).await,
    ))
}

/// Read from the instance state file beside the registry, where the update
/// check writes it.
fn update_check_status(
    state: &AppState,
    snapshot: &ConfigSnapshot,
) -> crate::update_check::UpdateCheckStatus {
    crate::update_check::status(
        snapshot,
        &crate::instance_state::InstanceStateStore::beside_registry(state.vault_registry.path()),
        &crate::config::version_string(),
    )
}

/// A viewer who already authenticated with the web bearer token gains no new
/// capability by seeing it. The response is deliberately non-cacheable and is
/// not part of the ordinary settings document, which never contains secrets.
pub async fn reveal_web_token_handler(Extension(token): Extension<Option<Arc<str>>>) -> Response {
    let Some(token) = token else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut response = Json(serde_json::json!({ "value": token.to_string() })).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response
}

/// Produce an MCP token candidate for the browser to place in its current
/// draft. It is intentionally neither saved nor made live by this endpoint.
pub async fn generate_mcp_token_handler() -> Response {
    match crate::auth::generate_bearer_token() {
        Ok(value) => no_store_json(serde_json::json!({ "value": value })),
        Err(error) => crate::app_state::internal_error(error).into_response(),
    }
}

/// A settings viewer may see the MCP token only when its web credential is the
/// same credential, which means revealing it gives the viewer no new access.
pub async fn reveal_mcp_token_handler(
    State(state): State<AppState>,
    Extension(web_token): Extension<Option<Arc<str>>>,
) -> Response {
    let Some(web_token) = web_token else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let config = match AppState::runtime_mcp_config(&state.runtime_snapshot()) {
        Ok(config) => config,
        Err(error) => return crate::app_state::internal_error(error).into_response(),
    };
    let may_reveal = config.bearer_token.as_deref().is_some_and(|mcp_token| {
        crate::auth::constant_time_eq(mcp_token.as_bytes(), web_token.as_bytes())
    });
    if !may_reveal {
        return StatusCode::NOT_FOUND.into_response();
    }
    no_store_json(serde_json::json!({ "value": web_token.to_string() }))
}

fn no_store_json(value: serde_json::Value) -> Response {
    let mut response = Json(value).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response
}

pub async fn patch_settings_handler(
    State(state): State<AppState>,
    request: Result<Json<PatchSettingsRequest>, JsonRejection>,
) -> Response {
    let request = match request {
        Ok(Json(request)) => request,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: error.body_text(),
                }),
            )
                .into_response();
        }
    };
    if let Some(unknown) = request
        .confirm
        .iter()
        .find(|value| !KNOWN_CONSEQUENCES.contains(&value.as_str()))
    {
        return validation_response(vec![FieldError::general(format!(
            "'{unknown}' is not a consequence this save can confirm."
        ))]);
    }

    // Every save takes one path. The instance-wide versioning-task lifecycle
    // this handler used to run for a `HATCHDOOR_GIT_*` change is gone with the
    // legacy single-Vault lane (issue #185); nothing reads those keys per
    // operation now, except `HATCHDOOR_GIT_AUTHOR_NAME`/`_EMAIL`, which the
    // collection lane's Git turns read per turn and therefore need no restart
    // and no lifecycle work here.
    let result = state
        .runtime_config
        .validate_and_save(request.updates.clone(), |snapshot| {
            decide_patch(snapshot, &request)
        });
    match result {
        Ok((plan, saved)) => finish_patch(&state, plan, &saved).await,
        Err(refusal) => refusal.into_response(),
    }
}

/// The single authoritative decision for a save: field validation and the
/// reindex confirmation. Runs inside `RuntimeConfig::validate_and_save`'s
/// critical section, against the snapshot current at the moment of
/// persistence, so two concurrent PATCHes cannot both validate against a
/// snapshot the other has already invalidated (issue #54).
fn decide_patch(
    snapshot: &ConfigSnapshot,
    request: &PatchSettingsRequest,
) -> Result<PatchPlan, PatchRefusal> {
    let errors = validate_updates(snapshot, &request.updates);
    if !errors.is_empty() {
        return Err(PatchRefusal::Validation(errors));
    }

    let reindex_changed = reindex_setting_changed(snapshot, &request.updates);
    if reindex_changed && !request.confirmed("reindex") {
        return Err(PatchRefusal::Confirmation { kind: "reindex" });
    }
    let mcp_write_toggled = mcp_write_setting_toggled(snapshot, &request.updates);

    Ok(PatchPlan {
        reindex_changed,
        mcp_write_toggled,
    })
}

/// The tail every successful save shares: request the work the save implies
/// and build the response.
async fn finish_patch(state: &AppState, plan: PatchPlan, saved: &ConfigSnapshot) -> Response {
    if plan.reindex_changed {
        // One Index turn per active Vault through the shared work coordinator,
        // rather than the legacy instance-wide rebuild. Each Vault reports
        // `indexing` for its own turn and keeps answering reads from its
        // previous snapshot until the new one is published.
        request_collection_reindex(state);
    }
    if plan.mcp_write_toggled {
        // The write tools just entered or left `tools/list`; tell subscribed
        // MCP sessions to re-list. Failure only means nobody is subscribed.
        let _ = state.mcp_tools_changed.send(());
    }
    // The usage report's install ID follows its setting (ADR-45): created
    // when the save turns the report on, cleared when it turns it off.
    state.usage_report.reconcile();
    Json(settings_response(
        saved,
        state.demo_mode,
        state.agent_connections.latest(),
        update_check_status(state, saved),
        crate::usage_report::status(state, saved).await,
    ))
    .into_response()
}

fn validation_response(errors: Vec<FieldError>) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(SettingsValidationError {
            error: "Nothing was saved. Check the highlighted settings.".to_string(),
            fields: errors,
        }),
    )
        .into_response()
}

/// The server sends only the machine-readable consequence; issue #55 moved
/// the words describing it (what will happen, and its permanence) onto the
/// page, keyed off this identifier.
fn confirmation_required(kind: &'static str) -> Response {
    (
        StatusCode::CONFLICT,
        Json(serde_json::json!({ "confirmation_required": kind })),
    )
        .into_response()
}

fn reindex_setting_changed(snapshot: &ConfigSnapshot, updates: &BTreeMap<String, String>) -> bool {
    updates
        .iter()
        .any(|(key, value)| is_reindex_setting_changed(snapshot, key, value))
}

/// Whether this save actually flips `HATCHDOOR_MCP_WRITE_ENABLED`. Comparing
/// parsed truthiness rather than the raw string keeps a cosmetic rewrite (for
/// example `true` saved over `TRUE`) from claiming the tool catalogue changed.
///
/// A key the snapshot does not carry counts as currently off rather than as
/// "no change": `live_settings_defaults` always supplies it today, but failing
/// *open* here costs one redundant re-list, while failing closed would leave a
/// client's advertised tool list silently wrong.
fn mcp_write_setting_toggled(
    snapshot: &ConfigSnapshot,
    updates: &BTreeMap<String, String>,
) -> bool {
    let Some(value) = updates.get("HATCHDOOR_MCP_WRITE_ENABLED") else {
        return false;
    };
    let current = snapshot
        .setting("HATCHDOOR_MCP_WRITE_ENABLED")
        .is_some_and(|setting| crate::runtime_config::is_truthy(&setting.value));
    current != crate::runtime_config::is_truthy(value)
}

fn is_reindex_setting_changed(snapshot: &ConfigSnapshot, key: &str, value: &str) -> bool {
    SETTINGS.iter().any(|(known, class, _)| {
        key == *known
            && *class == "reindex"
            && snapshot
                .setting(key)
                .is_some_and(|setting| setting.value != value)
    })
}

fn settings_response(
    snapshot: &ConfigSnapshot,
    demo_mode: bool,
    last_agent: Option<crate::instance_state::AgentConnection>,
    update_check: crate::update_check::UpdateCheckStatus,
    usage_report: crate::usage_report::UsageReportStatus,
) -> SettingsResponse {
    let mut settings: Vec<SettingResponse> = SETTINGS
        .iter()
        .filter_map(|&(key, class, kind)| {
            let setting = snapshot.setting(key)?;
            let locked = if key == "HATCHDOOR_GIT_BRANCH" {
                Some("never")
            } else if setting.pinned {
                Some("environment")
            } else {
                None
            };
            let secret = kind == "secret";
            Some(SettingResponse {
                key,
                value: (!secret).then(|| setting.value.clone()),
                configured: secret.then(|| !setting.value.trim().is_empty()),
                source: match setting.source {
                    SettingSource::Environment => "environment",
                    SettingSource::Stored => "stored",
                    SettingSource::Default => "default",
                },
                locked,
                class,
                kind,
            })
        })
        .collect();

    // Not a live-applicable setting at all (it is boot-only: AppConfig reads
    // it once from the environment and it shapes the whole server's write
    // posture), so it never goes through RuntimeConfig/validate_updates. It
    // is surfaced here purely so the page can report it as locked, for a
    // reason distinct from both "environment" (a live setting pinned by
    // .env) and "never" (HATCHDOOR_GIT_BRANCH's fixed-by-checkout case).
    settings.push(SettingResponse {
        key: "HATCHDOOR_DEMO_MODE",
        value: Some(demo_mode.to_string()),
        configured: None,
        source: "environment",
        locked: Some("demo"),
        class: "instant",
        kind: "switch",
    });

    SettingsResponse {
        settings,
        last_agent,
        update_check,
        usage_report,
    }
}

fn validate_updates(
    snapshot: &ConfigSnapshot,
    updates: &BTreeMap<String, String>,
) -> Vec<FieldError> {
    let mut errors = Vec::new();
    for (key, value) in updates {
        let Some((_, _, kind)) = SETTINGS.iter().find(|(known, ..)| known == key) else {
            errors.push(FieldError::on(key, "This setting is not available."));
            continue;
        };
        if key == "HATCHDOOR_GIT_BRANCH" {
            errors.push(FieldError::on(
                key,
                "This value is managed by the vault's checked-out branch.",
            ));
            continue;
        }
        if snapshot.setting(key).is_some_and(|setting| setting.pinned) {
            errors.push(FieldError::on(
                key,
                "This value is managed by your .env file.",
            ));
            continue;
        }
        if *kind == "number" {
            match value.trim().parse::<u64>() {
                Ok(number) if number > 0 => {
                    if matches!(
                        key.as_str(),
                        "HATCHDOOR_MAX_ATTACHMENT_BYTES" | "HATCHDOOR_MCP_MAX_BASE64_BYTES"
                    ) && number > MAX_IN_MEMORY_UPLOAD_BYTES
                    {
                        errors.push(FieldError::on(key, "Choose 512 MB or less: uploads are held in memory while Hatchdoor writes them."));
                    }
                }
                _ => errors.push(FieldError::on(
                    key,
                    "Enter a whole number greater than zero.",
                )),
            }
        }
        if key == "HATCHDOOR_PUBLIC_URL" && crate::mcp::config::parse_public_url(value).is_err() {
            errors.push(FieldError::on(
                key,
                "Enter a full address starting with http:// or https://, such as https://notes.example.com, or leave it empty.",
            ));
        }
        if *kind == "switch" && !matches!(value.trim(), "true" | "false") {
            errors.push(FieldError::on(key, "Choose on or off."));
        }
        if key == "HATCHDOOR_GIT_SYNC_ENABLED"
            && !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "off" | "local" | "remote" | "false" | "true" | "0" | "1" | "no" | "yes" | "on"
            )
        {
            errors.push(FieldError::on(key, "Choose off, local, or remote."));
        }
    }
    if updates.keys().any(|key| {
        matches!(
            key.as_str(),
            "HATCHDOOR_MCP_ENABLED"
                | "HATCHDOOR_MCP_WRITE_ENABLED"
                | "HATCHDOOR_MCP_BEARER_TOKEN"
                | "HATCHDOOR_MCP_ALLOWED_ORIGINS"
        )
    }) {
        let prospective = snapshot.with_updates(updates);
        if let Err(message) =
            AppState::runtime_mcp_config(&prospective).and_then(|config| config.validate())
        {
            // Belongs to no single field: the refusal is that MCP is enabled
            // with no token, which spans HATCHDOOR_MCP_ENABLED and
            // HATCHDOOR_MCP_BEARER_TOKEN together, not either one alone
            // (issue #55).
            errors.push(FieldError::general(message));
        }
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime_config::{Environment, RuntimeConfig, live_settings_defaults};
    use crate::vault_registry::{NewVaultDefinition, VaultRegistryState, VaultSource};
    use crate::vault_work::{VaultWorkKind, VaultWorkWorker};
    use tempfile::tempdir;

    /// A minimal `AppState` for the settings save path's collection-lane
    /// effects. Mirrors `handlers/vaults.rs`'s own `test_state`: no Vault is
    /// registered, and `startup_sqlite` gets a cheap in-memory cache because
    /// the field is not optional. The worker is returned because nothing else
    /// in this process consumes the coordinator's queue, and the `TempDir` so
    /// it outlives the state.
    fn test_state() -> (AppState, VaultWorkWorker, tempfile::TempDir) {
        let directory = tempdir().expect("temp dir");
        let (mcp_tools_changed, _) = tokio::sync::broadcast::channel(16);
        let (vault_work, worker) = crate::vault_work::VaultWorkCoordinator::new();
        let managed_git = std::sync::Arc::new(
            crate::git::ManagedGitScheduler::without_durable_state(vault_work.clone()),
        );
        let state = AppState {
            vault_registry: crate::vault_registry::VaultRegistryStore::new(
                directory.path().join("state/vaults.json"),
            ),
            vaults: crate::vault_runtime::VaultCollectionRuntime::new(),
            vault_work,
            managed_git,
            commit_cooldown: Arc::new(crate::git::CommitCooldown::new()),
            startup_sqlite: std::sync::Arc::new(
                crate::cache::SqliteCache::in_memory(384).expect("in-memory cache"),
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
            runtime_config: RuntimeConfig::for_tests(),
            startup: crate::startup::StartupTracker::ready(),
            transfer_links: Default::default(),
            vault_mount_root: Default::default(),
            instance_versions: Default::default(),
            agent_connections: Default::default(),
            usage_report: Default::default(),
            shutdown: Default::default(),
        };
        (state, worker, directory)
    }

    /// Commit one `Local` Vault definition and activate it in the collection,
    /// so the settings save path under test has a real active runtime to
    /// request an Index turn for.
    async fn add_local_vault(
        state: &AppState,
        name: &str,
        enabled: bool,
    ) -> crate::vault_registry::VaultId {
        let path = state
            .vault_registry
            .path()
            .parent()
            .and_then(std::path::Path::parent)
            .expect("state root")
            .join(name);
        std::fs::create_dir_all(&path).expect("vault dir");
        let before: std::collections::BTreeSet<_> = match state.vault_registry.load() {
            Ok(VaultRegistryState::Ready(snapshot)) => snapshot.vault_ids().collect(),
            _ => Default::default(),
        };
        let revision = match state.vault_registry.load() {
            Ok(VaultRegistryState::Ready(snapshot)) => snapshot.revision(),
            _ => 0,
        };
        let snapshot = state
            .vault_registry
            .add(
                revision,
                NewVaultDefinition {
                    name: name.to_string(),
                    enabled,
                    source: VaultSource::Local { path },
                    exclude_patterns: Vec::new(),
                    https_credentials: None,
                    archive_folder: None,
                    commit_identity: None,
                },
            )
            .expect("add Vault");
        state
            .vaults
            .reconcile_and_reconstruct(
                &state.vault_registry,
                &snapshot,
                &state.vault_work,
                &state.managed_git,
            )
            .await;
        snapshot
            .vault_ids()
            .find(|id| !before.contains(id))
            .expect("the newly added Vault")
    }

    /// Drain everything the coordinator currently holds, recording which turns
    /// were requested. Each turn is answered with success without doing any
    /// real work, which is all these tests need.
    async fn drain(
        worker: &mut VaultWorkWorker,
    ) -> Vec<(crate::vault_registry::VaultId, VaultWorkKind)> {
        let mut seen = Vec::new();
        while let Ok(Some(outcome)) = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            worker.run_next(|_| async move { Ok::<(), crate::vault_work::VaultWorkError>(()) }),
        )
        .await
        {
            seen.push((outcome.request.vault_id(), outcome.request.kind()));
        }
        seen
    }

    #[tokio::test]
    async fn a_confirmed_layer_save_queues_one_index_turn_for_every_active_vault() {
        let (state, mut worker, _directory) = test_state();
        let first = add_local_vault(&state, "first", true).await;
        let second = add_local_vault(&state, "second", true).await;
        let disabled = add_local_vault(&state, "disabled", false).await;
        // Clear the turns Vault activation itself requested, so what remains
        // is attributable to the settings save alone.
        drain(&mut worker).await;

        let request = PatchSettingsRequest {
            updates: BTreeMap::from([("HATCHDOOR_EMBED_LAYERS".into(), "false".into())]),
            confirm: vec!["reindex".into()],
        };
        let response = patch_settings_handler(State(state.clone()), Ok(Json(request))).await;
        assert_eq!(response.status(), StatusCode::OK);

        let turns = drain(&mut worker).await;
        let indexed: std::collections::BTreeSet<_> = turns
            .iter()
            .filter(|(_, kind)| *kind == VaultWorkKind::Index)
            .map(|(vault_id, _)| *vault_id)
            .collect();
        assert_eq!(
            indexed,
            std::collections::BTreeSet::from([first, second]),
            "one Index turn per active Vault, and none for the disabled one"
        );
        assert!(
            !indexed.contains(&disabled),
            "a disabled Vault has no active runtime and must not be queued"
        );
    }

    #[tokio::test]
    async fn a_save_with_no_indexing_change_queues_nothing() {
        let (state, mut worker, _directory) = test_state();
        add_local_vault(&state, "first", true).await;
        drain(&mut worker).await;

        let request = PatchSettingsRequest {
            updates: BTreeMap::from([("HATCHDOOR_ARCHIVE_PREFIX".into(), "archive/".into())]),
            confirm: Vec::new(),
        };
        let response = patch_settings_handler(State(state.clone()), Ok(Json(request))).await;
        assert_eq!(response.status(), StatusCode::OK);

        assert!(
            drain(&mut worker).await.is_empty(),
            "an instant setting must not queue background work"
        );
    }

    /// The Git author defaults are the commit identity every Vault without its
    /// own falls back to, so a save of them has to succeed like any other
    /// instant setting: no lifecycle work, no confirmation, and no queued
    /// background turn. Every `HATCHDOOR_GIT_*` key used to route into the
    /// legacy single-Vault versioning-task lifecycle instead, which #185
    /// deleted.
    #[tokio::test]
    async fn git_author_defaults_save_like_any_other_instant_setting() {
        let (state, _worker, _directory) = test_state();

        let request = PatchSettingsRequest {
            updates: BTreeMap::from([
                ("HATCHDOOR_GIT_AUTHOR_NAME".into(), "Second Author".into()),
                (
                    "HATCHDOOR_GIT_AUTHOR_EMAIL".into(),
                    "second@example.test".into(),
                ),
            ]),
            confirm: Vec::new(),
        };
        let response = patch_settings_handler(State(state.clone()), Ok(Json(request))).await;
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "the commit identity must be savable on any deployment"
        );

        let snapshot = state.runtime_snapshot();
        assert_eq!(
            snapshot
                .setting("HATCHDOOR_GIT_AUTHOR_NAME")
                .expect("setting")
                .value,
            "Second Author"
        );
        assert_eq!(
            snapshot
                .setting("HATCHDOOR_GIT_AUTHOR_EMAIL")
                .expect("setting")
                .value,
            "second@example.test"
        );
    }

    #[tokio::test]
    async fn toggling_mcp_writes_broadcasts_a_tool_list_change_and_other_saves_do_not() {
        let (state, _worker, _directory) = test_state();
        let mut tools_changed = state.mcp_tools_changed.subscribe();

        let request = PatchSettingsRequest {
            updates: BTreeMap::from([("HATCHDOOR_ARCHIVE_PREFIX".into(), "archive/".into())]),
            confirm: Vec::new(),
        };
        assert_eq!(
            patch_settings_handler(State(state.clone()), Ok(Json(request)))
                .await
                .status(),
            StatusCode::OK
        );
        assert!(
            tools_changed.try_recv().is_err(),
            "a setting that cannot change the tool catalogue must not signal one"
        );

        let request = PatchSettingsRequest {
            updates: BTreeMap::from([("HATCHDOOR_MCP_WRITE_ENABLED".into(), "true".into())]),
            confirm: Vec::new(),
        };
        assert_eq!(
            patch_settings_handler(State(state.clone()), Ok(Json(request)))
                .await
                .status(),
            StatusCode::OK
        );
        assert!(
            tools_changed.try_recv().is_ok(),
            "enabling MCP writes adds the write tools, so clients must re-list"
        );

        // Saving the same value again changes nothing about the catalogue.
        let request = PatchSettingsRequest {
            updates: BTreeMap::from([("HATCHDOOR_MCP_WRITE_ENABLED".into(), "true".into())]),
            confirm: Vec::new(),
        };
        assert_eq!(
            patch_settings_handler(State(state.clone()), Ok(Json(request)))
                .await
                .status(),
            StatusCode::OK
        );
        assert!(
            tools_changed.try_recv().is_err(),
            "re-saving the same value must not claim the tool catalogue changed"
        );
    }

    #[test]
    fn environment_values_are_reported_as_locked_and_secrets_are_masked() {
        let directory = tempdir().expect("test settings directory");
        let snapshot = RuntimeConfig::load(
            directory.path().join("settings.json"),
            Environment::from_values([
                ("HATCHDOOR_ARCHIVE_PREFIX".into(), "env-archive/".into()),
                ("HATCHDOOR_MCP_BEARER_TOKEN".into(), "secret".into()),
            ]),
            live_settings_defaults(),
        )
        .expect("runtime config")
        .snapshot();
        let response = settings_response(
            &snapshot,
            false,
            None,
            Default::default(),
            Default::default(),
        );
        let archive = response
            .settings
            .iter()
            .find(|setting| setting.key == "HATCHDOOR_ARCHIVE_PREFIX")
            .unwrap();
        assert_eq!(archive.source, "environment");
        assert_eq!(archive.locked, Some("environment"));
        let token = response
            .settings
            .iter()
            .find(|setting| setting.key == "HATCHDOOR_MCP_BEARER_TOKEN")
            .unwrap();
        assert_eq!(token.value, None);
        assert_eq!(token.configured, Some(true));
    }

    #[test]
    fn demo_mode_is_reported_as_locked_for_a_reason_distinct_from_environment_and_branch() {
        let snapshot = RuntimeConfig::for_tests().snapshot();
        let response = settings_response(
            &snapshot,
            true,
            None,
            Default::default(),
            Default::default(),
        );
        let demo = response
            .settings
            .iter()
            .find(|setting| setting.key == "HATCHDOOR_DEMO_MODE")
            .expect("demo mode setting present");
        assert_eq!(demo.locked, Some("demo"));
        assert_ne!(demo.locked, Some("environment"));
        assert_ne!(demo.locked, Some("never"));
        assert_eq!(demo.value.as_deref(), Some("true"));
    }

    #[test]
    fn the_last_agent_is_reported_or_null_when_none_has_connected() {
        let snapshot = RuntimeConfig::for_tests().snapshot();
        let none = serde_json::to_value(settings_response(
            &snapshot,
            false,
            None,
            Default::default(),
            Default::default(),
        ))
        .unwrap();
        assert_eq!(none["last_agent"], serde_json::Value::Null);

        let agent = crate::instance_state::AgentConnection {
            name: "Claude Code".into(),
            connected_at: "2026-10-03T09:00:00Z".into(),
        };
        let some = serde_json::to_value(settings_response(
            &snapshot,
            false,
            Some(agent),
            Default::default(),
            Default::default(),
        ))
        .unwrap();
        assert_eq!(
            some["last_agent"],
            serde_json::json!({"name": "Claude Code", "connected_at": "2026-10-03T09:00:00Z"})
        );
    }

    #[tokio::test]
    async fn the_update_check_is_a_switch_and_its_status_is_read_beside_the_registry() {
        let (state, _worker, _directory) = test_state();
        let store =
            crate::instance_state::InstanceStateStore::beside_registry(state.vault_registry.path());
        store
            .write_section(
                "update_check",
                &serde_json::json!({
                    "checked_at": "2026-10-03T09:00:00Z",
                    "latest": {
                        "version": "99.0.0",
                        "release_url": "https://github.com/BatterWorks/Hatchdoor/releases/tag/v99.0.0"
                    }
                }),
            )
            .unwrap();
        let read = |state: AppState| async move {
            let response = get_settings_handler(State(state)).await.into_response();
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            serde_json::from_slice::<serde_json::Value>(&body).unwrap()
        };

        let off = read(state.clone()).await;
        assert_eq!(
            off["update_check"],
            serde_json::json!({
                "enabled": false,
                "checked_at": "2026-10-03T09:00:00Z",
                "update_available": null
            }),
            "nothing is offered while the check is off"
        );
        let setting = off["settings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|setting| setting["key"] == "HATCHDOOR_UPDATE_CHECK_ENABLED")
            .expect("the setting is editable from Settings (ADR-14)");
        assert_eq!(setting["kind"], "switch");
        assert_eq!(setting["value"], "false");

        assert!(
            validate_updates(
                &state.runtime_snapshot(),
                &BTreeMap::from([("HATCHDOOR_UPDATE_CHECK_ENABLED".into(), "true".into())]),
            )
            .is_empty()
        );
        state
            .runtime_config
            .save([(
                "HATCHDOOR_UPDATE_CHECK_ENABLED".to_string(),
                "true".to_string(),
            )])
            .unwrap();
        let on = read(state).await;
        assert_eq!(on["update_check"]["enabled"], true);
        assert_eq!(on["update_check"]["update_available"]["version"], "99.0.0");
    }

    #[tokio::test]
    async fn the_usage_report_is_a_switch_and_settings_shows_the_exact_report_on_and_off() {
        let (mut state, _worker, directory) = test_state();
        add_local_vault(&state, "notes", true).await;
        state.usage_report = Arc::new(crate::usage_report::UsageReport::new(
            crate::instance_state::InstanceStateStore::beside_registry(state.vault_registry.path()),
            state.runtime_config.clone(),
            false,
        ));
        state.usage_report.reconcile();
        let body = |response: Response| async move {
            assert_eq!(response.status(), StatusCode::OK);
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            serde_json::from_slice::<serde_json::Value>(&body).unwrap()
        };
        let save = |state: AppState, value: &'static str| async move {
            let request = PatchSettingsRequest {
                updates: BTreeMap::from([(
                    "HATCHDOOR_USAGE_REPORT_ENABLED".to_string(),
                    value.to_string(),
                )]),
                confirm: Vec::new(),
            };
            patch_settings_handler(State(state), Ok(Json(request))).await
        };
        let report = |settings: &serde_json::Value| {
            serde_json::from_str::<serde_json::Value>(
                settings["usage_report"]["report"].as_str().unwrap(),
            )
            .unwrap()
        };

        let off = body(
            get_settings_handler(State(state.clone()))
                .await
                .into_response(),
        )
        .await;
        let setting = off["settings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|setting| setting["key"] == "HATCHDOOR_USAGE_REPORT_ENABLED")
            .expect("the setting is editable from Settings (ADR-14)");
        assert_eq!(setting["kind"], "switch");
        assert_eq!(setting["value"], "false");
        assert_eq!(setting["locked"], serde_json::Value::Null);
        assert_eq!(off["usage_report"]["enabled"], false);
        assert_eq!(off["usage_report"]["install_id"], serde_json::Value::Null);
        let shown = report(&off);
        assert_eq!(
            shown["payload"]["id"],
            crate::usage_report::INSTALL_ID_PLACEHOLDER
        );
        assert_eq!(shown["payload"]["data"]["schema"], 1);
        assert_eq!(shown["payload"]["data"]["vaults"], "1");
        assert_eq!(shown["payload"]["data"]["git_sync"], "none");
        assert_eq!(shown["payload"]["data"]["search_model"], "none");
        assert_eq!(shown["payload"]["data"]["mcp_enabled"], false);

        // On without a restart: the save's own response already carries the ID.
        let on = body(save(state.clone(), "true").await).await;
        assert_eq!(on["usage_report"]["enabled"], true);
        let id = on["usage_report"]["install_id"]
            .as_str()
            .expect("an install ID once the report is on")
            .to_string();
        assert_eq!(report(&on)["payload"]["id"], id.as_str());
        assert_eq!(
            on["usage_report"]["last_sent_at"],
            serde_json::Value::Null,
            "on, and nothing sent yet"
        );
        // The job's send, with the request itself replaced (#478).
        let mut reporter =
            crate::usage_report::Reporter::new(state.clone(), Arc::new(|_request| Ok(())));
        let sent_at =
            std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_800_000_000);
        assert!(reporter.tick(sent_at).await);
        let sent = body(
            get_settings_handler(State(state.clone()))
                .await
                .into_response(),
        )
        .await;
        assert_eq!(sent["usage_report"]["last_sent_at"], "2027-01-15T08:00:00Z");
        assert_eq!(
            state.usage_report.install_id().as_deref(),
            Some(id.as_str())
        );

        let off_again = body(save(state.clone(), "false").await).await;
        assert_eq!(
            off_again["usage_report"]["install_id"],
            serde_json::Value::Null
        );
        assert_eq!(
            report(&off_again)["payload"]["id"],
            crate::usage_report::INSTALL_ID_PLACEHOLDER
        );
        assert_eq!(
            off_again["usage_report"]["last_sent_at"],
            serde_json::Value::Null
        );
        let state_file = std::fs::read_to_string(directory.path().join("state/instance.json"))
            .unwrap_or_default();
        assert!(!state_file.contains("usage_report"), "{state_file}");
        assert!(!state_file.contains(&id), "{state_file}");
    }

    #[tokio::test]
    async fn an_install_id_that_could_not_be_saved_is_not_shown() {
        let (mut state, _worker, directory) = test_state();
        // A file where the report's state directory should be: the ID cannot
        // be saved, so it must not reach the response or the report.
        std::fs::write(directory.path().join("blocked"), b"in the way").unwrap();
        state
            .runtime_config
            .save([(
                "HATCHDOOR_USAGE_REPORT_ENABLED".to_string(),
                "true".to_string(),
            )])
            .unwrap();
        state.usage_report = Arc::new(crate::usage_report::UsageReport::new(
            crate::instance_state::InstanceStateStore::new(
                directory.path().join("blocked/instance.json"),
            ),
            state.runtime_config.clone(),
            false,
        ));
        state.usage_report.reconcile();

        let response = get_settings_handler(State(state)).await.into_response();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let settings = serde_json::from_slice::<serde_json::Value>(&body).unwrap();

        assert_eq!(settings["usage_report"]["enabled"], true);
        assert_eq!(
            settings["usage_report"]["install_id"],
            serde_json::Value::Null
        );
        let report = serde_json::from_str::<serde_json::Value>(
            settings["usage_report"]["report"].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(
            report["payload"]["id"],
            crate::usage_report::INSTALL_ID_PLACEHOLDER
        );
    }

    #[test]
    fn a_usage_report_setting_from_the_environment_is_locked() {
        let directory = tempfile::tempdir().unwrap();
        let config = RuntimeConfig::load(
            directory.path().join("settings.json"),
            crate::runtime_config::Environment::from_values([(
                "HATCHDOOR_USAGE_REPORT_ENABLED".to_string(),
                "true".to_string(),
            )]),
            crate::runtime_config::live_settings_defaults(),
        )
        .unwrap();
        let listed = settings_response(
            &config.snapshot(),
            false,
            None,
            Default::default(),
            Default::default(),
        );
        let setting = listed
            .settings
            .iter()
            .find(|setting| setting.key == "HATCHDOOR_USAGE_REPORT_ENABLED")
            .unwrap();
        assert_eq!(setting.locked, Some("environment"));
        assert_eq!(setting.value.as_deref(), Some("true"));
        assert_eq!(
            validate_updates(
                &config.snapshot(),
                &BTreeMap::from([("HATCHDOOR_USAGE_REPORT_ENABLED".into(), "false".into())]),
            )
            .len(),
            1,
            "a pinned setting cannot be saved over"
        );
    }

    #[test]
    fn attachment_limit_rejects_an_unsafe_in_memory_value() {
        let config = RuntimeConfig::for_tests();
        let errors = validate_updates(
            &config.snapshot(),
            &BTreeMap::from([(
                "HATCHDOOR_MAX_ATTACHMENT_BYTES".into(),
                (MAX_IN_MEMORY_UPLOAD_BYTES + 1).to_string(),
            )]),
        );
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn enabled_mcp_without_a_token_is_a_form_level_refusal_with_no_single_field() {
        let config = RuntimeConfig::for_tests();
        let errors = validate_updates(
            &config.snapshot(),
            &BTreeMap::from([("HATCHDOOR_MCP_ENABLED".into(), "true".into())]),
        );

        assert_eq!(errors.len(), 1);
        assert_eq!(
            errors[0].key, None,
            "the refusal spans two fields (enabled + token), so it belongs to neither alone"
        );
    }

    #[test]
    fn public_url_accepts_an_absolute_address_or_empty_and_refuses_anything_else() {
        let config = RuntimeConfig::for_tests();
        let check = |value: &str| {
            validate_updates(
                &config.snapshot(),
                &BTreeMap::from([("HATCHDOOR_PUBLIC_URL".into(), value.into())]),
            )
        };
        assert!(check("https://notes.example.com").is_empty());
        assert!(check("").is_empty());
        let errors = check("notes.example.com");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].key.as_deref(), Some("HATCHDOOR_PUBLIC_URL"));

        let listed = settings_response(
            &config.snapshot(),
            false,
            None,
            Default::default(),
            Default::default(),
        );
        assert!(
            listed
                .settings
                .iter()
                .any(|setting| setting.key == "HATCHDOOR_PUBLIC_URL" && setting.kind == "text"),
            "the setting is editable from Settings (ADR-14)"
        );
    }
}
