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
use crate::app_state::{AppState, schedule_settings_reindex};
use crate::runtime_config::{ConfigSnapshot, SettingSource};

/// A deliberately generous ceiling: multipart files are buffered while they
/// are written, so accepting arbitrarily large limits would let one upload
/// exhaust the process.
pub const MAX_IN_MEMORY_UPLOAD_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Serialize)]
pub struct SettingsResponse {
    pub settings: Vec<SettingResponse>,
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

/// The consequences a save may need explicit consent for. Accumulated by the
/// page across successive `409`s rather than carried one at a time, so a save
/// needing two consents (e.g. downgrading remote versioning onto a vault that
/// is no longer a git repository) does not ping-pong between them forever
/// (issue #57).
const KNOWN_CONSEQUENCES: &[&str] = &["reindex", "git_init", "git_downgrade"];

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

/// What to do once a validated save has been persisted: schedule a reindex,
/// and/or install a newly spawned versioning task.
struct PatchPlan {
    reindex_changed: bool,
    git_config: Option<crate::git::GitConfig>,
    initialize_local_repo: Option<crate::git::GitConfig>,
}

/// The paths a git-touching save needs to preflight a mode change and (for a
/// fresh local repo) derive `.gitignore` entries from.
struct GitPaths {
    vault_path: std::path::PathBuf,
    cache_db_path: std::path::PathBuf,
    settings_file_path: std::path::PathBuf,
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
    ("HATCHDOOR_MAX_ATTACHMENT_BYTES", "instant", "number"),
    ("HATCHDOOR_MCP_MAX_BASE64_BYTES", "instant", "number"),
    ("HATCHDOOR_GIT_SYNC_ENABLED", "instant", "mode"),
    ("HATCHDOOR_GIT_HTTPS_USERNAME", "instant", "text"),
    ("HATCHDOOR_GIT_HTTPS_TOKEN", "instant", "secret"),
    ("HATCHDOOR_GIT_DEBOUNCE_SECONDS", "instant", "number"),
    ("HATCHDOOR_GIT_AUTHOR_NAME", "instant", "text"),
    ("HATCHDOOR_GIT_AUTHOR_EMAIL", "instant", "text"),
    ("HATCHDOOR_GIT_BRANCH", "instant", "text"),
];

pub async fn get_settings_handler(State(state): State<AppState>) -> impl IntoResponse {
    Json(settings_response(
        &state.runtime_snapshot(),
        state.demo_mode,
    ))
}

/// Settings-only background reindex state. This deliberately stays separate
/// from startup readiness: search continues to use its prior coherent SQLite
/// snapshot while this status reports configuration drift.
pub async fn get_index_status_handler(State(state): State<AppState>) -> Response {
    let mut response = Json(state.index_status.status()).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response
}

/// Versioning has its own live surface because the settings document is the
/// durable desired configuration, while this reports the task that is applying
/// it right now.
pub async fn get_git_status_handler(State(state): State<AppState>) -> Response {
    let vault_path = state.vault_path().await;
    let status = match state.git_sync.try_read() {
        Ok(sync) => match sync.as_ref() {
            Some(handle) => handle.status().read().await.clone(),
            None => crate::git::GitSyncStatus::disabled(),
        },
        Err(_) => {
            let mode = vault_path
                .and_then(|vault_path| {
                    crate::git::GitConfig::from_snapshot(vault_path, &state.runtime_snapshot())
                        .ok()
                        .flatten()
                })
                .map(|config| config.mode.as_str().to_string())
                .unwrap_or_else(|| "off".to_string());
            crate::git::GitSyncStatus {
                enabled: mode != "off",
                state: "stopping".to_string(),
                mode,
                ..Default::default()
            }
        }
    };
    no_store_json(serde_json::to_value(status).expect("git status serializes"))
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

    let git_changed = request
        .updates
        .keys()
        .any(|key| key.starts_with("HATCHDOOR_GIT_"));

    if git_changed {
        patch_settings_with_git_lifecycle(&state, request).await
    } else {
        let result = state
            .runtime_config
            .validate_and_save(request.updates.clone(), |snapshot| {
                decide_patch(snapshot, &request, None)
            });
        match result {
            Ok((plan, saved)) => finish_patch(&state, plan, &saved),
            Err(refusal) => refusal.into_response(),
        }
    }
}

/// The git-touching path: the versioning task lifecycle (stop the old task,
/// persist, spawn the new one) is its own function so `patch_settings_handler`
/// reads as one short dispatch rather than one long function mixing
/// validation, confirmations, task lifecycle, and persistence together.
async fn patch_settings_with_git_lifecycle(
    state: &AppState,
    request: PatchSettingsRequest,
) -> Response {
    let Some(vault_path) = state.vault_path().await else {
        return crate::app_state::vault_unavailable().into_response();
    };
    let old_git_config =
        match crate::git::GitConfig::from_snapshot(vault_path.clone(), &state.runtime_snapshot()) {
            Ok(config) => config,
            Err(error) => {
                return validation_response(vec![FieldError::on(
                    "HATCHDOOR_GIT_SYNC_ENABLED",
                    error,
                )]);
            }
        };

    let mut active = state.git_sync.write().await;
    if let Some(handle) = active.as_ref()
        && let Err(message) = handle.stop(std::time::Duration::from_secs(15)).await
    {
        let status = handle.status();
        let mut status = status.write().await;
        status.last_ok = false;
        status.last_error = Some(message.clone());
        return busy_response(message);
    }
    active.take();

    let git_paths = GitPaths {
        vault_path,
        cache_db_path: state.cache_db_path.clone(),
        settings_file_path: state.runtime_config.settings_path().to_path_buf(),
    };
    let init_cache_db_path = git_paths.cache_db_path.clone();
    let init_settings_file_path = git_paths.settings_file_path.clone();
    let result = state.runtime_config.validate_and_save_then(
        request.updates.clone(),
        |snapshot| decide_patch(snapshot, &request, Some(git_paths)),
        |plan, _saved| {
            let Some(config) = &plan.initialize_local_repo else {
                return Ok(());
            };
            crate::git::init_local_repo(config, &init_cache_db_path, &init_settings_file_path)
                .map_err(|error| {
                    PatchRefusal::Internal(format!(
                        "Local versioning setup failed after the setting was saved; Hatchdoor restored the previous setting: {error}"
                    ))
                })
        },
    );

    match result {
        Ok((plan, saved)) => {
            if let Some(config) = plan.git_config.clone() {
                *active = Some(crate::git::spawn_sync_task(
                    config,
                    state.vault_write_lock.clone(),
                    production_sync_ops(),
                ));
            }
            drop(active);
            finish_patch(state, plan, &saved)
        }
        Err(refusal) => {
            // Nothing was persisted: restore the task we stopped so the
            // refusal leaves versioning exactly as it was.
            if let Some(config) = old_git_config {
                *active = Some(crate::git::spawn_sync_task(
                    config,
                    state.vault_write_lock.clone(),
                    production_sync_ops(),
                ));
            }
            drop(active);
            refusal.into_response()
        }
    }
}

/// The single authoritative decision for a save: field validation, the
/// reindex confirmation, and (when `vault_path` is `Some`, i.e. this save
/// touches `HATCHDOOR_GIT_*`) the downgrade/init confirmations and git
/// preflight. Runs inside `RuntimeConfig::validate_and_save`'s critical
/// section, against the snapshot current at the moment of persistence, so two
/// concurrent PATCHes cannot both validate against a snapshot the other has
/// already invalidated (issue #54).
fn decide_patch(
    snapshot: &ConfigSnapshot,
    request: &PatchSettingsRequest,
    git_paths: Option<GitPaths>,
) -> Result<PatchPlan, PatchRefusal> {
    let errors = validate_updates(snapshot, &request.updates);
    if !errors.is_empty() {
        return Err(PatchRefusal::Validation(errors));
    }

    let reindex_changed = reindex_setting_changed(snapshot, &request.updates);
    if reindex_changed && !request.confirmed("reindex") {
        return Err(PatchRefusal::Confirmation { kind: "reindex" });
    }

    let Some(git_paths) = git_paths else {
        return Ok(PatchPlan {
            reindex_changed,
            git_config: None,
            initialize_local_repo: None,
        });
    };
    let vault_path = git_paths.vault_path;

    let prospective = snapshot.with_updates(&request.updates);
    let git_config = crate::git::GitConfig::from_snapshot(vault_path.clone(), &prospective)
        .map_err(|message| {
            PatchRefusal::Validation(vec![FieldError::on("HATCHDOOR_GIT_SYNC_ENABLED", message)])
        })?;
    let old_git_config = crate::git::GitConfig::from_snapshot(vault_path, snapshot)
        .expect("current git configuration was validated when saved");

    if old_git_config
        .as_ref()
        .is_some_and(|config| config.mode == crate::git::GitMode::Remote)
        && git_config
            .as_ref()
            .is_none_or(|config| config.mode != crate::git::GitMode::Remote)
        && !request.confirmed("git_downgrade")
    {
        return Err(PatchRefusal::Confirmation {
            kind: "git_downgrade",
        });
    }

    let mut initialize_local_repo = None;
    if let Some(config) = &git_config {
        let preflight = match config.mode {
            crate::git::GitMode::Local => crate::git::validate_local_repo(config),
            crate::git::GitMode::Remote => crate::git::validate_repo(config),
        };
        if let Err(error) = preflight {
            if config.mode == crate::git::GitMode::Local {
                if !request.confirmed("git_init") {
                    return Err(PatchRefusal::Confirmation { kind: "git_init" });
                }
                initialize_local_repo = Some(config.clone());
            } else {
                return Err(PatchRefusal::Validation(vec![FieldError::on(
                    "HATCHDOOR_GIT_SYNC_ENABLED",
                    error.to_string(),
                )]));
            }
        }
    }

    Ok(PatchPlan {
        reindex_changed,
        git_config,
        initialize_local_repo,
    })
}

/// The tail every successful save shares: schedule a reindex when needed and
/// build the response. Previously duplicated at the end of both the git and
/// non-git branches.
fn finish_patch(state: &AppState, plan: PatchPlan, saved: &ConfigSnapshot) -> Response {
    if plan.reindex_changed {
        schedule_settings_reindex(state.clone());
    }
    Json(settings_response(saved, state.demo_mode)).into_response()
}

fn production_sync_ops() -> crate::git::SyncOps {
    crate::git::SyncOps {
        commit: Box::new(crate::git::commit_local),
        fetch: Box::new(crate::git::fetch_remote),
        integrate: Box::new(crate::git::integrate_fetched),
        push: Box::new(crate::git::push_branch),
    }
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

fn busy_response(message: String) -> Response {
    (
        StatusCode::CONFLICT,
        Json(serde_json::json!({ "error": message, "state": "stopping" })),
    )
        .into_response()
}

fn reindex_setting_changed(snapshot: &ConfigSnapshot, updates: &BTreeMap<String, String>) -> bool {
    updates
        .iter()
        .any(|(key, value)| is_reindex_setting_changed(snapshot, key, value))
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

fn settings_response(snapshot: &ConfigSnapshot, demo_mode: bool) -> SettingsResponse {
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

    SettingsResponse { settings }
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
    use tempfile::tempdir;

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
        let response = settings_response(&snapshot, false);
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
        let response = settings_response(&snapshot, true);
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
    fn failed_settings_persistence_does_not_initialize_local_versioning() {
        let temp = tempfile::tempdir().expect("tempdir");
        let vault = temp.path().join("vault");
        std::fs::create_dir(&vault).expect("vault");
        let settings_parent = temp.path().join("settings-parent-is-a-file");
        std::fs::write(&settings_parent, "not a directory").expect("block settings parent");
        let runtime = RuntimeConfig::load(
            settings_parent.join("settings.json"),
            Environment::empty(),
            live_settings_defaults(),
        )
        .expect("runtime configuration loads before its first save");
        let request = PatchSettingsRequest {
            updates: BTreeMap::from([("HATCHDOOR_GIT_SYNC_ENABLED".into(), "local".into())]),
            confirm: vec!["git_init".into()],
        };
        let git_paths = GitPaths {
            vault_path: vault.clone(),
            cache_db_path: vault.join(".hatchdoor-cache.sqlite3"),
            settings_file_path: settings_parent.join("settings.json"),
        };

        let result = runtime.validate_and_save(request.updates.clone(), |snapshot| {
            decide_patch(snapshot, &request, Some(git_paths))
        });

        assert!(
            result.is_err(),
            "the intentionally blocked settings write fails"
        );
        assert!(
            !vault.join(".git").exists(),
            "a failed settings persistence must not initialize a surprise repository"
        );
    }
}
