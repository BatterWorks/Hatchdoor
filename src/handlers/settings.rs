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
use crate::app_state::AppState;
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchSettingsRequest {
    pub updates: BTreeMap<String, String>,
}

#[derive(Debug, Serialize)]
struct FieldError {
    key: String,
    message: String,
}

#[derive(Debug, Serialize)]
struct SettingsValidationError {
    error: String,
    fields: Vec<FieldError>,
}

const SETTINGS: &[(&str, &str, &str)] = &[
    ("HATCHDOOR_ARCHIVE_PREFIX", "instant", "text"),
    ("HATCHDOOR_EXCLUDE", "reindex", "text"),
    ("HATCHDOOR_EMBED_LAYERS", "reindex", "switch"),
    ("HATCHDOOR_MCP_ENABLED", "instant", "switch"),
    ("HATCHDOOR_MCP_WRITE_ENABLED", "instant", "switch"),
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
    Json(settings_response(&state.runtime_snapshot()))
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
    let snapshot = state.runtime_snapshot();
    let errors = validate_updates(&snapshot, &request.updates);
    if !errors.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(SettingsValidationError {
                error: "Nothing was saved. Check the highlighted settings.".to_string(),
                fields: errors,
            }),
        )
            .into_response();
    }

    match state.runtime_config.save(request.updates) {
        Ok(snapshot) => Json(settings_response(&snapshot)).into_response(),
        Err(error) => crate::app_state::internal_error(error).into_response(),
    }
}

fn settings_response(snapshot: &ConfigSnapshot) -> SettingsResponse {
    let settings = SETTINGS
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
    SettingsResponse { settings }
}

fn validate_updates(
    snapshot: &ConfigSnapshot,
    updates: &BTreeMap<String, String>,
) -> Vec<FieldError> {
    let mut errors = Vec::new();
    for (key, value) in updates {
        let Some((_, _, kind)) = SETTINGS.iter().find(|(known, ..)| known == key) else {
            errors.push(FieldError {
                key: key.clone(),
                message: "This setting is not available.".to_string(),
            });
            continue;
        };
        if key == "HATCHDOOR_GIT_BRANCH" {
            errors.push(FieldError {
                key: key.clone(),
                message: "This value is managed by the vault's checked-out branch.".to_string(),
            });
            continue;
        }
        if snapshot.setting(key).is_some_and(|setting| setting.pinned) {
            errors.push(FieldError {
                key: key.clone(),
                message: "This value is managed by your .env file.".to_string(),
            });
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
                        errors.push(FieldError { key: key.clone(), message: "Choose 512 MB or less: uploads are held in memory while Hatchdoor writes them.".to_string() });
                    }
                }
                _ => errors.push(FieldError {
                    key: key.clone(),
                    message: "Enter a whole number greater than zero.".to_string(),
                }),
            }
        }
        if *kind == "switch" && !matches!(value.trim(), "true" | "false") {
            errors.push(FieldError {
                key: key.clone(),
                message: "Choose on or off.".to_string(),
            });
        }
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime_config::{Environment, RuntimeConfig, live_settings_defaults};

    #[test]
    fn environment_values_are_reported_as_locked_and_secrets_are_masked() {
        let snapshot = RuntimeConfig::load(
            std::env::temp_dir().join("hatchdoor-settings-handler-test.json"),
            Environment::from_values([
                ("HATCHDOOR_ARCHIVE_PREFIX".into(), "env-archive/".into()),
                ("HATCHDOOR_MCP_BEARER_TOKEN".into(), "secret".into()),
            ]),
            live_settings_defaults(),
        )
        .expect("runtime config")
        .snapshot();
        let response = settings_response(&snapshot);
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
    fn attachment_limit_rejects_an_unsafe_in_memory_value() {
        let config = RuntimeConfig::load(
            std::env::temp_dir().join("hatchdoor-settings-handler-test-unsafe.json"),
            Environment::empty(),
            live_settings_defaults(),
        )
        .expect("runtime config");
        let errors = validate_updates(
            &config.snapshot(),
            &BTreeMap::from([(
                "HATCHDOOR_MAX_ATTACHMENT_BYTES".into(),
                (MAX_IN_MEMORY_UPLOAD_BYTES + 1).to_string(),
            )]),
        );
        assert_eq!(errors.len(), 1);
    }
}
